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
    Context, KeyValue, TraceId,
    propagation::TextMapCompositePropagator,
    trace::{TraceContextExt, TracerProvider},
};
use opentelemetry_otlp::{SpanExporter as OtlpSpanExporter, WithExportConfig, WithHttpConfig};
use std::collections::HashMap;
use opentelemetry_sdk::{
    error::OTelSdkError, propagation::{BaggagePropagator, TraceContextPropagator}, trace::{RandomIdGenerator, Sampler, SdkTracerProvider, SpanData, SpanExporter as SpanExporterTrait, SpanProcessor}, Resource
};
use opentelemetry_semantic_conventions::{SCHEMA_URL, resource::SERVICE_VERSION};
use tracing::{Level, Span, error, info};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{
    EnvFilter,
    fmt::time::{FormatTime, UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::config::OtelConfig;

use std::fmt;
use tracing_core::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{
        FmtContext, FormatEvent,
        format::{FormatFields, Writer},
    },
    registry::LookupSpan,
};

/// An event formatter that renders event log messages without including span context.
///
/// Before, our logs were getting cluttered with http attributes coming from the axum middleware:
///
/// `2025-07-15T14:52:44.928603Z INFO HTTP request:skill_execution: pharia-kernel::skill-execution: skill=test-beta/hello Skill executed successfully http.request.method=POST network.protocol.version=1.1 server.address="127.0.0.1:8081" user_agent.original="PostmanRuntime/7.44.1"`
///
/// However, for our logs, we are mostly interested in the message itself. This formatter leads to
/// logs like:
///
/// `2025-07-15T14:52:44.928603Z pharia-kernel::skill-execution: skill=test-beta/hello Skill executed successfully`
struct NoSpan;

impl<S, N> FormatEvent<S, N> for NoSpan
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut w: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        // Grey color for time
        write!(w, "\x1b[90m")?;
        UtcTime::rfc_3339().format_time(&mut w)?;
        write!(w, "\x1b[0m")?;

        let level = *event.metadata().level();
        let (open, close) = ansi_for_level(level);
        write!(w, " {open}{level}{close}")?;

        // Grey color for target (module path)
        write!(w, " \x1b[90m{}\x1b[0m: ", event.metadata().target())?;
        ctx.field_format().format_fields(w.by_ref(), event)?;
        writeln!(w)?;
        Ok(())
    }
}

fn ansi_for_level(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::TRACE => ("\x1b[34m", "\x1b[0m"),   // blue
        Level::DEBUG => ("\x1b[36m", "\x1b[0m"),   // cyan
        Level::INFO => ("\x1b[32m", "\x1b[0m"),    // green
        Level::WARN => ("\x1b[33m", "\x1b[0m"),    // yellow
        Level::ERROR => ("\x1b[1;31m", "\x1b[0m"), // bold red
    }
}

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
                            Err(e) => {
                                error!(parent: self.span(), "Found invalid header value: {e:#}");
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(parent: self.span(), "Found invalid header value: {e:#}");
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


/// A span processor that filters spans based on whether they have GenAI attributes.
#[derive(Debug)]
pub struct GenAiSpanProcessor {
    inner_processor: Box<dyn SpanProcessor>,
    include_genai: bool,
}

impl GenAiSpanProcessor {
    /// Create a new filtered span processor.
    /// 
    /// # Arguments
    /// * `exporter` - The span exporter to send filtered spans to
    /// * `include_genai` - If true, only spans with genai.* attributes are processed.
    ///                     If false, spans with genai.* attributes are filtered out.
    pub fn new(exporter: OtlpSpanExporter, include_genai: bool) -> Self {
        let inner_processor = opentelemetry_sdk::trace::BatchSpanProcessor::builder(exporter).build();
        Self {
            inner_processor: Box::new(inner_processor),
            include_genai,
        }
    }

    /// Create a new filtered span processor with any test exporter.
    /// 
    /// # Arguments
    /// * `exporter` - The test exporter to send filtered spans to
    /// * `include_genai` - If true, only spans with genai.* attributes are processed.
    ///                     If false, spans with genai.* attributes are filtered out.
    #[cfg(test)]
    fn new_with_test_exporter(exporter: impl SpanExporterTrait + 'static, include_genai: bool) -> Self {
        let inner_processor = opentelemetry_sdk::trace::BatchSpanProcessor::builder(exporter).build();
        Self {
            inner_processor: Box::new(inner_processor),
            include_genai,
        }
    }
    
    /// Check if a span has any GenAI-related attributes.
    fn has_genai_attributes(span_data: &SpanData) -> bool {
        span_data.attributes.iter().any(|kv| {
            kv.key.as_str().starts_with("genai.")
        })
    }
    
    /// Determine if a span should be processed based on the filter criteria.
    fn should_process_span(&self, span_data: &SpanData) -> bool {
        let has_genai = Self::has_genai_attributes(span_data);
        if self.include_genai {
            // GenAI processor: only include spans with genai.* attributes
            has_genai
        } else {
            // Main processor: exclude spans with genai.* attributes
            !has_genai
        }
    }
}

impl SpanProcessor for GenAiSpanProcessor {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &Context) {
        self.inner_processor.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        if self.should_process_span(&span) {
            self.inner_processor.on_end(span);
        }
        // If the span doesn't match our filter criteria, we simply drop it
    }

    fn force_flush(&self) -> Result<(), OTelSdkError> {
        self.inner_processor.force_flush().map_err(|e| opentelemetry_sdk::error::OTelSdkError::from(e))
    }

    fn shutdown(&self) -> Result<(), OTelSdkError> {
        self.inner_processor.shutdown()
    }

    fn shutdown_with_timeout(&self, timeout: std::time::Duration) -> Result<(), OTelSdkError> {
        self.inner_processor.shutdown_with_timeout(timeout)
    }
}

/// Set up two tracing subscribers:
/// * Simple env logger
/// * OpenTelemetry
///
/// # Errors
/// Failed to parse the log level provided by the configuration.
pub fn initialize_tracing(otel_config: OtelConfig<'_>) -> anyhow::Result<OtelGuard> {
    // We write the errors to stderr as logging/tracing is not yet initialized
    let env_filter = EnvFilter::from_str(otel_config.log_level).inspect_err(|e| {
        eprintln!("Error parsing log level: {e:#}");
    })?;
    let fmt_layer = tracing_subscriber::fmt::layer().event_format(NoSpan);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);
    
    let tracer_provider = {
        let mut endpoint_descriptions = Vec::new();
        
        // Build main exporter if endpoint is configured
        let main_exporter = if let Some(endpoint) = otel_config.endpoint {
            let exporter = OtlpSpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .inspect_err(|e| {
                    eprintln!("Error building main span exporter: {e:#}");
                })?;
            endpoint_descriptions.push(format!("main endpoint: {}", endpoint));
            Some(exporter)
        } else {
            None
        };
        
        // Build genai exporter if genai_endpoint is configured
        let genai_exporter = if let Some(genai_endpoint) = otel_config.genai_endpoint {
            let exporter = OtlpSpanExporter::builder()
                // Studio only supports HTTP/proto, so we can not use tonic here
                .with_http()
                .with_headers(HashMap::from([("Authorization".to_string(), format!("Bearer {}", otel_config.genai_endpoint_api_key.unwrap_or("")))]))
                .with_endpoint(genai_endpoint)
                .build()
                .inspect_err(|e| {
                    eprintln!("Error building genai span exporter: {e:#}");
                })?;
            endpoint_descriptions.push(format!("genai endpoint: {}", genai_endpoint));
            Some(exporter)
        } else {
            None
        };
        
        if main_exporter.is_none() && genai_exporter.is_none() {
            registry.init();
            None
        } else {
            // Use the filtered tracer provider that separates GenAI and non-GenAI spans
            let tracer_provider = tracer_provider_with_filtered_exporters(
                main_exporter,
                genai_exporter,
                otel_config.sampling_ratio
            )?;
            init_propagator();

            // Sets otel.scope.name, a logical unit within the application code, see https://opentelemetry.io/docs/concepts/instrumentation-scope/
            let tracer = tracer_provider.tracer("pharia-kernel");
            let layer = tracing_opentelemetry::layer().with_tracer(tracer);
            registry.with(layer).init();
            
            let endpoints_str = endpoint_descriptions.join(" and ");
            info!(
                target: "pharia-kernel::logging",
                "Initialized OpenTelemetry tracer provider with {}",
                endpoints_str
            );
            Some(tracer_provider)
        }
    };

    Ok(OtelGuard { tracer_provider })
}

pub struct OtelGuard {
    tracer_provider: Option<SdkTracerProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(tracer_provider) = &mut self.tracer_provider
            && let Err(err) = tracer_provider.shutdown()
        {
            eprintln!("{err:?}");
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

/// Construct an opentelemetry tracer provider with filtered exporters.
///
/// This method creates filtered span processors:
/// - Main exporter: excludes spans with genai.* attributes
/// - GenAI exporter: includes only spans with genai.* attributes
///
/// # Arguments
/// * `main_exporter` - Optional exporter for non-GenAI spans
/// * `genai_exporter` - Optional exporter for GenAI spans only
/// * `sampling_ratio` - Sampling ratio for the tracer provider
///
/// # Errors
/// Failed to build the tracer provider.
pub fn tracer_provider_with_filtered_exporters(
    main_exporter: Option<OtlpSpanExporter>,
    genai_exporter: Option<OtlpSpanExporter>,
    sampling_ratio: f64,
) -> anyhow::Result<SdkTracerProvider> {
    let mut builder = SdkTracerProvider::builder();
    builder = builder
    // We respect the parent span's sampling decision, even if it is coming from
    // a remote service.
    .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
        sampling_ratio,
    ))))
    .with_id_generator(RandomIdGenerator::default())
    .with_resource(resource());

    // Add main processor that excludes GenAI spans
    if let Some(exporter) = main_exporter {
        let processor = GenAiSpanProcessor::new(exporter, false);
        builder = builder.with_span_processor(processor);
    }

    // Add GenAI processor that includes only GenAI spans
    if let Some(exporter) = genai_exporter {
        let processor = GenAiSpanProcessor::new(exporter, true);
        builder = builder.with_span_processor(processor);
    }

    Ok(builder.build())
}


/// Construct an opentelemetry tracer provider with a single exporter.
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


    /// Test span exporter that captures exported spans for inspection
    #[derive(Debug, Clone)]
    struct TestSpanExporter {
        spans: std::sync::Arc<std::sync::Mutex<Vec<SpanData>>>,
        name: String,
    }

    impl TestSpanExporter {
        fn new(name: &str) -> Self {
            Self {
                spans: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                name: name.to_string(),
            }
        }

        fn exported_spans(&self) -> Vec<SpanData> {
            self.spans.lock().unwrap().clone()
        }

        #[allow(dead_code)]
        fn clear(&self) {
            self.spans.lock().unwrap().clear();
        }
    }

    impl SpanExporterTrait for TestSpanExporter {
        async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
            println!("TestSpanExporter '{}' received {} spans", self.name, batch.len());
            for span in &batch {
                println!("  - Span '{}' with {} attributes", span.name, span.attributes.len());
                for attr in &span.attributes {
                    println!("    - {}: {:?}", attr.key, attr.value);
                }
            }
            self.spans.lock().unwrap().extend(batch);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_filtered_exporters_with_real_spans() {
        use opentelemetry::trace::{TracerProvider, Tracer, Span as TraceSpan};
        use opentelemetry::{KeyValue, Value};

        // Create test exporters that capture spans
        let main_exporter = TestSpanExporter::new("main");
        let genai_exporter = TestSpanExporter::new("genai");

        // Create a tracer provider with filtered exporters using our test exporters
        let tracer_provider = {
            let mut builder = SdkTracerProvider::builder();

            // Add main processor that excludes GenAI spans
            let main_processor = GenAiSpanProcessor::new_with_test_exporter(main_exporter.clone(), false);
            builder = builder.with_span_processor(main_processor);

            // Add GenAI processor that includes only GenAI spans
            let genai_processor = GenAiSpanProcessor::new_with_test_exporter(genai_exporter.clone(), true);
            builder = builder.with_span_processor(genai_processor);

            builder
                .with_sampler(Sampler::AlwaysOn) // Always sample for testing
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(resource())
                .build()
        };

        let tracer = tracer_provider.tracer("test");

        // Create span with GenAI attributes
        {
            let mut span = tracer
                .span_builder("genai_inference")
                .with_attributes(vec![
                    KeyValue::new("genai.model", Value::String("gpt-4".into())),
                    KeyValue::new("genai.request_id", Value::String("req-123".into())),
                    KeyValue::new("http.method", Value::String("POST".into())),
                ])
                .start(&tracer);
            span.end();
        }

        // Create span without GenAI attributes (regular span)
        {
            let mut span = tracer
                .span_builder("http_request")
                .with_attributes(vec![
                    KeyValue::new("http.method", Value::String("GET".into())),
                    KeyValue::new("http.status_code", Value::I64(200)),
                ])
                .start(&tracer);
            span.end();
        }

        // Create span with mixed attributes (has GenAI attributes)
        {
            let mut span = tracer
                .span_builder("mixed_span")
                .with_attributes(vec![
                    KeyValue::new("genai.usage.input_tokens", Value::I64(100)),
                    KeyValue::new("http.method", Value::String("POST".into())),
                    KeyValue::new("user.id", Value::String("user123".into())),
                ])
                .start(&tracer);
            span.end();
        }

        // Force flush to ensure all spans are exported
        tracer_provider.force_flush().unwrap();

        // Wait a bit for async processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Verify spans were routed correctly
        let main_spans = main_exporter.exported_spans();
        let genai_spans = genai_exporter.exported_spans();

        println!("Main exporter received {} spans", main_spans.len());
        println!("GenAI exporter received {} spans", genai_spans.len());

        // Verify service name is correctly set on all spans
        let resource = resource();
        let service_name_key = opentelemetry::Key::from_static_str("service.name");
        let service_name_value = resource.get(&service_name_key).unwrap();
        let expected_service_name = service_name_value.as_str();
        println!("Expected service name from resource: {}", expected_service_name);
        assert_eq!(expected_service_name, "pharia-kernel", "Resource should have service name set to 'pharia-kernel'");

        // Check service name on all spans
        for span in &main_spans {
            println!("Main span '{}' instrumentation scope: {}", span.name, span.instrumentation_scope.name());
            // Note: service.name is part of the resource, not individual span attributes
            // But we can verify it's available through the tracer provider's resource
        }
        
        for span in &genai_spans {
            println!("GenAI span '{}' instrumentation scope: {}", span.name, span.instrumentation_scope.name());
        }

        // Main exporter should have only the regular span (no GenAI attributes)
        assert_eq!(main_spans.len(), 1, "Main exporter should receive 1 span");
        assert_eq!(main_spans[0].name, "http_request");
        assert!(!GenAiSpanProcessor::has_genai_attributes(&main_spans[0]), 
               "Main exporter should not receive spans with GenAI attributes");

        // GenAI exporter should have 2 spans (genai_inference and mixed_span)
        assert_eq!(genai_spans.len(), 2, "GenAI exporter should receive 2 spans");
        
        let genai_span_names: Vec<_> = genai_spans.iter().map(|s| s.name.as_ref()).collect();
        assert!(genai_span_names.contains(&"genai_inference"), "GenAI exporter should receive genai_inference span");
        assert!(genai_span_names.contains(&"mixed_span"), "GenAI exporter should receive mixed_span");

        for span in &genai_spans {
            assert!(GenAiSpanProcessor::has_genai_attributes(span), 
                   "GenAI exporter should only receive spans with GenAI attributes");
        }
    }
}
