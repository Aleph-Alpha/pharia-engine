mod actor;
mod runtime;
use std::fmt;

pub use actor::{ExecuteSkillError, SkillExecutor, SkillExecutorApi, SkillRuntimeMetrics};
pub use runtime::{Engine, Skill, SupportedVersion};

use crate::namespace_watcher::Namespace;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillPath {
    pub namespace: Namespace,
    pub name: String,
}

impl SkillPath {
    pub fn new(namespace: Namespace, name: impl Into<String>) -> Self {
        Self {
            namespace,
            name: name.into(),
        }
    }
}
impl fmt::Display for SkillPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

#[cfg(test)]
pub mod tests {
    use fake::faker::company::en::Buzzword;
    use fake::{Dummy, Fake, Faker};
    use rand::Rng;

    use crate::namespace_watcher::Namespace;

    pub use super::actor::SkillExecutorMsg;
    use super::SkillPath;

    impl SkillPath {
        pub fn dummy() -> Self {
            Faker.fake()
        }

        pub fn local(name: impl Into<String>) -> Self {
            let namespace = Namespace::new("local").unwrap();
            Self {
                namespace,
                name: name.into(),
            }
        }
    }

    impl Dummy<Faker> for SkillPath {
        fn dummy_with_rng<R: Rng + ?Sized>(_config: &Faker, rng: &mut R) -> Self {
            Self {
                namespace: Namespace::new("dummy").unwrap(),
                name: Fake::fake_with_rng::<_, _>(&Buzzword(), rng),
            }
        }
    }
}
