use std::{env, str::FromStr};

use opentelemetry::{
    KeyValue, TraceId,
    propagation::TextMapCompositePropagator,
    trace::{TraceContextExt, TracerProvider},
};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    propagation::{BaggagePropagator, TraceContextPropagator},
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use opentelemetry_semantic_conventions::{SCHEMA_URL, resource::SERVICE_VERSION};
use tracing::{info, span::Id};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::OtelConfig;

#[macro_export]
macro_rules! context_event {
    // If target is provided, it must be specified before the parent.
    (context: $tracing_context:expr, level: $lvl:expr, target: $target:literal, $($fields:tt)*) => {
        use tracing::{Level, info, warn, error};

        if let Some(span_id) = $tracing_context.span_id() {
            match $lvl {
                Level::INFO => info!(target: $target, parent: span_id, $($fields)*),
                Level::WARN => warn!(target: $target, parent: span_id, $($fields)*),
                Level::ERROR => error!(target: $target, parent: span_id, $($fields)*),
                _ => (
                    panic!("Do not use this macro for debugging, as the logs are intended for the operator.")
                )
            }
        }
    };
    (context: $tracing_context:expr, level: $lvl:expr, $($fields:tt)*) => {
        use tracing::{Level, info, warn, error};

        if let Some(span_id) = $tracing_context.span_id() {
            match $lvl {
                Level::INFO => info!(parent: span_id, $($fields)*),
                Level::WARN => warn!(parent: span_id, $($fields)*),
                Level::ERROR => error!(parent: span_id, $($fields)*),
                _ => (
                    panic!("Do not use this macro for debugging, as the logs are intended for the operator.")
                )
            }
        }
    };
}

/// Context that is needed to situate certain actions in the overall context.
///
/// In this opaque type we specify decisions on what context needs to be passed
/// within actors to correlate actions that belong together.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct TracingContext {
    /// This is the id from the `tracing` ecosystem. Within each trace there is a hierarchy
    /// of span. the `span_id` allows to situate spans in this hierarchy.
    ///
    /// If the subscriber indicates that it does not track the current span, or
    /// that the thread from which this function is called is not currently
    /// inside a span, we will have a `None` here.
    span_id: Option<Id>,
    /// The opentelemetry trace id. We store this separately from the span id, because
    /// it seems very hard to lookup a trace id for a span id without relying on
    /// some current span thread-local magic. And we can only use this magic in the handler,
    /// not in other actors. So providing the trace id allows other actors to include them
    /// in outgoing requests.
    trace_id: TraceId,
}

impl TracingContext {
    /// Retrieve the current thread-local tracing context.
    ///
    /// This method MUST ONLY be invoked in an axum handler, and not in a different actor.
    /// We know that the `AxumOtelLayer` middleware makes sure to create a span, provide the
    /// opentelemetry context, and enter the span when polling its inner future (other middleware
    /// or the handlers).
    pub fn current() -> Self {
        let span = tracing::Span::current();
        let trace_id = span.context().span().span_context().trace_id();
        Self {
            span_id: span.id(),
            trace_id,
        }
    }

    pub fn span_id(&self) -> Option<&Id> {
        self.span_id.as_ref()
    }

    #[allow(dead_code)]
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Convert the tracing context to what the inference client expects.
    pub fn as_inference_client_context(&self) -> Option<aleph_alpha_client::TraceContext> {
        self.span_id().map(|id| {
            aleph_alpha_client::TraceContext::new_sampled(self.trace_id_u128(), id.into_u64())
        })
    }

    /// Get the inner u128 of the trace id.
    ///
    /// `TraceId` is a new type around u128. While it would be nice to access the inner value
    /// directly, it is private, so we need to convert to bytes first.
    fn trace_id_u128(&self) -> u128 {
        u128::from_be_bytes(self.trace_id().to_bytes())
    }
}

/// Set up two tracing subscribers:
/// * Simple env logger
/// * OpenTelemetry
///
/// # Errors
/// Failed to parse the log level provided by the configuration.
pub fn initialize_tracing(otel_config: OtelConfig<'_>) -> anyhow::Result<OtelGuard> {
    let env_filter = EnvFilter::from_str(otel_config.log_level)?;
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer());
    let tracer_provider = if let Some(endpoint) = otel_config.endpoint {
        let tracer_provider = init_otel_tracer_provider(endpoint, otel_config.sampling_ratio)?;
        init_propagator();

        // Sets otel.scope.name, a logical unit within the application code, see https://opentelemetry.io/docs/concepts/instrumentation-scope/
        let tracer = tracer_provider.tracer("pharia-kernel");
        let layer = tracing_opentelemetry::layer().with_tracer(tracer);
        registry.with(layer).init();
        info!(
            "Initialized OpenTelemetry tracer provider with endpoint: {}",
            endpoint
        );
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

/// Set propagators that extract traceparent and tracestate from incoming requests.
/// Setting these (globally) is required for the `axum_tracing_opentelemetry` middleware to work.
pub fn init_propagator() {
    let context_propagator = TraceContextPropagator::new();
    let baggage_propagator = BaggagePropagator::new();

    let propagator = TextMapCompositePropagator::new(vec![
        Box::new(context_propagator),
        Box::new(baggage_propagator),
    ]);
    opentelemetry::global::set_text_map_propagator(propagator);
}

fn init_otel_tracer_provider(
    endpoint: &str,
    sampling_ratio: f64,
) -> anyhow::Result<SdkTracerProvider> {
    Ok(SdkTracerProvider::builder() // Customize sampling strategy
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            sampling_ratio,
        ))))
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
pub fn resource() -> Resource {
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
        .with_service_name("pharia-kernel")
        .build()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    impl TracingContext {
        pub fn dummy() -> Self {
            Self {
                span_id: None,
                trace_id: TraceId::INVALID,
            }
        }
    }
}
