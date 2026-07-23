//! Typed error taxonomy for the library surface (rule
//! `thiserror-for-libs-eyre-for-bins`).

/// Errors returned by the S3 and `DynamoDB` pollers, and by control-endpoint
/// setup.
///
/// AWS SDK errors are captured with their operation name and a `Debug`-
/// formatted detail string rather than being wrapped structurally: this
/// crate treats every SDK failure the same way (log it, count it, keep the
/// link running — see `s3.rs`/`ddb.rs` `apply_due`), so there is no call site
/// that needs to match on a specific AWS error variant.
#[derive(Debug, thiserror::Error)]
pub enum ReplicatorError {
    /// An S3 SDK call failed.
    #[error("S3 {op} failed: {detail}")]
    S3 {
        /// The S3 operation name (e.g. `"put_object"`).
        op: &'static str,
        /// `Debug`-formatted SDK error detail.
        detail: String,
    },
    /// A `DynamoDB` SDK call failed.
    #[error("DynamoDB {op} failed: {detail}")]
    Ddb {
        /// The `DynamoDB` operation name (e.g. `"put_item"`).
        op: &'static str,
        /// `Debug`-formatted SDK error detail.
        detail: String,
    },
    /// Reading a response body failed.
    #[error("failed to read response body: {0}")]
    Body(String),
    /// Link configuration was invalid.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    /// The control HTTP endpoint failed to bind.
    #[error("control endpoint failed to bind {addr}: {source}")]
    ControlBind {
        /// The address that failed to bind.
        addr: std::net::SocketAddr,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
}

impl ReplicatorError {
    /// Builds an [`ReplicatorError::S3`] from any `Debug`-formattable SDK
    /// error (typically `SdkError<Op..Error, HttpResponse>`).
    pub(crate) fn s3(op: &'static str, err: &impl std::fmt::Debug) -> Self {
        Self::S3 {
            op,
            detail: format!("{err:?}"),
        }
    }

    /// Builds a [`ReplicatorError::Ddb`] from any `Debug`-formattable SDK
    /// error.
    pub(crate) fn ddb(op: &'static str, err: &impl std::fmt::Debug) -> Self {
        Self::Ddb {
            op,
            detail: format!("{err:?}"),
        }
    }
}
