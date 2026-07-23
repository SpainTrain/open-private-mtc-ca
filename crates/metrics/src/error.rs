//! Error types for the metrics facade (`thiserror`, per repo rule
//! `thiserror-for-libs-eyre-for-bins`).

/// Errors surfaced by the metrics registry and its exporters.
///
/// Third-party error details are carried as strings so the public API stays
/// independent of the backing implementation crates.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MetricsError {
    /// A metric could not be registered (duplicate name, invalid options).
    #[error("metric registration failed: {0}")]
    Registration(String),

    /// Prometheus text-format encoding failed.
    #[error("prometheus text encoding failed: {0}")]
    Encode(String),

    /// EMF JSON serialization failed.
    #[error("EMF serialization failed: {0}")]
    Emf(String),

    /// I/O error from the admin endpoint or an EMF sink.
    #[error("metrics I/O error: {0}")]
    Io(#[from] std::io::Error),
}
