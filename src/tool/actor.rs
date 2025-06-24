//! Actor-based tool runtime for concurrent tool execution.
//!
//! Implements [`ToolRuntimeApi`] for invoking and listing tools, and [`ToolStoreApi`] for
//! updating the available tool set. The actor manages a [`Toolbox`] and executes multiple tool
//! calls concurrently.
//!
//! The [`ToolRuntimeApi`] is consumed by the [`crate::csi::RawCsi`] and the tool routes, whereas
//! the [`ToolStoreApi`] is consumed by the `McpActor`.

use futures::{StreamExt, stream::FuturesUnordered};
use std::{collections::HashMap, pin::Pin, sync::Arc};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    logging::TracingContext,
    namespace_watcher::Namespace,
    tool::{
        Argument, Modality, QualifiedToolName, Tool, ToolDescription, ToolError, ToolOutput,
        toolbox::{ConfiguredNativeTool, Toolbox},
    },
};

#[cfg(test)]
use double_trait::double;

/// Interact with tool storage.
///
/// Whereas the [`ToolRuntimeApi`] allows to interact with tools (and does not care that they are
/// implemented with different MCP servers), the [`ToolStoreApi`] allows someone else (e.g.
/// the `McpActor`) to notify about new or removed tools.
#[cfg_attr(test, double(ToolStoreDouble))]
pub trait ToolStoreApi {
    /// Update the list of tools known to the `ToolRuntime`.
    ///
    /// With the current exception of the native tools.
    fn report_updated_tools(
        &self,
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    ) -> impl Future<Output = ()> + Send;

    // These could become a separate interface, if we decide to have a separate actor for managing
    // MCP servers.
    fn native_tool_upsert(&self, tool: ConfiguredNativeTool) -> impl Future<Output = ()> + Send;

    fn native_tool_remove(&self, tool: ConfiguredNativeTool) -> impl Future<Output = ()> + Send;
}

/// CSI facing interface, allows to invoke and list tools
#[cfg_attr(test, double(ToolRuntimeDouble))]
pub trait ToolRuntimeApi {
    fn invoke_tool(
        &self,
        tool: QualifiedToolName,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send;

    fn list_tools(&self, namespace: Namespace)
    -> impl Future<Output = Vec<ToolDescription>> + Send;
}

pub struct ToolRuntime {
    handle: JoinHandle<()>,
    send: mpsc::Sender<ToolMsg>,
}

impl ToolRuntime {
    pub fn new() -> Self {
        let (send, receiver) = tokio::sync::mpsc::channel::<ToolMsg>(1);
        let mut actor = ToolActor::new(receiver);
        let handle = tokio::spawn(async move { actor.run().await });
        Self { handle, send }
    }

    pub fn api(&self) -> ToolRuntimeSender {
        ToolRuntimeSender(self.send.clone())
    }

    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

impl ToolRuntimeApi for ToolRuntimeSender {
    async fn invoke_tool(
        &self,
        tool: QualifiedToolName,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
    ) -> Result<ToolOutput, ToolError> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::InvokeTool {
            name: tool,
            arguments,
            tracing_context,
            send,
        };

        // We know that the receiver is still alive as long as Tool is alive.
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }

    async fn list_tools(&self, namespace: Namespace) -> Vec<ToolDescription> {
        let (send, receive) = oneshot::channel();
        let msg = ToolMsg::ListTools { send, namespace };
        self.0.send(msg).await.unwrap();
        receive.await.unwrap()
    }
}

/// Opaque wrapper around a sender to the tool actor, so we do not need to expose our message
/// type.
#[derive(Clone)]
pub struct ToolRuntimeSender(mpsc::Sender<ToolMsg>);

impl ToolStoreApi for ToolRuntimeSender {
    async fn native_tool_upsert(&self, tool: ConfiguredNativeTool) {
        let msg = ToolMsg::UpsertNativeTool { tool };
        self.0.send(msg).await.unwrap();
    }

    async fn native_tool_remove(&self, tool: ConfiguredNativeTool) {
        let msg = ToolMsg::RemoveNativeTool { tool };
        self.0.send(msg).await.unwrap();
    }

    async fn report_updated_tools(
        &self,
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    ) {
        let msg = ToolMsg::UpdatedTools { tools };
        self.0.send(msg).await.unwrap();
    }
}

enum ToolMsg {
    InvokeTool {
        name: QualifiedToolName,
        arguments: Vec<Argument>,
        tracing_context: TracingContext,
        send: oneshot::Sender<Result<Vec<Modality>, ToolError>>,
    },
    ListTools {
        namespace: Namespace,
        send: oneshot::Sender<Vec<ToolDescription>>,
    },
    UpsertNativeTool {
        tool: ConfiguredNativeTool,
    },
    RemoveNativeTool {
        tool: ConfiguredNativeTool,
    },
    UpdatedTools {
        tools: HashMap<QualifiedToolName, Arc<dyn Tool + Send + Sync>>,
    },
}

struct ToolActor {
    toolbox: Toolbox,
    receiver: mpsc::Receiver<ToolMsg>,
    running_requests: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl ToolActor {
    fn new(receiver: mpsc::Receiver<ToolMsg>) -> Self {
        Self {
            toolbox: Toolbox::new(),
            receiver,
            running_requests: FuturesUnordered::new(),
        }
    }

    async fn run(&mut self) {
        // While there are messages and running requests, poll both.
        // If there is a message, add it to the queue.
        // If there are running requests, make progress on them.
        loop {
            select! {
                msg = self.receiver.recv() => match msg {
                    Some(msg) => self.act(msg),
                    None => break
                },
                () = self.running_requests.select_next_some(), if !self.running_requests.is_empty() => {}
            };
        }
    }

    fn act(&mut self, msg: ToolMsg) {
        match msg {
            ToolMsg::InvokeTool {
                name: qualified_name,
                arguments,
                tracing_context,
                send,
            } => {
                let maybe_tool = self.toolbox.fetch_tool(&qualified_name);
                if let Some(tool) = maybe_tool {
                    self.running_requests.push(Box::pin(async move {
                        let result = tool.invoke(arguments, tracing_context).await;
                        drop(send.send(result));
                    }));
                } else {
                    drop(send.send(Err(ToolError::ToolNotFound(qualified_name.name))));
                }
            }
            ToolMsg::ListTools { namespace, send } => {
                let tools = self.toolbox.list_tools_in_namespace(&namespace);
                drop(send.send(tools));
            }
            ToolMsg::UpdatedTools { tools } => {
                self.toolbox.update_tools(tools);
            }
            ToolMsg::UpsertNativeTool { tool } => self.toolbox.upsert_native_tool(tool),
            ToolMsg::RemoveNativeTool { tool } => self.toolbox.remove_native_tool(tool),
        }
    }
}

pub struct InvokeRequest {
    pub name: String,
    pub arguments: Vec<Argument>,
}

#[cfg(test)]
pub mod tests {
    use std::{sync::Arc, time::Duration};

    use async_trait::async_trait;
    use double_trait::Dummy;
    use serde_json::json;

    use crate::{
        logging::TracingContext,
        tool::{ToolDouble, ToolRuntime, actor::ToolStoreApi},
    };

    use super::*;

    #[tokio::test]
    async fn invoking_unknown_tool_without_servers_gives_tool_not_found() {
        // Given a tool client
        let tool = ToolRuntime::new().api();

        // When we invoke an unknown tool
        let tool_name = QualifiedToolName {
            namespace: Namespace::dummy(),
            name: "unknown".to_owned(),
        };
        let arguments = Vec::new();
        let result = tool
            .invoke_tool(tool_name, arguments, TracingContext::dummy())
            .await;

        // Then we get a tool not found error
        assert!(matches!(result, Err(ToolError::ToolNotFound(_))));
    }

    #[tokio::test]
    async fn tool_from_different_namespace_is_not_available() {
        // Given an mcp server available for the foo namespace
        let foo = Namespace::new("foo").unwrap();
        let tool = ToolRuntime::new().api();
        tool.report_updated_tools(HashMap::from([(
            QualifiedToolName {
                namespace: foo.clone(),
                name: "add".to_owned(),
            },
            Arc::new(Dummy) as Arc<dyn Tool + Send + Sync>,
        )]))
        .await;

        // When we invoke a tool from a different namespace
        let tool_name = QualifiedToolName {
            namespace: Namespace::new("bar").unwrap(),
            name: "add".to_owned(),
        };
        let arguments = vec![];
        let result = tool
            .invoke_tool(tool_name, arguments, TracingContext::dummy())
            .await;

        // Then we get a tool not found error
        assert!(matches!(result, Err(ToolError::ToolNotFound(_))));
    }

    #[tokio::test]
    async fn correct_tool_is_called() {
        // Given a tool named search and another tool
        struct ToolStub;
        #[async_trait]
        impl ToolDouble for ToolStub {
            async fn invoke(
                &self,
                _: Vec<Argument>,
                _: TracingContext,
            ) -> Result<Vec<Modality>, ToolError> {
                Ok(vec![Modality::Text {
                    text: "search result".to_owned(),
                }])
            }
        }
        let namespace = Namespace::new("test").unwrap();
        let tool_runtime = ToolRuntime::new().api();
        let tools = HashMap::from([
            (
                QualifiedToolName {
                    namespace: namespace.clone(),
                    name: "search".to_owned(),
                },
                Arc::new(ToolStub) as Arc<dyn Tool + Send + Sync>,
            ),
            (
                QualifiedToolName {
                    namespace: namespace.clone(),
                    name: "other_tool".to_owned(),
                },
                Arc::new(Dummy),
            ),
        ]);
        tool_runtime.report_updated_tools(tools).await;

        // When we invoke the search tool
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "search".to_owned(),
        };
        let result = tool_runtime
            .invoke_tool(tool_name, vec![], TracingContext::dummy())
            .await
            .unwrap();

        // Then we get the result from the brave_search server
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "search result"));
    }

    #[tokio::test]
    async fn tools_are_listed() {
        struct ToolStub;

        impl ToolDouble for ToolStub {
            fn description(&self) -> ToolDescription {
                ToolDescription::new("catch_fish", "Catch a fish", json!({}))
            }
        }
        // Given a tools configured for a namespace
        let namespace = Namespace::new("test").unwrap();
        let tool_runtime = ToolRuntime::new().api();
        tool_runtime
            .report_updated_tools(HashMap::from([(
                QualifiedToolName {
                    namespace: namespace.clone(),
                    name: "add".to_owned(),
                },
                Arc::new(ToolStub) as Arc<dyn Tool + Send + Sync>,
            )]))
            .await;

        // When listing listing tools for that namespace
        let result = tool_runtime.list_tools(namespace).await;

        // Then we get the tool description from the tool
        assert_eq!(
            result,
            vec![ToolDescription::new(
                "catch_fish",
                "Catch a fish",
                json!({})
            )]
        );
    }

    #[tokio::test]
    async fn tool_calls_are_processed_in_parallel() {
        // Given a tool that hangs forever for some tool invocation requests
        struct PendingTool;
        #[async_trait::async_trait]
        impl ToolDouble for PendingTool {
            async fn invoke(
                &self,
                _arguments: Vec<Argument>,
                _tracing_context: TracingContext,
            ) -> Result<Vec<Modality>, ToolError> {
                std::future::pending().await
            }
        }
        struct SuccessfulTool;
        #[async_trait::async_trait]
        impl ToolDouble for SuccessfulTool {
            async fn invoke(
                &self,
                _arguments: Vec<Argument>,
                _tracing_context: TracingContext,
            ) -> Result<Vec<Modality>, ToolError> {
                Ok(vec![Modality::Text {
                    text: "Success".to_owned(),
                }])
            }
        }
        let namespace = Namespace::new("test").unwrap();
        let tool = ToolRuntime::new();
        let api = tool.api();
        let tools = HashMap::from([
            (
                QualifiedToolName {
                    namespace: namespace.clone(),
                    name: "success".to_owned(),
                },
                Arc::new(SuccessfulTool) as Arc<dyn Tool + Send + Sync>,
            ),
            (
                QualifiedToolName {
                    namespace: namespace.clone(),
                    name: "pending".to_owned(),
                },
                Arc::new(PendingTool) as Arc<dyn Tool + Send + Sync>,
            ),
        ]);
        api.report_updated_tools(tools).await;

        // When one hanging request is in progress
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "pending".to_owned(),
        };
        let cloned_api = api.clone();
        let handle = tokio::spawn(async move {
            drop(
                cloned_api
                    .invoke_tool(tool_name, vec![], TracingContext::dummy())
                    .await,
            );
        });

        // And waiting shortly for the message to arrive
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Then another request can still be answered
        let tool_name = QualifiedToolName {
            namespace: namespace.clone(),
            name: "success".to_owned(),
        };
        let result = api
            .invoke_tool(tool_name, vec![], TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Modality::Text { text } if text == "Success"));

        drop(handle);
    }
}
