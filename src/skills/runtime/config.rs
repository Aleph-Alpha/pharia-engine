use std::{env, path::Path};

use async_trait::async_trait;
use axum::http::HeaderValue;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
}

#[async_trait]
pub trait SkillConfig {
    async fn skills(&mut self) -> &[Skill];
}

#[derive(Debug, Serialize, Deserialize)]
struct TomlConfig {
    skills: Vec<Skill>,
}

impl TomlConfig {
    pub fn from_str(skill_config: &str) -> anyhow::Result<Self> {
        let tc = toml::from_str(skill_config)?;
        Ok(tc)
    }
}

pub struct LocalConfig {
    skills: Vec<Skill>,
}

impl LocalConfig {
    pub fn from_default_file() -> Option<Self> {
        Self::new("skill_config.toml").ok()
    }

    pub fn new<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
        let config = std::fs::read_to_string(p)?;
        let skills = TomlConfig::from_str(&config)?.skills;
        Ok(Self { skills })
    }
}

#[async_trait]
impl SkillConfig for LocalConfig {
    async fn skills(&mut self) -> &[Skill] {
        &self.skills
    }
}

pub struct RemoteConfig {
    skills: Vec<Skill>,
    token: String,
    url: String,
    sync_interval: u64,
    last_sync: Option<std::time::Instant>,
}

impl RemoteConfig {
    pub fn from_env() -> Self {
        drop(dotenvy::dotenv());
        let token = env::var("REMOTE_SKILL_CONFIG_TOKEN")
            .expect("Remote skill config token must be provided.");
        let url =
            env::var("REMOTE_SKILL_CONFIG_URL").expect("Remote skill config URL must be provided.");
        let sync_interval = env::var("REMOTE_SKILL_CONFIG_UPDATE_INTERVAL_SEC")
            .unwrap_or("60".to_owned())
            .parse::<u64>()
            .expect("Failed to parse remote skill config update interval.");
        let skills = vec![];
        Self {
            skills,
            token,
            url,
            sync_interval,
            last_sync: None,
        }
    }

    fn expired(&self) -> bool {
        if let Some(last_sync) = self.last_sync {
            let elapsed = last_sync.elapsed();
            elapsed.as_secs() > self.sync_interval
        } else {
            true
        }
    }

    async fn load(&self) -> anyhow::Result<Vec<Skill>> {
        let mut auth_value = HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap();
        auth_value.set_sensitive(true);
        let client = reqwest::Client::new();
        let resp = client
            .get(&self.url)
            .header(AUTHORIZATION, auth_value)
            .send()
            .await?;
        let content = resp.text().await?;
        let config = TomlConfig::from_str(&content)?;
        Ok(config.skills)
    }

    async fn sync(&mut self) {
        if self.expired() {
            match self.load().await {
                Ok(skills) => {
                    self.skills = skills;
                    self.last_sync = Some(std::time::Instant::now());
                }
                Err(e) => {
                    warn!("Failed to load remote skill config, fallback to existing config: {e}");
                }
            }
        }
    }
}

#[async_trait]
impl SkillConfig for RemoteConfig {
    async fn skills(&mut self) -> &[Skill] {
        self.sync().await;
        &self.skills
    }
}

#[cfg(test)]
pub mod tests {
    use crate::skills::runtime::config::TomlConfig;

    use super::*;

    pub struct StubConfig {
        skills: Vec<Skill>,
    }

    impl StubConfig {
        pub fn new(skills: &[&str]) -> Self {
            let skills = skills
                .iter()
                .map(|&s| Skill { name: s.to_owned() })
                .collect();
            Self { skills }
        }
    }

    #[async_trait]
    impl SkillConfig for StubConfig {
        async fn skills(&mut self) -> &[Skill] {
            &self.skills
        }
    }

    impl RemoteConfig {
        pub fn set_last_sync(&mut self, last_sync: std::time::Instant) {
            self.last_sync = Some(last_sync);
        }
    }

    #[test]
    fn remote_config_expired_after_creation() {
        let config = RemoteConfig::from_env();
        assert!(config.expired());
    }

    #[test]
    fn remote_config_expired_if_last_sync_is_yesterday() {
        let mut config = RemoteConfig::from_env();
        let yesterday = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(86400))
            .unwrap();
        config.set_last_sync(yesterday);

        assert!(config.expired());
    }

    #[test]
    fn remote_config_not_expired_if_just_synced() {
        let mut config = RemoteConfig::from_env();
        let now = std::time::Instant::now();
        config.set_last_sync(now);

        assert!(!config.expired());
    }
    #[test]
    fn load_skill_list_config_toml() {
        let tc: TomlConfig = toml::from_str(
            r#"
            skills = [
                {name = "Goofy"},
                {name = "Pluto"},
                {name = "Gamma"}
            ]
            "#,
        )
        .unwrap();
        assert_eq!(tc.skills.len(), 3);
    }

    #[tokio::test]
    async fn load_gitlab_config() {
        // Given a gitlab skill config
        let mut config = RemoteConfig::from_env();

        // when fetch skill config
        let result = config.load().await;

        // then the configured skills must listed in the config
        assert!(result.is_ok());
        assert!(!config.skills().await.is_empty());
    }
}
