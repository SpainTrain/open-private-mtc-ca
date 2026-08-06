//! Maps `aws-sdk-s3` errors onto the shared [`CloudError`] taxonomy (ticket
//! aws-backend AC: "SDK errors mapped onto the cloud-types error taxonomy
//! ... `ConditionFailed`/`AlreadyExists` on precondition failure").
//!
//! S3 error codes are ambiguous on their own: `AccessDenied` means "IAM
//! denied this call" in general, but in this crate the only S3 calls that
//! can legitimately hit it are the ones an active Object Lock Compliance
//! retention would block. [`Op`] carries that context so [`classify`]
//! reinterprets each code correctly per call site -- the translation layer
//! rule no-sdk-types-in-domain (spec §22.8) says belongs in this backend
//! crate, never in `cloud-types` itself.

use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use cloud_types::CloudError;

/// What the failing S3 call was trying to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// A read: `GetObject`, `HeadObject`, `ListObjectsV2`,
    /// `GetObjectRetention`.
    Read,
    /// `PutObject` under [`PutMode::IfNotExists`](cloud_types::PutMode::IfNotExists)
    /// or `ObjectLock::put_with_retention` -- both create-only via
    /// `If-None-Match: *`.
    CreateOnlyWrite,
    /// `PutObject` under [`PutMode::Overwrite`](cloud_types::PutMode::Overwrite),
    /// or the versioned `DeleteObject` `ObjectStore::delete` issues -- both
    /// blocked by an active Object Lock Compliance retention.
    RetentionGuardedWrite,
    /// `PutObjectRetention`, for `ObjectLock::extend_retention`.
    RetentionExtend,
}

/// Maps a failed S3 SDK call to [`CloudError`].
///
/// `key` is the object key involved (used to populate
/// [`CloudError::NotFound`] / [`CloudError::AlreadyExists`] /
/// [`CloudError::RetentionViolation`]).
pub fn map_error<E>(op: Op, key: &str, err: &SdkError<E>) -> CloudError
where
    E: ProvideErrorMetadata,
{
    if !matches!(err, SdkError::ServiceError(_)) {
        return transport_error(err);
    }
    let status = err.raw_response().map(|r| r.status().as_u16());
    classify(op, key, err.code().unwrap_or_default(), status)
}

/// Pure classification of an already-extracted S3 error `code` -- no SDK or
/// network types involved, so this is directly unit-testable without a
/// `LocalStack` container (ticket Testing AC: "error-mapping ... tests
/// without network").
pub fn classify(op: Op, key: &str, code: &str, status: Option<u16>) -> CloudError {
    match (op, code) {
        // "NoSuchKey"/"NoSuchVersion" come from operations with an XML error
        // body (GetObject, DeleteObject with a bad version, ...). "NotFound"
        // is what HeadObject synthesizes instead -- a HEAD response has no
        // body to carry an S3 error code, so the SDK reports the bare HTTP
        // 404 as this generic code.
        (_, "NoSuchKey" | "NoSuchVersion" | "NotFound") => CloudError::NotFound {
            key: key.to_string(),
        },
        (Op::Read, "NoSuchObjectLockConfiguration" | "ObjectLockConfigurationNotFoundError") => {
            CloudError::NotFound {
                key: key.to_string(),
            }
        }
        (Op::CreateOnlyWrite, "PreconditionFailed" | "ConditionalRequestConflict") => {
            CloudError::AlreadyExists {
                key: key.to_string(),
            }
        }
        (Op::RetentionGuardedWrite, "AccessDenied" | "InvalidRequest" | "InvalidArgument") => {
            CloudError::RetentionViolation {
                reason: format!(
                    "{key}: blocked by an active Object Lock Compliance retention (S3 {code})"
                ),
            }
        }
        (Op::RetentionExtend, "AccessDenied" | "InvalidRequest" | "InvalidArgument") => {
            CloudError::RetentionViolation {
                reason: format!(
                    "{key}: retention extension rejected -- Compliance mode is forward-only \
                     (S3 {code})"
                ),
            }
        }
        _ => generic_transport(code, status),
    }
}

/// Retryable-vs-terminal fallback for S3 error codes this crate does not
/// give special-case meaning to.
fn generic_transport(code: &str, status: Option<u16>) -> CloudError {
    let retryable = matches!(
        code,
        "SlowDown"
            | "Throttling"
            | "ThrottlingException"
            | "RequestTimeout"
            | "InternalError"
            | "ServiceUnavailable"
            | "RequestTimeTooSkewed"
    ) || matches!(status, Some(status) if status == 429 || status >= 500);
    CloudError::Transport {
        retryable,
        reason: status.map_or_else(
            || format!("S3 error {code}"),
            |status| format!("S3 error {code} (HTTP {status})"),
        ),
    }
}

/// Classifies `SdkError` variants that never reached the service (no S3
/// error code to inspect): construction, timeout, dispatch, and malformed
/// response.
fn transport_error<E>(err: &SdkError<E>) -> CloudError
where
    E: ProvideErrorMetadata,
{
    match err {
        SdkError::ConstructionFailure(_) => CloudError::Transport {
            retryable: false,
            reason: format!("request construction failed: {err}"),
        },
        SdkError::TimeoutError(_) => CloudError::Transport {
            retryable: true,
            reason: "request timed out".to_string(),
        },
        SdkError::DispatchFailure(_) => CloudError::Transport {
            retryable: true,
            reason: format!("dispatch failure: {err}"),
        },
        SdkError::ResponseError(_) => CloudError::Transport {
            retryable: true,
            reason: "malformed response".to_string(),
        },
        SdkError::ServiceError(_) => generic_transport(
            ProvideErrorMetadata::code(err).unwrap_or_default(),
            err.raw_response().map(|r| r.status().as_u16()),
        ),
        // SdkError is #[non_exhaustive] on the vendor side; every variant it
        // has today is matched above, so this can only be a future addition
        // -- fall back to the same code/status-based classification the
        // ServiceError arm uses rather than panicking.
        _ => generic_transport(
            ProvideErrorMetadata::code(err).unwrap_or_default(),
            err.raw_response().map(|r| r.status().as_u16()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_is_not_found_regardless_of_operation() {
        for op in [
            Op::Read,
            Op::CreateOnlyWrite,
            Op::RetentionGuardedWrite,
            Op::RetentionExtend,
        ] {
            let err = classify(op, "entries/0001", "NoSuchKey", Some(404));
            assert!(matches!(err, CloudError::NotFound { key } if key == "entries/0001"));
        }
    }

    #[test]
    fn head_object_missing_key_generic_not_found_code_is_not_found() {
        // HeadObject has no response body, so S3 (observed empirically
        // against LocalStack) reports the bare 404 as "NotFound" rather than
        // "NoSuchKey" -- see crate-level docs "LocalStack emulation
        // caveats".
        let err = classify(Op::Read, "entries/0001", "NotFound", Some(404));
        assert!(matches!(err, CloudError::NotFound { key } if key == "entries/0001"));
    }

    #[test]
    fn no_retention_configuration_on_read_is_not_found() {
        let err = classify(
            Op::Read,
            "entries/0001",
            "NoSuchObjectLockConfiguration",
            Some(404),
        );
        assert!(matches!(err, CloudError::NotFound { key } if key == "entries/0001"));
    }

    #[test]
    fn precondition_failed_on_create_only_write_is_already_exists() {
        let err = classify(
            Op::CreateOnlyWrite,
            "entries/0001",
            "PreconditionFailed",
            Some(412),
        );
        assert!(matches!(err, CloudError::AlreadyExists { key } if key == "entries/0001"));
    }

    #[test]
    fn access_denied_on_retention_guarded_write_is_retention_violation() {
        let err = classify(
            Op::RetentionGuardedWrite,
            "checkpoints/0001",
            "AccessDenied",
            Some(403),
        );
        assert!(
            matches!(err, CloudError::RetentionViolation { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn access_denied_on_retention_extend_is_retention_violation() {
        let err = classify(
            Op::RetentionExtend,
            "checkpoints/0001",
            "AccessDenied",
            Some(403),
        );
        assert!(
            matches!(err, CloudError::RetentionViolation { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn access_denied_on_a_plain_read_is_not_reinterpreted_as_retention_violation() {
        // Op-scoping matters: the same S3 code means something different
        // depending on which cloud-types operation triggered it.
        let err = classify(Op::Read, "entries/0001", "AccessDenied", Some(403));
        assert!(
            matches!(
                err,
                CloudError::Transport {
                    retryable: false,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn throttling_codes_are_retryable_transport() {
        for code in ["SlowDown", "Throttling", "RequestTimeout", "InternalError"] {
            let err = classify(Op::Read, "k", code, Some(503));
            assert!(
                matches!(
                    err,
                    CloudError::Transport {
                        retryable: true,
                        ..
                    }
                ),
                "{code} -> {err:?}"
            );
        }
    }

    #[test]
    fn five_xx_status_is_retryable_even_for_an_unrecognized_code() {
        let err = classify(Op::Read, "k", "SomeNewServiceFault", Some(503));
        assert!(
            matches!(
                err,
                CloudError::Transport {
                    retryable: true,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn four_xx_status_is_terminal_for_an_unrecognized_code() {
        let err = classify(Op::Read, "k", "SomeClientFault", Some(400));
        assert!(
            matches!(
                err,
                CloudError::Transport {
                    retryable: false,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn construction_failure_is_terminal_transport() {
        let err: CloudError = super::transport_error(&SdkError::<TestError>::construction_failure(
            "bad request shape",
        ));
        assert!(
            matches!(
                err,
                CloudError::Transport {
                    retryable: false,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn timeout_is_retryable_transport() {
        let err: CloudError =
            super::transport_error(&SdkError::<TestError>::timeout_error("took too long"));
        assert!(
            matches!(
                err,
                CloudError::Transport {
                    retryable: true,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    /// Minimal `ProvideErrorMetadata` implementor for exercising
    /// `transport_error` without a real S3 operation error type.
    #[derive(Debug)]
    struct TestError;

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("test error")
        }
    }

    impl std::error::Error for TestError {}

    impl ProvideErrorMetadata for TestError {
        fn meta(&self) -> &aws_sdk_s3::error::ErrorMetadata {
            use std::sync::OnceLock;
            static META: OnceLock<aws_sdk_s3::error::ErrorMetadata> = OnceLock::new();
            META.get_or_init(|| aws_sdk_s3::error::ErrorMetadata::builder().build())
        }
    }
}
