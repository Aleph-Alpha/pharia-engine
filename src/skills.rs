mod actor;
mod runtime;
pub use actor::{SkillExecutor, SkillExecutorApi};

#[derive(Debug)]
pub struct SkillPath {
    pub namespace: String,
    pub name: String,
}

impl SkillPath {
    fn from_str(s: &str) -> Self {
        let (namespace, name) = s.split_once('/').unwrap_or(("pharia-kernel-team", s));
        Self::new(namespace, name)
    }

    pub fn new(namespace: &str, name: &str) -> Self {
        Self {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        }
    }
}

#[cfg(test)]
pub mod tests {
    pub use super::actor::tests::LiarRuntime;
    pub use super::actor::SkillExecutorMessage;
}
