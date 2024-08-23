mod actor;
mod config;
mod skill_config;

pub use self::actor::{ConfigImpl, ConfigurationObserver};
pub use self::config::{NamespaceReference, OperatorConfig};
pub use self::skill_config::{namespace_from_url, NamespaceDescriptionLoader};
