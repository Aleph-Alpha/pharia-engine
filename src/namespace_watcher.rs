mod actor;
mod config;
mod namespace_description;

pub use self::{
    actor::{NamespaceDescriptionLoaders, NamespaceWatcher},
    config::{Namespace, NamespaceConfigs, Registry},
    namespace_description::{NamespaceDescriptionLoader, SkillDescription},
};

#[cfg(test)]
pub mod tests {
    pub use super::config::NamespaceConfig;
}
