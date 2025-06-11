use tokio::sync::mpsc;

use crate::{
    authorization::{AuthorizationMsg, AuthorizationProvider},
    csi::CsiDrivers,
    csi_shell::CsiProvider,
    inference::InferenceSender,
    search::SearchSender,
    shell::AppState,
    skill_runtime::{SkillRuntimeMsg, SkillRuntimeProvider},
    skill_store::{SkillStoreMsg, SkillStoreProvider},
    tokenizers::TokenizerSender,
    tool::{McpServerStoreProvider, ToolProvider, ToolSender},
};

type CsiDriversImpl = CsiDrivers<InferenceSender, SearchSender, TokenizerSender, ToolSender>;

#[derive(Clone)]
pub struct ShellState {
    skill_runtime: mpsc::Sender<SkillRuntimeMsg>,
    skill_store: mpsc::Sender<SkillStoreMsg>,
    authorization: mpsc::Sender<AuthorizationMsg>,
    csi_drivers: CsiDriversImpl,
}

impl ShellState {
    pub fn new(
        skill_runtime: mpsc::Sender<SkillRuntimeMsg>,
        skill_store: mpsc::Sender<SkillStoreMsg>,
        authorization: mpsc::Sender<AuthorizationMsg>,
        csi_drivers: CsiDriversImpl,
    ) -> Self {
        Self {
            skill_runtime,
            skill_store,
            authorization,
            csi_drivers,
        }
    }
}

impl SkillRuntimeProvider for ShellState {
    type SkillRuntime = mpsc::Sender<SkillRuntimeMsg>;

    fn skill_runtime(&self) -> &Self::SkillRuntime {
        &self.skill_runtime
    }
}

impl AuthorizationProvider for ShellState {
    type Authorization = mpsc::Sender<AuthorizationMsg>;

    fn authorization(&self) -> &Self::Authorization {
        &self.authorization
    }
}

impl CsiProvider for ShellState {
    type Csi = CsiDriversImpl;

    fn csi(&self) -> &Self::Csi {
        &self.csi_drivers
    }
}

impl SkillStoreProvider for ShellState {
    type SkillStore = mpsc::Sender<SkillStoreMsg>;

    fn skill_store(&self) -> &Self::SkillStore {
        &self.skill_store
    }
}

impl ToolProvider for ShellState {
    type Tool = ToolSender;

    fn tool(&self) -> &Self::Tool {
        &self.csi_drivers.tool
    }
}

impl McpServerStoreProvider for ShellState {
    type McpServerStore = ToolSender;

    fn mcp_server_store(&self) -> &Self::McpServerStore {
        &self.csi_drivers.tool
    }
}

impl AppState for ShellState {}
