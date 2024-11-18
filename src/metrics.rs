use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};

use crate::shell::MetricNames;

/// Initializes a recorder for metrics macros and exposes a Prometheus exporter
/// and exposes it over port 9000 (by default). Any GET request at the port
/// will return metrics.
///
/// # Errors
/// Returns an error if:
/// - Unable to set metric buckets
/// - Unable to install the Prometheus recorder
pub fn initialize_metrics() -> anyhow::Result<()> {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(MetricNames::HttpRequestsDurationSeconds.to_string()),
            EXPONENTIAL_SECONDS,
        )?
        .install()?;

    Ok(())
}
