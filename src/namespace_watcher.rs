mod actor;
mod config;
mod namespace_description;

pub use self::actor::{NamespaceDescriptionLoaders, NamespaceWatcher};
pub use self::config::{NamespaceConfig, OperatorConfig, Registry};
pub use self::namespace_description::NamespaceDescriptionLoader;

#[cfg(test)]
pub mod tests {
    pub use super::namespace_description::SkillDescription;
}
