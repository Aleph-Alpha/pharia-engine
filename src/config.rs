use bytesize::ByteSize;
use config::{Case, Config, Environment, File, FileFormat, FileSourceFile};
use engine_room::{EngineConfig, WasmtimeCache};
use jiff::SignedDuration;
use serde::{Deserialize, Deserializer};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use crate::{
    feature_set::FeatureSet, inference::InferenceConfig, namespace_watcher::NamespaceConfigs,
    tokenizers::TokenizersConfig,
};

mod defaults {
    use std::{net::SocketAddr, time::Duration};

    pub fn kernel_address() -> SocketAddr {
        "0.0.0.0:8081".parse().unwrap()
    }

    pub fn metrics_address() -> SocketAddr {
        "0.0.0.0:9000".parse().unwrap()
    }

    pub fn namespace_update_interval() -> Duration {
        Duration::from_secs(10)
    }

    pub fn log_level() -> String {
        "info".to_owned()
    }

    pub fn otel_sampling_ratio() -> f64 {
        0.1
    }
}

fn deserialize_feature_set<'de, D>(deserializer: D) -> Result<FeatureSet, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;

    FeatureSet::from_str(&buf).map_err(serde::de::Error::custom)
}

fn deserialize_empty_string_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let url = Option::<String>::deserialize(deserializer)?
        .and_then(|url| if url.is_empty() { None } else { Some(url) });
    Ok(url)
}

fn deserialize_sampling_ratio<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let ratio = f64::deserialize(deserializer)?;
    (0.0..=1.0)
        .contains(&ratio)
        .then_some(ratio)
        .ok_or_else(|| {
            serde::de::Error::custom("otel_sampling_ratio must be between 0.0 and 1.0 inclusive")
        })
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
    /// Aleph Alpha inference base URL. It is used to do chat/completion requests against models
    /// hosted by the Aleph Alpha inference stack, as well as used to fetch Tokenizers for said
    /// models. Takes precedence over the `openai_inference_url` if both are set. If neither is set,
    /// the Kernel will run without inference capabilities. In case skills try to use inference
    /// functionality without a configured inference URL, an error is returned and skill execution
    /// is suspended.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    inference_url: Option<String>,
    /// OpenAI-compatible inference. Set these variables and do not set the `inference_url`
    /// variable in case you want to do chat requests against OpenAI-compatible inferences. Not all
    /// model capabilities will be available to Skills if using OpenAI-compatible inferences. Only
    /// chat requests are supported, completion, explanation and chunking requests are not.
    #[serde(default)]
    openai_inference: Option<OpenAiInference>,
    /// This base URL is used to do search hosted by the Aleph Alpha Document Index.
    /// The Kernel supports running without a configured Document Index. This might be useful if
    /// the Kernel runs outside of `PhariaAI`, or if no Document Index is available because of
    /// resource constraints. In case skills try to use search functionality without a configured
    /// Document Index, an error is returned and Skill execution is suspended.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    document_index_url: Option<String>,
    /// This base URL is used to authorize an `PHARIA_AI_TOKEN` for use by the kernel. The
    /// implementation is specific to the authorization service used inside `PhariaAI`. If set to
    /// `None`, the Kernel will not check the permissions of the provided tokens.
    #[serde(default, deserialize_with = "deserialize_empty_string_as_none")]
    authorization_url: Option<String>,
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
    /// OTEL sampling ratio between 0.0 and 1.0, where 0.0 means no sampling and 1.0 means all traces
    #[serde(
        default = "defaults::otel_sampling_ratio",
        deserialize_with = "deserialize_sampling_ratio"
    )]
    otel_sampling_ratio: f64,
    #[serde(default)]
    use_pooling_allocator: bool,
    /// Optionally set amount of memory requested from Kubernetes
    memory_request: Option<ByteSize>,
    /// Optionally set memory limit for Kubernetes
    memory_limit: Option<ByteSize>,
    /// Optionally set directory for wasmtime cache
    wasmtime_cache_dir: Option<PathBuf>,
    /// Optionally set amount of storage requested from Kubernetes for wasmtime cache
    wasmtime_cache_size_request: Option<ByteSize>,
    /// Optionally set storage limit for Kubernetes for wasmtime cache
    wasmtime_cache_size_limit: Option<ByteSize>,
}

/// Configuration for an OpenAI-compatible inference.
#[derive(Clone, Deserialize, Debug)]
pub struct OpenAiInference {
    /// Base URL for OpenAI-compatible inference.
    url: String,
    /// Token to authenticate with the OpenAI-compatible inference.
    token: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct OtelConfig<'a> {
    /// Endpoint that traces are sent to. If set to None, no traces are emitted
    pub endpoint: Option<&'a str>,
    /// ratio between 0.0 and 1.0, where 0.0 means no sampling and 1.0 means all traces
    pub sampling_ratio: f64,
    /// Minimum log level for traces.
    pub log_level: &'a str,
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
        // We write the errors to stderr as logging/tracing is not yet initialized
        let mut config = Config::builder()
            .add_source(file)
            .add_source(env)
            .build()
            .inspect_err(|e| {
                eprintln!("Error building app config: {e:#}");
            })?
            .try_deserialize::<Self>()
            .inspect_err(|e| {
                eprintln!("Error deserializing app config: {e:#}");
            })?;

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
    pub fn inference_url(&self) -> Option<&str> {
        self.inference_url.as_deref()
    }

    #[must_use]
    pub fn with_inference_url(mut self, url: impl Into<String>) -> Self {
        self.inference_url = Some(url.into());
        self
    }

    #[must_use]
    pub fn with_inference_config(mut self, inference_config: InferenceConfig<'_>) -> Self {
        match inference_config {
            InferenceConfig::AlephAlpha { url } => {
                self.inference_url = Some(url.to_owned());
            }
            InferenceConfig::OpenAi { url, token } => {
                self.openai_inference = Some(OpenAiInference {
                    url: url.to_owned(),
                    token: token.to_owned(),
                });
            }
            InferenceConfig::None => {
                self.inference_url = None;
                self.openai_inference = None;
            }
        }
        self
    }

    #[must_use]
    pub fn openai_inference(&self) -> Option<&OpenAiInference> {
        self.openai_inference.as_ref()
    }

    #[must_use]
    pub fn with_openai_inference(
        mut self,
        url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        self.openai_inference = Some(OpenAiInference {
            url: url.into(),
            token: token.into(),
        });
        self
    }

    #[must_use]
    pub fn document_index_url(&self) -> Option<&str> {
        self.document_index_url.as_deref()
    }

    #[must_use]
    pub fn with_document_index_url(mut self, url: impl Into<String>) -> Self {
        self.document_index_url = Some(url.into());
        self
    }

    #[must_use]
    pub fn authorization_url(&self) -> Option<&str> {
        self.authorization_url.as_deref()
    }

    #[must_use]
    pub fn with_authorization_url(mut self, url: impl Into<String>) -> Self {
        self.authorization_url = Some(url.into());
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
    pub fn as_otel_config(&self) -> OtelConfig<'_> {
        OtelConfig {
            endpoint: self.otel_endpoint.as_deref(),
            sampling_ratio: self.otel_sampling_ratio,
            log_level: self.log_level.as_str(),
        }
    }

    #[must_use]
    pub fn as_inference_config(&self) -> InferenceConfig<'_> {
        if let Some(url) = self.inference_url() {
            // Default to the Aleph Alpha one
            InferenceConfig::AlephAlpha { url }
        } else if let Some(openai) = self.openai_inference() {
            InferenceConfig::OpenAi {
                url: openai.url.as_str(),
                token: openai.token.as_str(),
            }
        } else {
            InferenceConfig::None
        }
    }

    #[must_use]
    pub fn as_tokenizers_config(&self) -> TokenizersConfig<'_> {
        if let Some(url) = self.inference_url() {
            TokenizersConfig::AlephAlpha { inference_url: url }
        } else {
            TokenizersConfig::None
        }
    }

    #[must_use]
    pub fn otel_sampling_ratio(&self) -> f64 {
        self.otel_sampling_ratio
    }

    /// # Errors
    ///
    /// Returns an error if the sampling ratio is not between 0.0 and 1.0 inclusive.
    pub fn with_otel_sampling_ratio(mut self, ratio: f64) -> anyhow::Result<Self> {
        if !(0.0..=1.0).contains(&ratio) {
            return Err(anyhow::anyhow!(
                "otel_sampling_ratio must be between 0.0 and 1.0 inclusive"
            ));
        }
        self.otel_sampling_ratio = ratio;
        Ok(self)
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

    #[must_use]
    pub fn memory_request(&self) -> Option<ByteSize> {
        self.memory_request
    }

    #[must_use]
    pub fn with_memory_request(mut self, memory_request: Option<ByteSize>) -> Self {
        self.memory_request = memory_request;
        self
    }

    #[must_use]
    pub fn memory_limit(&self) -> Option<ByteSize> {
        self.memory_limit
    }

    #[must_use]
    pub fn with_memory_limit(mut self, memory_limit: Option<ByteSize>) -> Self {
        self.memory_limit = memory_limit;
        self
    }

    #[must_use]
    pub fn wasmtime_cache_dir(&self) -> Option<&Path> {
        self.wasmtime_cache_dir.as_deref()
    }

    #[must_use]
    pub fn with_wasmtime_cache_dir(mut self, wasmtime_cache_dir: Option<PathBuf>) -> Self {
        self.wasmtime_cache_dir = wasmtime_cache_dir;
        self
    }

    #[must_use]
    pub fn wasmtime_cache_size_request(&self) -> Option<ByteSize> {
        self.wasmtime_cache_size_request
    }

    #[must_use]
    pub fn with_wasmtime_cache_size_request(
        mut self,
        wasmtime_cache_size_request: Option<ByteSize>,
    ) -> Self {
        self.wasmtime_cache_size_request = wasmtime_cache_size_request;
        self
    }

    #[must_use]
    pub fn wasmtime_cache_size_limit(&self) -> Option<ByteSize> {
        self.wasmtime_cache_size_limit
    }

    #[must_use]
    pub fn with_wasmtime_cache_size_limit(
        mut self,
        wasmtime_cache_size_limit: Option<ByteSize>,
    ) -> Self {
        self.wasmtime_cache_size_limit = wasmtime_cache_size_limit;
        self
    }

    /// We predominantly load Python skills, which are quite heavy. Python skills are roughly 60MB in size from the registry.
    /// The first skill is roughly 850MB in memory, and subsequent ones are between 100-150MB.
    /// We aim to use about 1/2 of the available memory.
    #[must_use]
    pub fn desired_skill_cache_memory_usage(&self) -> ByteSize {
        let memory_limit = self
            // Default to requested memory limit if available. Since this is what we are guaranteed by k8s, we are conservative and use it.
            .memory_request
            // Fallback to memory limit if available.
            .or(self.memory_limit)
            // Fallback to an assumption of 4GB if no limit is set (our helm chart default request)
            .unwrap_or(ByteSize::gib(4));

        ByteSize(memory_limit.as_u64() / 2)
    }

    /// We currently use around ~30MB per skill in the incremental cache.
    /// This allows for more variety within the skill code to still benefit from caching,
    /// while not consuming too much memory.
    /// 128MB seems like a fair tradeoff for the speed up we gain (50-80% for skills after firsts)
    #[must_use]
    pub fn max_incremental_cache_size(&self) -> Option<ByteSize> {
        Some(ByteSize::mib(128))
    }

    #[must_use]
    pub fn wasmtime_cache_size(&self) -> Option<ByteSize> {
        self
            // Default to requested limit if available. Since this is what we are guaranteed by k8s, we are conservative and use it.
            .wasmtime_cache_size_request
            // Fallback to limit if available.
            .or(self.wasmtime_cache_size_limit)
    }

    fn wasmtime_cache(&self) -> Option<WasmtimeCache> {
        self.wasmtime_cache_size()
            .zip(self.wasmtime_cache_dir())
            .map(|(size, dir)| WasmtimeCache::new(dir, size))
    }

    #[must_use]
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig::default()
            .use_pooling_allocator(self.use_pooling_allocator())
            .with_max_incremental_cache_size(self.max_incremental_cache_size())
            .with_wasmtime_cache(self.wasmtime_cache())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pharia_ai_feature_set: FeatureSet::default(),
            kernel_address: defaults::kernel_address(),
            metrics_address: defaults::metrics_address(),
            inference_url: None,
            openai_inference: None,
            document_index_url: None,
            authorization_url: None,
            namespaces: NamespaceConfigs::default(),
            namespace_update_interval: defaults::namespace_update_interval(),
            log_level: defaults::log_level(),
            otel_endpoint: None,
            otel_sampling_ratio: defaults::otel_sampling_ratio(),
            use_pooling_allocator: false,
            memory_request: None,
            memory_limit: None,
            wasmtime_cache_dir: None,
            wasmtime_cache_size_limit: None,
            wasmtime_cache_size_request: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, io::Write, time::Duration};

    use config::Config;
    use tempfile::tempdir;

    use crate::{
        feature_set::PRODUCTION_FEATURE_SET,
        namespace_watcher::{Namespace, Registry, tests::NamespaceConfig},
    };

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
    fn load_openai_inference() -> anyhow::Result<()> {
        // Given environment variables for openai inference
        let env_vars = HashMap::from([
            (
                "OPENAI_INFERENCE__URL".to_owned(),
                "https://openai.com".to_owned(),
            ),
            (
                "OPENAI_INFERENCE__TOKEN".to_owned(),
                "sk-1234567890".to_owned(),
            ),
        ]);
        let env_source = AppConfig::environment().source(Some(env_vars));
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path).unwrap();
        let file_source = File::with_name(file_path.to_str().unwrap());

        // When we load the config
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then the openai inference is configured
        assert!(matches!(
            config.as_inference_config(),
            InferenceConfig::OpenAi {
                url: "https://openai.com",
                token: "sk-1234567890"
            }
        ));
        Ok(())
    }

    #[test]
    fn aleph_alpha_inference_is_default() {
        // Given an app config with both, an aleph alpha inference and an openai inference configured
        let app_config = AppConfig::default()
            .with_inference_url("https://inference-api.product.pharia.com")
            .with_openai_inference("https://openai.com", "sk-1234567890");

        // When we convert it into an inference config
        let config = app_config.as_inference_config();

        // Then the aleph alpha inference is used
        assert!(matches!(
            config,
            InferenceConfig::AlephAlpha {
                url: "https://inference-api.product.pharia.com"
            }
        ));
    }

    #[test]
    fn empty_document_index_url_is_none() -> anyhow::Result<()> {
        // Given a config with an empty document index url
        let env_vars = HashMap::from([("DOCUMENT_INDEX_URL".to_owned(), String::new())]);
        let env_source = AppConfig::environment().source(Some(env_vars));
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());

        // When we load the config
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then the document index url is none
        assert!(config.document_index_url().is_none());
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
        assert_eq!(
            config.document_index_url(),
            Some("https://document-index.product.pharia.com")
        );
        assert_eq!(
            config.inference_url(),
            Some("https://inference-api.product.pharia.com")
        );
        assert_eq!(
            config.authorization_url(),
            Some("https://pharia-iam.product.pharia.com")
        );
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
        assert!(config.inference_url().is_none());
        assert!(config.document_index_url().is_none());
        assert!(config.authorization_url().is_none());
        assert_eq!(config.namespace_update_interval(), Duration::from_secs(10));
        assert_eq!(config.log_level(), "info");
        assert!(config.otel_endpoint().is_none());
        assert!((config.otel_sampling_ratio() - 0.1).abs() < 1e-6);
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

    #[test]
    fn otel_sampling_ratio_from_env() -> anyhow::Result<()> {
        // Given environment variables with a sampling ratio
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([("OTEL_SAMPLING_RATIO".to_owned(), "0.25".to_owned())]);
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When we load the config
        let config = AppConfig::from_sources(file_source, env_source)?;

        // Then the sampling ratio is set from the environment variable
        assert!((config.otel_sampling_ratio() - 0.25).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn invalid_otel_sampling_ratio_is_rejected() -> anyhow::Result<()> {
        // Given a config with an invalid sampling ratio
        let dir = tempdir()?;
        let file_path = dir.path().join("config.toml");
        fs::File::create_new(&file_path)?;
        let file_source = File::with_name(file_path.to_str().unwrap());
        let env_vars = HashMap::from([("OTEL_SAMPLING_RATIO".to_owned(), "1.1".to_owned())]);
        let env_source = AppConfig::environment().source(Some(env_vars));

        // When we load the config
        let error = AppConfig::from_sources(file_source, env_source).unwrap_err();

        // Then we receive an error
        assert!(
            error
                .to_string()
                .contains("otel_sampling_ratio must be between 0.0 and 1.0 inclusive")
        );
        Ok(())
    }
}
