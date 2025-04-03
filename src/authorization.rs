use std::{future::Future, pin::Pin, sync::Arc};

use axum::http::{HeaderMap, StatusCode};
use futures::{StreamExt, channel::oneshot, stream::FuturesUnordered};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use tokio::{select, sync::mpsc, task::JoinHandle};

use crate::http::HttpClient;

pub struct Authorization {
    send: mpsc::Sender<AuthorizationMsg>,
    handle: JoinHandle<()>,
}

impl Authorization {
    pub fn new(authorization_url: String) -> Self {
        let client = HttpAuthorizationClient::new(authorization_url);
        Self::with_client(client)
    }

    pub fn with_client(client: impl AuthorizationClient) -> Self {
        let (send, recv) = mpsc::channel(1);
        let mut actor = AuthorizationActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Authorization { send, handle }
    }

    pub fn api(&self) -> mpsc::Sender<AuthorizationMsg> {
        self.send.clone()
    }

    /// Authorization is going to shutdown, as soon as the last instance of [`AuthorizationApi`] is
    /// dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

struct AuthorizationActor<C: AuthorizationClient> {
    recv: mpsc::Receiver<AuthorizationMsg>,
    running_authorizations: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
    client: Arc<C>,
}

impl<C: AuthorizationClient> AuthorizationActor<C> {
    fn new(client: C, recv: mpsc::Receiver<AuthorizationMsg>) -> Self {
        Self {
            recv,
            running_authorizations: FuturesUnordered::new(),
            client: Arc::new(client),
        }
    }
    async fn run(&mut self) {
        loop {
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) =>  self.act(msg),
                    None => break
                },
                () = self.running_authorizations.select_next_some(), if !self.running_authorizations.is_empty() => {}
            }
        }
    }
    fn act(&mut self, msg: AuthorizationMsg) {
        let client = self.client.clone();
        self.running_authorizations.push(Box::pin(async move {
            msg.act(client.as_ref()).await;
        }));
    }
}

pub trait AuthorizationApi {
    fn check_permission(
        &self,
        api_token: String,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

impl AuthorizationApi for mpsc::Sender<AuthorizationMsg> {
    async fn check_permission(&self, api_token: String) -> anyhow::Result<bool> {
        let (send, recv) = oneshot::channel();
        let msg = AuthorizationMsg::Auth { api_token, send };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }
}

pub enum AuthorizationMsg {
    Auth {
        api_token: String,
        send: oneshot::Sender<anyhow::Result<bool>>,
    },
}

impl AuthorizationMsg {
    async fn act(self, client: &impl AuthorizationClient) {
        match self {
            AuthorizationMsg::Auth { api_token, send } => {
                let result = client.token_valid(api_token).await;
                drop(send.send(result));
            }
        }
    }
}

pub trait AuthorizationClient: Send + Sync + 'static {
    fn token_valid(&self, api_token: String) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

struct HttpAuthorizationClient {
    url: String,
    client: HttpClient,
}

impl HttpAuthorizationClient {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: HttpClient::default(),
        }
    }
}

impl AuthorizationClient for HttpAuthorizationClient {
    async fn token_valid(&self, api_token: String) -> anyhow::Result<bool> {
        #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(tag = "permission")]
        enum Permission {
            KernelAccess,
        }

        let required_permissions = [Permission::KernelAccess];
        let response = self
            .client
            .post(format!("{}/check_privileges", self.url))
            .headers(HeaderMap::from_iter([(
                AUTHORIZATION,
                format!("Bearer {api_token}").parse().unwrap(),
            )]))
            .json(&required_permissions)
            .send()
            .await?;

        // Response succeeded, but not allowed
        if [StatusCode::FORBIDDEN, StatusCode::UNAUTHORIZED].contains(&response.status()) {
            return Ok(false);
        }

        let allowed_permissions = response
            // Error for any other status
            .error_for_status()?
            .json::<Vec<Permission>>()
            .await?;

        // Check that we got the same list back
        Ok(allowed_permissions == required_permissions)
    }
}

#[cfg(test)]
pub mod tests {

    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;
    use crate::tests::{api_token, authorization_url};

    #[derive(Debug, Clone)]
    pub struct StubAuthorization {
        /// Whether the permission check should succeed or not
        response: bool,
    }

    impl StubAuthorization {
        pub fn new(response: bool) -> Self {
            Self { response }
        }
    }

    impl AuthorizationApi for StubAuthorization {
        async fn check_permission(&self, _api_token: String) -> anyhow::Result<bool> {
            Ok(self.response)
        }
    }

    #[tokio::test]
    async fn true_for_valid_permissions() {
        // Given a client that is configured against the inference api
        let url = authorization_url();
        let client = HttpAuthorizationClient::new(url.to_owned());

        // When the client is used to check a valid api token
        let result = client.token_valid(api_token().to_owned()).await;

        // Then the result is true
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn false_for_invalid_token() {
        // Given a client that is configured against the inference api
        let url = authorization_url();
        let client = HttpAuthorizationClient::new(url.to_owned());

        // When the client is used to check an invalid api token
        let result = client.token_valid("invalid".to_owned()).await;

        // Then the result is false
        assert!(!result.unwrap());
    }

    struct StubAuthorizationClient;

    impl AuthorizationClient for StubAuthorizationClient {
        async fn token_valid(&self, _api_token: String) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn auth_api_answers_messages() {
        // Given a stub authorization client
        let client = StubAuthorizationClient;
        let auth = Authorization::with_client(client);

        // When checking permissions for a token
        let result = timeout(
            Duration::from_millis(10),
            auth.api().check_permission("valid".to_owned()),
        )
        .await;

        // Then the message get's answered within 10ms
        assert!(result.unwrap().unwrap());
    }
}
