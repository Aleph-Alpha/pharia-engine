mod actor;
mod runtime;
use std::fmt;

pub use actor::{ExecuteSkillError, SkillExecutor, SkillExecutorApi, SkillExecutorConfig};
pub use runtime::{SkillProviderActorHandle, SkillProviderApi};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillPath {
    pub namespace: String,
    pub name: String,
}

impl SkillPath {
    pub fn from_str(s: &str) -> Self {
        let (namespace, name) = s.split_once('/').unwrap_or(("pharia-kernel-team", s));
        Self::new(namespace, name)
    }

    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
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
    use fake::faker::company::en::{Buzzword, CompanyName};
    use fake::{Dummy, Fake, Faker};
    use rand::Rng;

    pub use super::actor::SkillExecutorMessage;
    pub use super::runtime::{tests::dummy_skill_provider_api, SkillProviderMsg};
    use super::SkillPath;

    impl SkillPath {
        pub fn dummy() -> Self {
            Faker.fake()
        }
    }

    impl Dummy<Faker> for SkillPath {
        fn dummy_with_rng<R: Rng + ?Sized>(_config: &Faker, rng: &mut R) -> Self {
            Self {
                namespace: Fake::fake_with_rng::<_, _>(&CompanyName(), rng),
                name: Fake::fake_with_rng::<_, _>(&Buzzword(), rng),
            }
        }
    }
}
