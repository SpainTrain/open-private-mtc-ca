//! Maps `aws-sdk-s3` and `aws-sdk-dynamodb` errors onto the shared
//! [`CloudError`] taxonomy (ticket aws-backend AC: "SDK errors mapped onto
//! the cloud-types error taxonomy ... `ConditionFailed`/`AlreadyExists` on
//! precondition failure"; ticket mtc-lf7 carries the same requirement for
//! `ConditionalCheckFailedException` -> [`CloudError::ConditionFailed`]).
//!
//! S3 error codes are ambiguous on their own: `AccessDenied` means "IAM
//! denied this call" in general, but in this crate the only S3 calls that
//! can legitimately hit it are the ones an active Object Lock Compliance
//! retention would block. [`Op`] carries that context so [`classify`]
//! reinterprets each code correctly per call site -- the translation layer
//! rule no-sdk-types-in-domain (spec §22.8) says belongs in this backend
//! crate, never in `cloud-types` itself.
//!
//! `DynamoDB`'s errors need no such per-call-site reinterpretation --
//! `ConditionalCheckFailedException` unambiguously means
//! [`CloudError::ConditionFailed`] everywhere it can occur (`PutItem`,
//! `UpdateItem`; `TransactWriteItems` surfaces the equivalent per-item
//! `"ConditionalCheckFailed"` cancellation-reason code instead, see
//! [`map_transact_write_items_error`]) -- so the `DynamoDB` section below is a
//! flat, context-free classifier per operation-error type rather than an
//! `Op`-scoped one.

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

// ===========================================================================
// DynamoDB (ticket mtc-lf7) -- see the module docs for why this section is a
// flat, context-free classifier rather than an Op-scoped one like S3's above.
// Aliased imports avoid colliding with the S3 `ProvideErrorMetadata`/
// `SdkError` names already in scope from the top of this file.
// ===========================================================================

use aws_sdk_dynamodb::error::ProvideErrorMetadata as DdbProvideErrorMetadata;
use aws_sdk_dynamodb::error::SdkError as DdbSdkError;
use aws_sdk_dynamodb::operation::put_item::PutItemError;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::operation::update_item::UpdateItemError;

/// `true` iff `err` is a `DynamoDB` `ConditionalCheckFailedException` --
/// `PutItem`'s and `UpdateItem`'s ⚠️ CORRECTNESS-CRITICAL signal that a
/// [`cloud_types::Condition`] (or, for `atomic_update`, an increment-target
/// guard -- see `ddb_replicated_kv`'s module docs) did not hold. The
/// lease/epoch protocol (`crates/coordination`) depends on this mapping
/// being exact in both directions: a `ConditionalCheckFailedException` that
/// leaks out as a generic [`CloudError::Transport`], or any other failure
/// misclassified as a condition loss, breaks single-writer safety.
pub fn ddb_is_put_condition_failed(err: &DdbSdkError<PutItemError>) -> bool {
    err.as_service_error()
        .is_some_and(PutItemError::is_conditional_check_failed_exception)
}

/// `true` iff `err` is a `DynamoDB` `ConditionalCheckFailedException` from
/// `UpdateItem` -- see [`ddb_is_put_condition_failed`] (identical role, for
/// `atomic_update`'s `UpdateItem` call).
pub fn ddb_is_update_condition_failed(err: &DdbSdkError<UpdateItemError>) -> bool {
    err.as_service_error()
        .is_some_and(UpdateItemError::is_conditional_check_failed_exception)
}

/// Maps a failed `PutItem` call (`ReplicatedKv::put`) to [`CloudError`]:
/// [`CloudError::ConditionFailed`] on a lost CAS, else the generic
/// `DynamoDB` transport classification.
pub fn map_put_item_error(key: &str, err: &DdbSdkError<PutItemError>) -> CloudError {
    if ddb_is_put_condition_failed(err) {
        return CloudError::ConditionFailed {
            reason: format!("{key}: condition not satisfied"),
        };
    }
    ddb_generic_error(err)
}

/// Maps a failed `TransactWriteItems` call (`ReplicatedKv::transact`) to
/// [`CloudError`]. `TransactWriteItems` reports a lost condition as a
/// `TransactionCanceledException` whose `CancellationReasons` carry one
/// entry per transact item, ordered with the request (spec: the `DynamoDB`
/// API contract); a `"ConditionalCheckFailed"` code on *any* entry means the
/// whole transaction applied nothing (spec §9.5 -- `transact`'s contract
/// collapses every condition-loss cause to one [`CloudError::ConditionFailed`],
/// so this deliberately does not report *which* operation lost).
pub fn map_transact_write_items_error(err: &DdbSdkError<TransactWriteItemsError>) -> CloudError {
    if let Some(TransactWriteItemsError::TransactionCanceledException(cancel)) =
        err.as_service_error()
    {
        if any_condition_check_failed(cancel.cancellation_reasons()) {
            return CloudError::ConditionFailed {
                reason: "transact: at least one operation's condition did not hold".to_string(),
            };
        }
    }
    ddb_generic_error(err)
}

/// Pure classification of an already-extracted `CancellationReasons` list --
/// no SDK error-wrapper or network types involved, so this is directly
/// unit-testable (mirrors S3's [`classify`] "pure classification" split
/// above). `true` iff any entry carries `DynamoDB`'s
/// `"ConditionalCheckFailed"` cancellation code (the per-item code; note
/// this is *not* the same string as the standalone
/// `ConditionalCheckFailedException` service-error name that `PutItem`/
/// `UpdateItem` use).
fn any_condition_check_failed(reasons: &[aws_sdk_dynamodb::types::CancellationReason]) -> bool {
    reasons
        .iter()
        .any(|reason| reason.code() == Some("ConditionalCheckFailed"))
}

/// Generic `DynamoDB` classification for any operation-error type: no
/// `ConditionalCheckFailedException`/`TransactionCanceledException` special
/// case applies (either because this call site cannot produce one --
/// `GetItem`, `Query`, `Scan` -- or because the caller already ruled it out).
/// Transport/service faults only, retryable vs terminal by code/status --
/// the same shape as S3's [`generic_transport`], kept as a separate function
/// because the two SDKs' retryable-code vocabularies differ.
pub fn ddb_generic_error<E>(err: &DdbSdkError<E>) -> CloudError
where
    E: DdbProvideErrorMetadata,
{
    if !matches!(err, DdbSdkError::ServiceError(_)) {
        return ddb_transport_error(err);
    }
    let status = err.raw_response().map(|r| r.status().as_u16());
    ddb_generic_transport(
        DdbProvideErrorMetadata::code(err).unwrap_or_default(),
        status,
    )
}

/// Retryable-vs-terminal fallback for `DynamoDB` error codes: throttling,
/// capacity, and internal-fault codes are retryable; everything else
/// (validation, access-denied, resource-not-found -- e.g. a misconfigured
/// table name) is terminal.
fn ddb_generic_transport(code: &str, status: Option<u16>) -> CloudError {
    let retryable = matches!(
        code,
        "ProvisionedThroughputExceededException"
            | "ThrottlingException"
            | "RequestLimitExceeded"
            | "InternalServerError"
            | "LimitExceededException"
            | "TransactionInProgressException"
    ) || matches!(status, Some(status) if status == 429 || status >= 500);
    CloudError::Transport {
        retryable,
        reason: status.map_or_else(
            || format!("DynamoDB error {code}"),
            |status| format!("DynamoDB error {code} (HTTP {status})"),
        ),
    }
}

/// Classifies `DdbSdkError` variants that never reached the service --
/// mirrors S3's [`transport_error`] exactly (same variant set, same
/// retryable judgment per variant).
fn ddb_transport_error<E>(err: &DdbSdkError<E>) -> CloudError
where
    E: DdbProvideErrorMetadata,
{
    match err {
        DdbSdkError::ConstructionFailure(_) => CloudError::Transport {
            retryable: false,
            reason: format!("request construction failed: {err}"),
        },
        DdbSdkError::TimeoutError(_) => CloudError::Transport {
            retryable: true,
            reason: "request timed out".to_string(),
        },
        DdbSdkError::DispatchFailure(_) => CloudError::Transport {
            retryable: true,
            reason: format!("dispatch failure: {err}"),
        },
        DdbSdkError::ResponseError(_) => CloudError::Transport {
            retryable: true,
            reason: "malformed response".to_string(),
        },
        DdbSdkError::ServiceError(_) => ddb_generic_transport(
            DdbProvideErrorMetadata::code(err).unwrap_or_default(),
            err.raw_response().map(|r| r.status().as_u16()),
        ),
        // DdbSdkError is #[non_exhaustive] on the vendor side; fall back to
        // the same code/status classification rather than panicking.
        _ => ddb_generic_transport(
            DdbProvideErrorMetadata::code(err).unwrap_or_default(),
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

    // =======================================================================
    // DynamoDB (ticket mtc-lf7)
    // =======================================================================

    use aws_sdk_dynamodb::types::CancellationReason;

    #[test]
    fn condition_check_failed_is_detected_among_other_reasons() {
        let reasons = vec![
            CancellationReason::builder().build(), // "no error" entry: Null code
            CancellationReason::builder()
                .code("ConditionalCheckFailed")
                .message("The conditional request failed.")
                .build(),
        ];
        assert!(super::any_condition_check_failed(&reasons));
    }

    #[test]
    fn no_condition_check_failed_among_unrelated_or_absent_codes() {
        assert!(!super::any_condition_check_failed(&[]));
        let reasons = vec![CancellationReason::builder()
            .code("ProvisionedThroughputExceeded")
            .build()];
        assert!(!super::any_condition_check_failed(&reasons));
    }

    #[test]
    fn ddb_generic_transport_marks_throttling_and_capacity_codes_retryable() {
        for code in [
            "ProvisionedThroughputExceededException",
            "ThrottlingException",
            "RequestLimitExceeded",
            "InternalServerError",
        ] {
            let err = super::ddb_generic_transport(code, Some(400));
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
    fn ddb_generic_transport_five_xx_is_retryable_even_for_an_unrecognized_code() {
        let err = super::ddb_generic_transport("SomeNewServiceFault", Some(503));
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
    fn ddb_generic_transport_four_xx_is_terminal_for_an_unrecognized_code() {
        let err = super::ddb_generic_transport("ValidationException", Some(400));
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
    fn ddb_construction_failure_is_terminal_transport() {
        let err: CloudError = super::ddb_transport_error(
            &DdbSdkError::<DdbTestError>::construction_failure("bad request shape"),
        );
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
    fn ddb_timeout_is_retryable_transport() {
        let err: CloudError = super::ddb_transport_error(
            &DdbSdkError::<DdbTestError>::timeout_error("took too long"),
        );
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

    /// Minimal `DynamoDB` `ProvideErrorMetadata` implementor for exercising
    /// `ddb_transport_error` without a real `DynamoDB` operation error type --
    /// mirrors S3's `TestError` above exactly, against the `DynamoDB` SDK's
    /// (distinct) `ProvideErrorMetadata` trait.
    #[derive(Debug)]
    struct DdbTestError;

    impl std::fmt::Display for DdbTestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("test error")
        }
    }

    impl std::error::Error for DdbTestError {}

    impl DdbProvideErrorMetadata for DdbTestError {
        fn meta(&self) -> &aws_sdk_dynamodb::error::ErrorMetadata {
            use std::sync::OnceLock;
            static META: OnceLock<aws_sdk_dynamodb::error::ErrorMetadata> = OnceLock::new();
            META.get_or_init(|| aws_sdk_dynamodb::error::ErrorMetadata::builder().build())
        }
    }
}
