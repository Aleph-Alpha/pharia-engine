mod actor;
mod config;
mod namespace_description;

pub use self::actor::{NamespaceDescriptionLoaders, NamespaceWatcher};
pub use self::config::{Namespace, NamespaceConfigs, Registry};
pub use self::namespace_description::{NamespaceDescriptionLoader, SkillDescription};

#[cfg(test)]
pub mod tests {
    pub use super::config::NamespaceConfig;
}
