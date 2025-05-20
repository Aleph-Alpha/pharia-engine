use std::net::SocketAddr;

use crate::{shell::ShellMetrics, skill_runtime::SkillRuntimeMetrics};
use metrics::KeyName;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use tracing::error;

/// Initializes a recorder for metrics macros and exposes a Prometheus exporter
/// and exposes it over port 9000 (by default). Any GET request at the port
/// will return metrics.
///
/// We expect logging to be initialized already, so we may use error macros.
///
/// # Errors
/// Returns an error if:
/// - Unable to set metric buckets
/// - Unable to install the Prometheus recorder
pub fn initialize_metrics(addr: impl Into<SocketAddr>) -> anyhow::Result<()> {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(
                KeyName::from(ShellMetrics::HttpRequestsDurationSeconds)
                    .as_str()
                    .to_owned(),
            ),
            EXPONENTIAL_SECONDS,
        )
        .inspect_err(|e| {
            error!("Error setting buckets for metric: {e}");
        })?
        .set_buckets_for_metric(
            Matcher::Full(
                KeyName::from(SkillRuntimeMetrics::ExecutionDurationSeconds)
                    .as_str()
                    .to_owned(),
            ),
            EXPONENTIAL_SECONDS,
        )
        .inspect_err(|e| {
            error!("Error setting buckets for metric: {e}");
        })?
        .set_buckets_for_metric(
            Matcher::Full(
                KeyName::from(SkillRuntimeMetrics::FetchDurationSeconds)
                    .as_str()
                    .to_owned(),
            ),
            EXPONENTIAL_SECONDS,
        )
        .inspect_err(|e| {
            error!("Error setting buckets for metric: {e}");
        })?
        .with_http_listener(addr)
        .install()
        .inspect_err(|e| {
            error!("Error installing metrics: {e}");
        })?;

    Ok(())
}
