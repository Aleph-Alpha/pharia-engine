//! Logging and tracing utilities.
//!
//! This module encapsulates our decisions on how we want to do logging and tracing in the Kernel.
//! Since the Kernel is written in an actor model, certain assumptions of the [`tracing`] crate
//! do not hold. For example, it relies on thread local storage to store the current active span.
//! All [`tracing::event!`] macro calls will then use this thread local storage to situate an event
//! in the right span. This still works for async code, where then some middleware will make sure
//! to set the corresponding span as active whenever polling the inner future.
//!
//! However, in an actor model, we loose the acountability. Imagine different actors running in
//! different threads, and reacting to messages. If we want them to situate events in the right
//! context, we need to pass that context along with the messages, similar to how tracing context
//! is passed along as part of requests in distributed systems.
use std::{env, str::FromStr};

use axum::http::{HeaderMap, HeaderValue};
use opentelemetry::{
    KeyValue, TraceId,
    propagation::TextMapCompositePropagator,
    trace::{TraceContextExt, TracerProvider},
};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::SpanExporter as SpanExporterTrait;
use opentelemetry_sdk::{
    Resource,
    propagation::{BaggagePropagator, TraceContextPropagator},
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use opentelemetry_semantic_conventions::{SCHEMA_URL, resource::SERVICE_VERSION};
use tracing::{Span, error, info};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::OtelConfig;

/// Create new child context.
///
/// This macro creates a new child span and returns a new `TracingContext` that
/// is associated with the new span.
#[macro_export]
macro_rules! context {
    ($parent:expr, $target:expr, $($field:tt)+) => {
        {
            use tracing::Level;
            let parent_span = $parent.span();
            let new_span = tracing::span!(target: $target, parent: parent_span, Level::INFO, $($field)*);
            TracingContext::new(new_span)
        }
    };
}
/// Context that is needed to situate certain actions in the overall context.
///
/// In this opaque type we specify decisions on what context needs to be passed
/// within actors to correlate actions that belong together.
/// While we originally wanted to pass span and trace ids across actors, we
/// found that this is not possible in a safe way, as the span might be dropped
/// in the meantime. Therefore, we now pass the span itself, assuring it is not
/// dropped while someone is creating a child span.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct TracingContext(Span);

impl TracingContext {
    /// Retrieve the current thread-local tracing context.
    ///
    /// This method MUST ONLY be invoked in an axum handler, and not in a different actor.
    /// We know that the `AxumOtelLayer` middleware makes sure to create a span, provide the
    /// opentelemetry context, and enter the span when polling its inner future (other middleware
    /// or the handlers).
    pub fn current() -> Self {
        Self(tracing::Span::current())
    }

    /// Create a new tracing context.
    ///
    /// This method would be invoked if the caller has created a new span, and now
    /// wants to create a new trace context that is associated with that span.
    pub fn new(span: Span) -> Self {
        Self(span)
    }

    /// Get the inner span object.
    ///
    /// Most likely use case is the caller wanting to specify the parent for a new span
    /// or event.
    pub fn span(&self) -> &Span {
        &self.0
    }

    /// Standard HTTP headers to propagate context information and enable distributed tracing.
    ///
    /// <https://www.w3.org/TR/trace-context-2/#http-headers>
    ///
    /// In case we end up with an invalid header value, we do not return a result, but rather
    /// take the decision for the caller and return an empty header map. This comes down to
    /// the fact that we do not want to allow tracing information to alter the execution flow.
    pub fn w3c_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(traceparent) = self.traceparent_header() {
            match HeaderValue::from_str(&traceparent) {
                Ok(value) => {
                    headers.insert("traceparent", value);

                    // The tracestate header is a companion header to the traceparent header.
                    if let Some(tracestate) = self.tracestate_header() {
                        match HeaderValue::from_str(&tracestate) {
                            Ok(value) => {
                                headers.insert("tracestate", value);
                            }
                            Err(err) => {
                                error!(parent: self.span(), "Found invalid header value: {err}");
                            }
                        }
                    }
                }
                Err(err) => {
                    error!(parent: self.span(), "Found invalid header value: {err}");
                }
            }
        }
        headers
    }

    /// Is the current span sampled?
    ///
    /// The open telemetry span context holds information whether the current span is sampled.
    /// By respecting the parent's span decision for sampling, we allow remote services to decide
    /// whether a span is sampled or not. If no information is provided, our probability-based
    /// sampler will decide whether to sample the span or not.
    fn sampled(&self) -> bool {
        self.0.context().span().span_context().is_sampled()
    }

    /// Render the context as a traceparent header.
    ///
    /// <https://www.w3.org/TR/trace-context-2/#traceparent-header>
    fn traceparent_header(&self) -> Option<String> {
        self.0.id().map(|span_id| {
            Self::format_traceparent_header(
                span_id.into_u64(),
                self.trace_id_u128(),
                self.sampled(),
            )
        })
    }

    /// The version of the trace context specification that we support.
    ///
    /// <https://www.w3.org/TR/trace-context-2/#version>
    const SUPPORTED_VERSION: u8 = 0;

    /// Construct a traceparent header from a span id and trace id.
    fn format_traceparent_header(span_id: u64, trace_id: u128, sampled: bool) -> String {
        format!(
            "{:02x}-{:032x}-{:016x}-{:02x}",
            Self::SUPPORTED_VERSION,
            trace_id,
            span_id,
            u8::from(sampled),
        )
    }

    /// Render the provided trace state as a header.
    ///
    /// Trace state includes additional, vendor-specific key value pairs.
    /// The header will be empty if no data is provided.
    /// <https://www.w3.org/TR/trace-context/#tracestate-header>
    fn tracestate_header(&self) -> Option<String> {
        let header = self
            .0
            .context()
            .span()
            .span_context()
            .trace_state()
            .header();

        // Vendors MUST accept empty tracestate headers but SHOULD avoid sending them.
        if header.is_empty() {
            None
        } else {
            Some(header)
        }
    }

    /// Convert the tracing context to what the inference client expects.
    pub fn as_inference_client_context(&self) -> Option<aleph_alpha_client::TraceContext> {
        self.0.id().map(|id| {
            aleph_alpha_client::TraceContext::new(
                self.trace_id_u128(),
                id.into_u64(),
                self.sampled(),
                self.tracestate_header(),
            )
        })
    }

    /// Get the inner u128 of the trace id.
    ///
    /// `TraceId` is a new type around u128. While it would be nice to access the inner value
    /// directly, it is private, so we need to convert to bytes first.
    fn trace_id_u128(&self) -> u128 {
        u128::from_be_bytes(self.trace_id().to_bytes())
    }

    #[allow(dead_code)]
    pub fn trace_id(&self) -> TraceId {
        self.0.context().span().span_context().trace_id()
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
        let exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()?;
        let tracer_provider = tracer_provider(exporter, otel_config.sampling_ratio)?;
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

/// Construct an opentelemetry tracer provider.
///
/// This method is public so we can use the same provider and sampler for out integration tests.
///
/// # Errors
/// Failed to build the tracer provider.
pub fn tracer_provider(
    exporter: impl SpanExporterTrait + 'static,
    sampling_ratio: f64,
) -> anyhow::Result<SdkTracerProvider> {
    Ok(SdkTracerProvider::builder()
        // We respect the parent span's sampling decision, even if it is coming from
        // a remote service.
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            sampling_ratio,
        ))))
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource())
        .with_batch_exporter(exporter)
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
    use opentelemetry::trace::TraceState;
    use tracing::{Level, span};

    use super::*;
    use std::sync::LazyLock;

    impl TracingContext {
        pub fn dummy() -> Self {
            Self(Span::none())
        }
    }

    static INITIALIZE_TRACING_SUBSCRIBER: LazyLock<SdkTracerProvider> =
        LazyLock::new(tracing_subscriber);

    /// Ensure a tracing subscriber is initialized.
    ///
    /// This is useful for tests that are testing logging/tracing related functionality.
    /// While we would like to use [`tracing::subscriber::with_default`] to set the subscriber for
    /// the scope of the test, this only applies to a single thread, and proved to be flaky as
    /// tokio may choose to run certain actors on a different thread.
    pub fn given_tracing_subscriber() -> &'static SdkTracerProvider {
        &INITIALIZE_TRACING_SUBSCRIBER
    }

    /// Construct a subscriber that logs to stdout and allows to retrieve the traceparent
    fn tracing_subscriber() -> SdkTracerProvider {
        // This matches the setup in `logging`, but exporting to stdout instead of an OTLP endpoint
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
            .with_id_generator(RandomIdGenerator::default())
            .with_resource(resource())
            .with_batch_exporter(opentelemetry_stdout::SpanExporter::default())
            .build();

        // Allows to retrieve the traceparent
        init_propagator();

        let tracer = provider.tracer("test");
        let layer = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(EnvFilter::from_str("info").unwrap())
            .with(layer)
            .init();
        provider
    }

    #[test]
    fn w3c_headers_are_empty_if_no_trace_context_is_present() {
        let context = TracingContext::dummy();
        let headers = context.w3c_headers();
        assert!(headers.is_empty());
    }

    #[test]
    fn tracestate_is_not_included_if_none() {
        let subscriber = tracing_subscriber::registry();

        tracing::subscriber::with_default(subscriber, || {
            // Given a trace context with no specific trace state set
            let span = span!(Level::INFO, "test");
            let context = TracingContext::new(span);

            // When we render the W3C headers
            let headers = context.w3c_headers();

            // Then the traceparent is present, but the tracestate is not
            assert!(headers.get("traceparent").is_some());
            assert!(headers.get("tracestate").is_none());
        });
    }

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn traceparent_rendering_of_sampled_span() {
        let trace_id = 0x4bf92f3577b34da6a3ce929d0e0e4736;
        let span_id = 0x00f067aa0ba902b7;
        let sampled = true;
        assert_eq!(
            TracingContext::format_traceparent_header(span_id, trace_id, sampled),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
    }

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn traceparent_rendering_of_unsampled_span() {
        let trace_id = 0x4bf92f3577b34da6a3ce929d0e0e4736;
        let span_id = 0x00f067aa0ba902b7;
        let sampled = false;
        assert_eq!(
            TracingContext::format_traceparent_header(span_id, trace_id, sampled),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"
        );
    }

    /// This is a learning test around the tracing crate.
    ///
    /// In our actor model, we originally intended to send span id across messages to
    /// different actors, such that other actors could reference a parent span.
    /// However, we then can not guarantee that the parent span has not been dropped in the
    /// sending actor, so we risk that the subscriber panics.
    #[test]
    #[should_panic(expected = "tried to clone")]
    fn parent_span_needs_to_be_in_scope_when_creating_child_span() {
        let subscriber = tracing_subscriber::registry();

        tracing::subscriber::with_default(subscriber, || {
            // When we create a span and drop it
            let span = span!(Level::INFO, "test");
            let parent_id = span.id().unwrap();
            drop(span);

            // Then referring to the dropped span as parent will lead to a panic
            span!(parent: parent_id, Level::INFO, "child");
        });
    }

    #[test]
    fn empty_tracestate() {
        // Given a context containing a span with an empty tracestate
        let context = TracingContext::dummy();

        // When
        let tracestate = context.tracestate_header();

        // Then
        assert!(tracestate.is_none());
    }

    #[test]
    fn tracestate_header_includes_attributes() {
        let trace_state = TraceState::default().insert("foo", "bar").unwrap();

        let header = trace_state.header();
        assert_eq!(header, "foo=bar");
    }

    #[test]
    fn span_with_always_on_sampler_knows_it_is_sampled() {
        // Given a subscriber that is interested in all spans
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();

        let layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            // When creating a new span
            let span = span!(Level::INFO, "foo");
            let context = TracingContext::new(span);

            // Then the span says that it is sampled
            assert!(context.sampled());
        });
    }

    #[test]
    fn span_with_always_off_sampler_knows_it_is_not_sampled() {
        // Given a subscriber that is interested in no spans
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOff)
            .build();

        let layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            // When creating a new span
            let span = span!(Level::INFO, "foo");
            let context = TracingContext::new(span);

            // Then the span says that it is sampled
            assert!(!context.sampled());
        });
    }
}
