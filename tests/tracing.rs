//! Utilities for testing logging/tracing related functionality.
use std::{
    str::FromStr,
    sync::{Arc, LazyLock, Mutex},
};

use opentelemetry::{propagation::TextMapCompositePropagator, trace::TracerProvider};
use opentelemetry_sdk::{
    propagation::{BaggagePropagator, TraceContextPropagator},
    trace::{SdkTracerProvider, SpanData, SpanExporter},
};
use pharia_engine::tracer_provider;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// A span exporter that allows to inspect spans that have been recorded.
#[derive(Clone, Debug)]
pub struct SpySpanExporter {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl SpySpanExporter {
    pub fn new(spans: Arc<Mutex<Vec<SpanData>>>) -> Self {
        Self { spans }
    }
}

impl SpanExporter for SpySpanExporter {
    async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
        self.spans.lock().unwrap().extend(batch);
        Ok(())
    }
}

pub struct LogRecorder {
    spans: Arc<Mutex<Vec<SpanData>>>,
    // Ensure the provider is not dropped
    guard: SdkTracerProvider,
}

impl LogRecorder {
    /// Inspect all spans that have been recorded.
    #[must_use]
    pub fn spans(&self) -> Vec<SpanData> {
        // Force flush the tracer provider to ensure spans are exported
        self.guard.force_flush().unwrap();
        self.spans.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.guard.force_flush().unwrap();
        self.spans.lock().unwrap().clear();
    }
}

static SEQUENTIAL_TEST: LazyLock<RwLock<()>> = LazyLock::new(|| RwLock::new(()));

/// Guard that allows tests to run exclusively.
///
/// As tracing subscribers are initialized globally, tests need to run exclusively
/// if they want to inspect spans or logs.
#[allow(dead_code)]
pub enum SequentialTestGuard {
    Parallel(RwLockReadGuard<'static, ()>),
    Exclusive(RwLockWriteGuard<'static, ()>),
}

impl SequentialTestGuard {
    async fn parallel() -> Self {
        Self::Parallel(SEQUENTIAL_TEST.read().await)
    }

    async fn exclusive() -> Self {
        Self::Exclusive(SEQUENTIAL_TEST.write().await)
    }
}

/// Ensure a tracing subscriber is initialized.
///
/// This is useful for tests that are testing logging/tracing related functionality.
/// While we would like to use [`tracing::subscriber::with_default`] to set the subscriber for
/// the scope of the test, this only applies to a single thread, and proved to be flaky as
/// tokio may choose to run certain actors on a different thread.
static INITIALIZE_TRACING_SUBSCRIBER: LazyLock<LogRecorder> = LazyLock::new(tracing_subscriber);

/// Non-exclusive guard that ensures a tracing subscriber is initialized.
///
/// This might be used for tests where we want logging/tracing to be enabled to simulate
/// the production environment, but are not interested in inspecting the logs.
pub async fn log_recorder() -> (SequentialTestGuard, &'static LogRecorder) {
    let log_recorder = &INITIALIZE_TRACING_SUBSCRIBER;
    let guard = SequentialTestGuard::parallel().await;
    (guard, log_recorder)
}

/// Exclusive guard that ensures a tracing subscriber is initialized and returns a fresh log
/// recorder.
///
/// May be used for tests that want to inspect the logs.
pub async fn exclusive_log_recorder() -> (SequentialTestGuard, &'static LogRecorder) {
    let log_recorder = &INITIALIZE_TRACING_SUBSCRIBER;

    // Aquire an exclusive guard and clear the log recorder
    let guard = SequentialTestGuard::exclusive().await;
    log_recorder.clear();
    assert!(log_recorder.spans().is_empty());
    (guard, log_recorder)
}

fn tracing_subscriber() -> LogRecorder {
    let spans = Arc::new(Mutex::new(Vec::new()));
    let exporter = SpySpanExporter::new(spans.clone());
    let provider = tracer_provider(exporter, 1.0).unwrap();

    init_propagator();

    let tracer = provider.tracer("test");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    tracing_subscriber::registry()
        .with(EnvFilter::from_str("info").unwrap())
        .with(otel_layer)
        .init();

    LogRecorder {
        spans,
        guard: provider,
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
