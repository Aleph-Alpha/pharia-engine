use std::{env, str::FromStr};

use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    runtime,
    trace::{RandomIdGenerator, Sampler, Tracer},
    Resource,
};
use opentelemetry_semantic_conventions::{
    resource::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_NAME, SERVICE_VERSION},
    SCHEMA_URL,
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::AppConfig;

/// Set up two tracing subscribers:
/// * Simple env logger
/// * OpenTelemetry
///
/// # Errors
/// Failed to parse the log level provided by the configuration.
pub fn initialize_tracing(app_config: &AppConfig) -> anyhow::Result<()> {
    let env_filter = EnvFilter::from_str(&app_config.log_level)?;
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer());
    if let Some(endpoint) = &app_config.open_telemetry_endpoint {
        let otel_tracer = init_otel_tracer(endpoint)?;
        registry.with(OpenTelemetryLayer::new(otel_tracer)).init();
    } else {
        registry.init();
    }
    Ok(())
}

fn init_otel_tracer(endpoint: &str) -> anyhow::Result<Tracer> {
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_config(
            opentelemetry_sdk::trace::Config::default()
                // Customize sampling strategy
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
                    1.0,
                ))))
                // If export trace to AWS X-Ray, you can use XrayIdGenerator
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(resource()),
        )
        .with_batch_exporter(
            SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
            runtime::Tokio,
        )
        .build();

    global::set_tracer_provider(provider.clone());
    Ok(provider.tracer("tracing-otel-subscriber"))
}

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource() -> Resource {
    Resource::from_schema_url(
        [
            KeyValue::new(SERVICE_NAME, env!("CARGO_PKG_NAME")),
            KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, "develop"),
        ],
        SCHEMA_URL,
    )
}
