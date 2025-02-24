use std::{env, str::FromStr};

use opentelemetry::{KeyValue, trace::TracerProvider};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use opentelemetry_semantic_conventions::{
    SCHEMA_URL,
    resource::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_NAME, SERVICE_VERSION},
};
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
    let env_filter = EnvFilter::from_str(&app_config.log_level)?;
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer());
    let tracer_provider = if let Some(endpoint) = &app_config.otel_endpoint {
        let tracer_provider = init_otel_tracer_provider(endpoint)?;
        let tracer = tracer_provider.tracer("tracing-otel-subscriber");
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
            [
                KeyValue::new(SERVICE_NAME, env!("CARGO_PKG_NAME")),
                KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
                KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, "develop"),
            ],
            SCHEMA_URL,
        )
        .build()
}
