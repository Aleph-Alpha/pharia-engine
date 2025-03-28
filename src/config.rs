use anyhow::Ok;
use config::{Case, Config, Environment, File, FileFormat, FileSourceFile};
use jiff::SignedDuration;
use serde::{Deserialize, Deserializer};
use std::{net::SocketAddr, str::FromStr, time::Duration};

use crate::{feature_set::FeatureSet, namespace_watcher::NamespaceConfigs};

mod defaults {
    use std::{net::SocketAddr, time::Duration};

    pub fn kernel_address() -> SocketAddr {
        "0.0.0.0:8081".parse().unwrap()
    }

    pub fn metrics_address() -> SocketAddr {
        "0.0.0.0:9000".parse().unwrap()
    }

    pub fn inference_url() -> String {
        "https://inference-api.product.pharia.com".to_owned()
    }

    pub fn document_index_url() -> String {
        "https://document-index.product.pharia.com".to_owned()
    }

    pub fn authorization_url() -> String {
        "https://pharia-iam.product.pharia.com".to_owned()
    }

    pub fn namespace_update_interval() -> Duration {
        Duration::from_secs(10)
    }

    pub fn log_level() -> String {
        "info".to_owned()
    }
}

fn deserialize_feature_set<'de, D>(deserializer: D) -> Result<FeatureSet, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;

    FeatureSet::from_str(&buf).map_err(serde::de::Error::custom)
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct AppConfig {
    #[serde(default, deserialize_with = "deserialize_feature_set")]
    pharia_ai_feature_set: FeatureSet,
    #[serde(default = "defaults::kernel_address")]
    kernel_address: SocketAddr,
    /// Address to expose metrics on
    #[serde(default = "defaults::metrics_address")]
    metrics_address: SocketAddr,
    /// This base URL is used to do inference against models hosted by the Aleph Alpha inference
    /// stack, as well as used to fetch Tokenizers for said models.
    #[serde(default = "defaults::inference_url")]
    inference_url: String,
    /// This base URL is used to do search hosted by the Aleph Alpha Document Index.
    #[serde(default = "defaults::document_index_url")]
    document_index_url: String,
    /// This base URL is used to authorize an `PHARIA_AI_TOKEN` for use by the kernel
    #[serde(default = "defaults::authorization_url")]
    authorization_url: String,
    #[serde(default)]
    namespaces: NamespaceConfigs,
    #[serde(
        deserialize_with = "positive_duration_from_str",
        default = "defaults::namespace_update_interval"
    )]
    namespace_update_interval: Duration,
    #[serde(default = "defaults::log_level")]
    log_level: String,
    otel_endpoint: Option<String>,
    #[serde(default)]
    use_pooling_allocator: bool,
}

/// `jiff::SignedDuration` can parse human readable strings like `1h30m` into `SignedDuration`.
/// However, we want to ensure for this config that the duration is positive.
fn positive_duration_from_str<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let duration_str = String::deserialize(deserializer)?;
    let duration: SignedDuration = duration_str.parse().map_err(serde::de::Error::custom)?;
    Duration::try_from(duration).map_err(serde::de::Error::custom)
}

impl AppConfig {
    /// # Panics
    ///
    /// Will panic if the environment variables `inference_url` or `authorization_url` are provided but empty.
    ///
    /// # Errors
    ///
    /// Cannot parse operator config from the provided file or the environment variables.
    pub fn new() -> anyhow::Result<Self> {
        let file = File::with_name("config.toml").required(false);
        let env = Self::environment();
        Self::from_sources(file, env)
    }

    fn from_sources(
        file: File<FileSourceFile, FileFormat>,
        env: Environment,
    ) -> anyhow::Result<Self> {
        let mut config = Config::builder()
            .add_source(file)
            .add_source(env)
            .build()?
            .try_deserialize::<Self>()?;

        assert!(
            !config.inference_url.is_empty(),
            "The inference address must be available."
        );

        assert!(
            !config.authorization_url.is_empty(),
            "The authorization address must be available."
        );

        if ["debug", "trace"].contains(&config.log_level.as_str()) {
            // Don't allow third-party crates to go below info unless they user passed in a
            // RUST_LOG formatted string themselves
            config.log_level = format!("info,pharia_kernel={}", config.log_level);
        }

        Ok(config)
    }

    /// A namespace can contain the characters `[a-z0-9-]` e.g. `pharia-kernel-team`.
    ///
    /// As only `SCREAMING_SNAKE_CASE` is widely supported for environment variable keys,
    /// we support it by converting each key into `kebab-case`.
    /// Because we have a nested configuration, we use double underscores as the separators.
    fn environment() -> Environment {
        Environment::with_convert_case(Case::Kebab).separator("__")
    }

    #[must_use]
    pub fn pharia_ai_feature_set(&self) -> FeatureSet {
        self.pharia_ai_feature_set
    }

    #[must_use]
    pub fn with_pharia_ai_feature_set(mut self, feature_set: FeatureSet) -> Self {
        self.pharia_ai_feature_set = feature_set;
        self
    }

    #[must_use]
    pub fn kernel_address(&self) -> SocketAddr {
        self.kernel_address
    }

    #[must_use]
    pub fn with_kernel_address(mut self, addr: SocketAddr) -> Self {
        self.kernel_address = addr;
        self
    }

    #[must_use]
    pub fn metrics_address(&self) -> SocketAddr {
        self.metrics_address
    }

    #[must_use]
    pub fn with_metrics_address(mut self, addr: SocketAddr) -> Self {
        self.metrics_address = addr;
        self
    }

    #[must_use]
    pub fn inference_url(&self) -> &str {
        &self.inference_url
    }

    #[must_use]
    pub fn with_inference_url(mut self, url: String) -> Self {
        self.inference_url = url;
        self
    }

    #[must_use]
    pub fn document_index_url(&self) -> &str {
        &self.document_index_url
    }

    #[must_use]
    pub fn with_document_index_url(mut self, url: String) -> Self {
        self.document_index_url = url;
        self
    }

    #[must_use]
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    #[must_use]
    pub fn with_authorization_url(mut self, url: String) -> Self {
        self.authorization_url = url;
        self
    }

    #[must_use]
    pub fn namespaces(&self) -> &NamespaceConfigs {
        &self.namespaces
    }

    #[must_use]
    pub fn with_namespaces(mut self, namespaces: NamespaceConfigs) -> Self {
        self.namespaces = namespaces;
        self
    }

    #[must_use]
    pub fn namespace_update_interval(&self) -> Duration {
        self.namespace_update_interval
    }

    #[must_use]
    pub fn with_namespace_update_interval(mut self, interval: Duration) -> Self {
        self.namespace_update_interval = interval;
        self
    }

    #[must_use]
    pub fn log_level(&self) -> &str {
        &self.log_level
    }

    #[must_use]
    pub fn with_log_level(mut self, level: String) -> Self {
        self.log_level = level;
        self
    }

    #[must_use]
    pub fn otel_endpoint(&self) -> Option<&str> {
        self.otel_endpoint.as_deref()
    }

    #[must_use]
    pub fn with_otel_endpoint(mut self, endpoint: Option<String>) -> Self {
        self.otel_endpoint = endpoint;
        self
    }

    #[must_use]
    pub fn use_pooling_allocator(&self) -> bool {
        self.use_pooling_allocator
    }

    #[must_use]
    pub fn with_pooling_allocator(mut self, use_pooling: bool) -> Self {
        self.use_pooling_allocator = use_pooling;
        self
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pharia_ai_feature_set: FeatureSet::default(),
            kernel_address: defaults::kernel_address(),
            metrics_address: defaults::metrics_address(),
            inference_url: defaults::inference_url(),
            document_index_url: defaults::document_index_url(),
            authorization_url: defaults::authorization_url(),
            namespaces: NamespaceConfigs::default(),
            namespace_update_interval: defaults::namespace_update_interval(),
            log_level: defaults::log_level(),
            otel_endpoint: None,
            use_pooling_allocator: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::{collections::HashMap, fs, time::Duration};

    use config::Config;
    use tempfile::tempdir;

    use crate::feature_set::PRODUCTION_FEATURE_SET;
    use crate::namespace_watcher::tests::NamespaceConfig;
    use crate::namespace_watcher::{Namespace, Registry};

    use super::*;

    #[test]
    fn load_debug_log_level() -> anyhow::Result<()> {
        // Given a hashmap with debug log level
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([("LOG_LEVEL".to_owned(), "debug".to_owned())]);
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When we build the source from the environment variables
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then the debug log level is only applied for PhariaKernel
        assert_eq!(config.log_level(), "info,pharia_kernel=debug");
        Ok(())
    }

    #[test]
    fn load_app_config_from_one_source() -> anyhow::Result<()> {
        // Given a hashmap with variables
        let env_vars = HashMap::from([
            ("PHARIA_AI_FEATURE_SET".to_owned(), "42".to_owned()),
            ("KERNEL_ADDRESS".to_owned(), "192.123.1.1:8081".to_owned()),
            ("METRICS_ADDRESS".to_owned(), "0.0.0.0:9000".to_owned()),
            (
                "INFERENCE_URL".to_owned(),
                "https://inference-api.product.pharia.com".to_owned(),
            ),
            (
                "DOCUMENT_INDEX_URL".to_owned(),
                "https://document-index.product.pharia.com".to_owned(),
            ),
            (
                "AUTHORIZATION_URL".to_owned(),
                "https://pharia-iam.product.pharia.com".to_owned(),
            ),
            ("NAMESPACE_UPDATE_INTERVAL".to_owned(), "10s".to_owned()),
            ("LOG_LEVEL".to_owned(), "dummy".to_owned()),
            ("OTEL_ENDPOINT".to_owned(), "open-telemetry".to_owned()),
            ("USE_POOLING_ALLOCATOR".to_owned(), "true".to_owned()),
            ("NAMESPACES__DEV__DIRECTORY".to_owned(), "skills".to_owned()),
        ]);
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When we build the source from the environment variables
        let config = AppConfig::from_sources(file_source, env_source)?;

        assert_eq!(config.pharia_ai_feature_set(), FeatureSet::Stable(42));
        assert_eq!(config.kernel_address(), "192.123.1.1:8081".parse().unwrap());
        assert_eq!(config.log_level(), "dummy");
        assert_eq!(config.namespaces().len(), 1);
        assert_eq!(config.namespace_update_interval(), Duration::from_secs(10));
        Ok(())
    }

    #[test]
    fn load_default_app_config() -> anyhow::Result<()> {
        // Given a config without any sources
        let config = Config::builder().build()?;

        // When we deserialize it into AppConfig
        let config = config.try_deserialize::<AppConfig>()?;

        // Then the config contains the default values
        assert_eq!(config.pharia_ai_feature_set(), PRODUCTION_FEATURE_SET);
        assert_eq!(config.kernel_address(), "0.0.0.0:8081".parse().unwrap());
        assert_eq!(config.metrics_address(), "0.0.0.0:9000".parse().unwrap());
        assert_eq!(
            config.inference_url(),
            "https://inference-api.product.pharia.com"
        );
        assert_eq!(
            config.document_index_url(),
            "https://document-index.product.pharia.com"
        );
        assert_eq!(
            config.authorization_url(),
            "https://pharia-iam.product.pharia.com"
        );
        assert_eq!(config.namespace_update_interval(), Duration::from_secs(10));
        assert_eq!(config.log_level(), "info");
        assert!(config.otel_endpoint().is_none());
        assert!(!config.use_pooling_allocator());
        assert!(config.namespaces().is_empty());
        Ok(())
    }

    #[test]
    fn empty_namespace_name_is_rejected() -> anyhow::Result<()> {
        // Given toml file with non kebab-case namespaces
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            r#"[namespaces.""]
            directory = "skills""#
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_source = AppConfig::environment().source(Some(HashMap::new()));

        // When loading from the sources
        let error = AppConfig::from_sources(file_source, env_source).unwrap_err();

        // Then we receive an error
        assert!(
            error
                .to_string()
                .to_lowercase()
                .contains("invalid namespace")
        );
        Ok(())
    }

    #[test]
    fn load_non_kebab_case_namespace_name() -> anyhow::Result<()> {
        // Given toml file with non kebab-case namespaces
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            r#"[namespaces.-myteam]
            directory = "skills""#
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_source = AppConfig::environment().source(Some(HashMap::new()));

        // When loading from the sources
        let error = AppConfig::from_sources(file_source, env_source).unwrap_err();

        // Then we receive an error
        assert!(
            error
                .to_string()
                .to_lowercase()
                .contains("invalid namespace")
        );
        Ok(())
    }

    #[test]
    fn load_from_two_empty_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::new();
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When loading from the sources
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then both sources are applied, with the values from environment variables having precedence
        assert_eq!(config.namespaces().len(), 0);
        Ok(())
    }

    #[test]
    fn load_two_namespaces_from_independent_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            r#"[namespaces.a]
config-url = "a"
config-access-token = "a"
registry = "a"
base-repository = "a"
registry-user =  "a"
registry-password =  "a""#
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([
            ("NAMESPACES__B__CONFIG_URL".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__CONFIG_ACCESS_TOKEN".to_owned(),
                "b".to_owned(),
            ),
            ("NAMESPACES__B__REGISTRY".to_owned(), "b".to_owned()),
            ("NAMESPACES__B__BASE_REPOSITORY".to_owned(), "b".to_owned()),
            ("NAMESPACES__B__REGISTRY_USER".to_owned(), "b".to_owned()),
            (
                "NAMESPACES__B__REGISTRY_PASSWORD".to_owned(),
                "b".to_owned(),
            ),
        ]);
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When loading from the sources
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then both namespaces are loaded
        assert_eq!(config.namespaces().len(), 2);
        let namespace_a = Namespace::new("a").unwrap();
        assert!(config.namespaces().contains_key(&namespace_a));
        let namespace_b = Namespace::new("b").unwrap();
        assert!(config.namespaces().contains_key(&namespace_b));
        Ok(())
    }

    #[test]
    fn load_one_namespace_from_two_partial_sources() -> anyhow::Result<()> {
        // Given a TOML file and environment variables
        let config_url = "https://acme.com/latest/config.toml";
        let config_access_token = "ACME_CONFIG_ACCESS_TOKEN";
        let registry = "registry.acme.com";
        let base_repository = "engineering/skills";
        let user = "DUMMY_USER";
        let password = "DUMMY_PASSWORD";
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        let mut file = fs::File::create_new(&file_path)?;
        writeln!(
            file,
            "[namespaces.acme]
config-access-token = \"{config_access_token}\"
registry = \"{registry}\"
base-repository = \"{base_repository}\"
registry-password =  \"{password}\"
        "
        )?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([
            (
                "NAMESPACES__ACME__CONFIG_URL".to_owned(),
                config_url.to_owned(),
            ),
            (
                "NAMESPACES__ACME__REGISTRY_USER".to_owned(),
                user.to_owned(),
            ),
        ]);
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When loading from the sources
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then both sources are applied, with the values from environment variables having higher precedence
        assert_eq!(config.namespaces().len(), 1);
        let namespace_config = NamespaceConfig::TeamOwned {
            config_url: config_url.to_owned(),
            config_access_token: Some(config_access_token.to_owned()),
            registry: Registry::Oci {
                registry: registry.to_owned(),
                base_repository: base_repository.to_owned(),
                user: user.to_owned(),
                password: password.to_owned(),
            },
        };
        let namespace = Namespace::new("acme").unwrap();
        assert_eq!(
            config.namespaces().get(&namespace).unwrap(),
            &namespace_config
        );
        Ok(())
    }

    #[test]
    fn reads_from_file() {
        drop(dotenvy::dotenv());
        let config = AppConfig::new().unwrap();
        let namespace = Namespace::new("pharia-kernel-team").unwrap();
        assert!(config.namespaces().contains_key(&namespace));
    }
}
