mod actor;
mod config;
mod skill_config;

pub use self::actor::ConfigurationObserver;
pub use self::config::OperatorConfig;
pub use self::skill_config::{skill_config_from_url, NamespaceConfig};

pub mod tests {
    pub use super::skill_config::{LocalSkillConfig, Skill};
}
