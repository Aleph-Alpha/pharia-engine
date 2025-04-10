use std::{env, str::FromStr};

use opentelemetry::{KeyValue, trace::TracerProvider};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use opentelemetry_semantic_conventions::{SCHEMA_URL, resource::SERVICE_VERSION};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::AppConfig;

/// Set up two tracing subscribers:
/// * Simple env logger
/// * OpenTelemetry
///
/// # Errors
/// Failed to parse the log level provided by the configuration.
pub fn initialize_tracing(app_config: &AppConfig) -> anyhow::Result<OtelGuard> {
    let env_filter = EnvFilter::from_str(app_config.log_level())?;
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer());
    let tracer_provider = if let Some(endpoint) = app_config.otel_endpoint() {
        let tracer_provider = init_otel_tracer_provider(endpoint)?;
        // Sets otel.scope.name, a logical unit within the application code, see https://opentelemetry.io/docs/concepts/instrumentation-scope/
        let tracer = tracer_provider.tracer("pharia-kernel");
        registry.with(OpenTelemetryLayer::new(tracer)).init();
        Some(tracer_provider)
    } else {
        registry.init();
        None
    };

    Ok(OtelGuard { tracer_provider })
}

pub struct OtelGuard {
    tracer_provider: Option<SdkTracerProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(tracer_provider) = &mut self.tracer_provider {
            if let Err(err) = tracer_provider.shutdown() {
                eprintln!("{err:?}");
            }
        }
    }
}

fn init_otel_tracer_provider(endpoint: &str) -> anyhow::Result<SdkTracerProvider> {
    Ok(SdkTracerProvider::builder() // Customize sampling strategy
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            0.1,
        ))))
        // If export trace to AWS X-Ray, you can use XrayIdGenerator
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource())
        .with_batch_exporter(
            SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
        )
        .build())
}

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource() -> Resource {
    Resource::builder()
        .with_schema_url(
            [KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION"))],
            SCHEMA_URL,
        )
        // When calling [Resource::builder], the resource name get's read from the env variable `OTEL_SERVICE_NAME`.
        // We don't set this, and per default the service name of the resource inside Resource::builder is then set to `unknown_service`.
        // When providing a `SERVICE_NAME` as part of the attributes to `with_schema_url`, these attributes get merged with the existing
        // resource inside the builder. As the service name is already set to `unknown_service`, the newly provided service name will be ignored.
        // We therefore need to explicitly set the service name by using `with_service_name`.
        .with_service_name(env!("CARGO_PKG_NAME"))
        .build()
}
