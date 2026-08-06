//! [`ObjectLock`] impl for [`S3ObjectStore`] over S3 Object Lock (spec §9.3,
//! §9.5).
//!
//! Retention is enforced by S3 itself against its own real wall-clock time
//! -- this module never reads local time (rule no-systemtime-now-in-prod is
//! moot here, not merely obeyed: there is nothing to compare against
//! locally). See the crate-level docs for how S3's versioned object model
//! is reconciled with the `cloud-types` contract.

use async_trait::async_trait;
use aws_sdk_s3::primitives::DateTime as AwsDateTime;
use aws_sdk_s3::types::{ObjectLockMode, ObjectLockRetention, ObjectLockRetentionMode};
use cloud_types::{CloudError, ObjectLock};
use std::time::SystemTime;

use crate::error::{map_error, Op};
use crate::s3_object_store::S3ObjectStore;

/// [`S3ObjectStore`] also implements [`ObjectLock`].
///
/// Named by the capability it provides at construction sites (e.g. wiring a
/// `Backend`'s `object_lock: Arc<dyn ObjectLock>` field -- spec §9.4),
/// mirroring `cloud-memory`'s `MemoryObjectLock` alias.
pub type S3ObjectLock = S3ObjectStore;

/// Converts a caller-supplied retention instant to the wire `DateTime` type,
/// and back on read. S3 rejects a `retain_until_date` that is not in the
/// future (relative to its own clock) at write time -- callers must supply a
/// genuinely future `SystemTime`, exactly as
/// [`ObjectLock::put_with_retention`]'s contract already requires.
fn retention_conversion_error(key: &str, detail: impl std::fmt::Display) -> CloudError {
    CloudError::Transport {
        retryable: false,
        reason: format!("{key}: invalid Object Lock retain-until date from S3: {detail}"),
    }
}

#[async_trait]
impl ObjectLock for S3ObjectStore {
    async fn put_with_retention(
        &self,
        key: &str,
        data: &[u8],
        retain_until: SystemTime,
    ) -> Result<(), CloudError> {
        // One atomic PutObject: If-None-Match: * (create-only) plus the
        // Object Lock headers in the same request, so there is no window
        // where the object exists unretained.
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .if_none_match("*")
            .object_lock_mode(ObjectLockMode::Compliance)
            .object_lock_retain_until_date(AwsDateTime::from(retain_until))
            .body(data.to_vec().into())
            .send()
            .await
            .map(|_| ())
            .map_err(|err| map_error(Op::CreateOnlyWrite, key, &err))
    }

    async fn extend_retention(
        &self,
        key: &str,
        new_retain_until: SystemTime,
    ) -> Result<(), CloudError> {
        let current = self
            .current_retention(key)
            .await?
            .ok_or_else(|| CloudError::NotFound {
                key: key.to_string(),
            })?;
        let current_std =
            SystemTime::try_from(current).map_err(|err| retention_conversion_error(key, err))?;
        if new_retain_until <= current_std {
            return Err(CloudError::RetentionViolation {
                reason: format!(
                    "{key}: retention is forward-only (current {current_std:?}, requested \
                     {new_retain_until:?})"
                ),
            });
        }
        self.client
            .put_object_retention()
            .bucket(&self.bucket)
            .key(key)
            .retention(
                ObjectLockRetention::builder()
                    .mode(ObjectLockRetentionMode::Compliance)
                    .retain_until_date(AwsDateTime::from(new_retain_until))
                    .build(),
            )
            .send()
            .await
            .map(|_| ())
            .map_err(|err| map_error(Op::RetentionExtend, key, &err))
    }

    async fn get_retention(&self, key: &str) -> Result<SystemTime, CloudError> {
        let retain_until =
            self.current_retention(key)
                .await?
                .ok_or_else(|| CloudError::NotFound {
                    key: key.to_string(),
                })?;
        SystemTime::try_from(retain_until).map_err(|err| retention_conversion_error(key, err))
    }
}
