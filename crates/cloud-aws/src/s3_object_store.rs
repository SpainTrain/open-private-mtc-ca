//! [`S3ObjectStore`] -- [`ObjectStore`] over `aws-sdk-s3` (spec §9.3).
//!
//! See the crate-level docs for how S3's versioned object model is
//! reconciled with the single-mutable-object-per-key `cloud-types` contract;
//! this module implements the [`ObjectStore`] half (`s3_object_lock.rs`
//! implements [`ObjectLock`](cloud_types::ObjectLock) on the same struct).

use async_trait::async_trait;
use aws_sdk_s3::operation::put_object::builders::PutObjectFluentBuilder;
use aws_sdk_s3::primitives::{ByteStream, DateTime as AwsDateTime};
use aws_sdk_s3::Client;
use cloud_types::{CloudError, ObjectInfo, ObjectMetadata, ObjectStore, PutMode, PutOptions};
use std::time::SystemTime;

use crate::config::{build_client, S3Config};
use crate::error::{map_error, Op};

/// S3-backed [`ObjectStore`] -- and, via the same type,
/// [`ObjectLock`](cloud_types::ObjectLock) (see `s3_object_lock.rs` and the
/// crate-level docs).
///
/// Cheap to [`Clone`]: `aws_sdk_s3::Client` is internally `Arc`-shared, so
/// every clone talks to the same bucket over the same connection pool -- the
/// same sharing pattern `Arc<dyn ObjectStore>` / `Arc<dyn ObjectLock>` need
/// from the `Backend` factory (spec §9.4).
#[derive(Clone)]
pub struct S3ObjectStore {
    pub(crate) client: Client,
    pub(crate) bucket: String,
}

impl S3ObjectStore {
    /// Creates a store targeting `config.bucket` via a client built from
    /// `config` (ticket aws-backend AC: "Client construction accepts
    /// injected endpoint/credentials config so `LocalStack` and
    /// (hypothetically) real AWS use the same code path").
    ///
    /// Does not verify the bucket exists or is configured correctly --
    /// bucket lifecycle is out of this crate's scope (see the crate-level
    /// docs).
    #[must_use]
    pub fn new(config: S3Config) -> Self {
        Self {
            client: build_client(&config),
            bucket: config.bucket,
        }
    }

    /// Builds (without sending) the `PutObject` request for `put`, shaped by
    /// `opts.mode`: [`PutMode::IfNotExists`] adds the `If-None-Match: *`
    /// conditional-create header (ticket AC: "put (If-None-Match for
    /// no-overwrite semantics)"); [`PutMode::Overwrite`] adds none.
    fn put_request(&self, key: &str, data: &[u8], opts: PutOptions) -> PutObjectFluentBuilder {
        let req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data.to_vec()));
        match opts.mode {
            PutMode::IfNotExists => req.if_none_match("*"),
            PutMode::Overwrite => req,
        }
    }

    /// Fetches the current Object Lock retain-until date for `key`, if any.
    ///
    /// `Ok(None)` covers *both* "no such key" and "key exists but carries no
    /// Object Lock retention" -- exactly the two cases
    /// [`ObjectLock::get_retention`](cloud_types::ObjectLock::get_retention)'s
    /// contract collapses into a single [`CloudError::NotFound`], and
    /// exactly the two cases under which [`ObjectStore::put`]'s
    /// [`PutMode::Overwrite`] pre-check (below) has nothing to block.
    ///
    /// # Errors
    ///
    /// Returns [`CloudError::Transport`] on a transport/service failure
    /// other than "not found" (which collapses into `Ok(None)` above).
    pub(crate) async fn current_retention(
        &self,
        key: &str,
    ) -> Result<Option<AwsDateTime>, CloudError> {
        match self
            .client
            .get_object_retention()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => Ok(output
                .retention()
                .and_then(|retention| retention.retain_until_date())
                .copied()),
            Err(err) => match map_error(Op::Read, key, &err) {
                CloudError::NotFound { .. } => Ok(None),
                other => Err(other),
            },
        }
    }
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    async fn put(&self, key: &str, data: &[u8], opts: PutOptions) -> Result<(), CloudError> {
        match opts.mode {
            PutMode::IfNotExists => self
                .put_request(key, data, opts)
                .send()
                .await
                .map(|_| ())
                .map_err(|err| map_error(Op::CreateOnlyWrite, key, &err)),
            PutMode::Overwrite => {
                // A plain PutObject never touches an existing version -- it
                // just creates a new current one -- so S3 itself would never
                // reject this write even if the current version is locked.
                // See the crate-level docs' "S3 Object Lock semantics" for
                // why this crate refuses any key that has ever carried
                // Object Lock retention rather than trying to re-derive
                // "has retention expired" client-side.
                if self.current_retention(key).await?.is_some() {
                    return Err(CloudError::RetentionViolation {
                        reason: format!(
                            "{key}: carries Object Lock retention; PutMode::Overwrite never \
                             targets append-only log content"
                        ),
                    });
                }
                self.put_request(key, data, opts)
                    .send()
                    .await
                    .map(|_| ())
                    .map_err(|err| map_error(Op::RetentionGuardedWrite, key, &err))
            }
        }
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|err| map_error(Op::Read, key, &err))?;
        let bytes = output
            .body
            .collect()
            .await
            .map_err(|err| CloudError::Transport {
                retryable: true,
                reason: format!("failed to read response body: {err}"),
            })?;
        Ok(bytes.into_bytes().to_vec())
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, CloudError> {
        let output = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|err| map_error(Op::Read, key, &err))?;
        Ok(ObjectMetadata {
            size_bytes: non_negative_u64(output.content_length()),
            last_modified: system_time_or_epoch(output.last_modified().copied()),
        })
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>, CloudError> {
        let mut listed = Vec::new();
        let mut continuation_token: Option<String> = None;
        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix);
            if let Some(token) = &continuation_token {
                req = req.continuation_token(token);
            }
            let output = req
                .send()
                .await
                .map_err(|err| map_error(Op::Read, prefix, &err))?;
            for object in output.contents() {
                let Some(key) = object.key() else { continue };
                listed.push(ObjectInfo {
                    key: key.to_string(),
                    size_bytes: non_negative_u64(object.size()),
                    last_modified: system_time_or_epoch(object.last_modified().copied()),
                });
            }
            if output.is_truncated() == Some(true) {
                continuation_token = output.next_continuation_token().map(String::from);
            } else {
                break;
            }
        }
        // ListObjectsV2 already returns lexicographic key order; sorting
        // explicitly documents that guarantee as essential rather than
        // assumed, matching the shared suite's
        // `test_list_returns_only_matching_prefix_sorted_by_key`.
        listed.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(listed)
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        // Resolve the current version_id via HeadObject (also our existence
        // check -- NotFound if missing), then delete *that version*
        // specifically. See the crate-level docs' "S3 Object Lock
        // semantics": an unversioned DeleteObject on a locked key would
        // "succeed" by adding a delete marker while leaving the locked bytes
        // physically present, which is not what this crate's delete means.
        let head = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|err| map_error(Op::Read, key, &err))?;
        let mut req = self.client.delete_object().bucket(&self.bucket).key(key);
        if let Some(version_id) = head.version_id() {
            req = req.version_id(version_id);
        }
        req.send()
            .await
            .map(|_| ())
            .map_err(|err| map_error(Op::RetentionGuardedWrite, key, &err))
    }
}

/// Saturating, non-panicking `i64` (S3 sizes/lengths) -> `u64` conversion --
/// `as u64` is a banned lossy cast under `clippy::pedantic`, and a negative
/// or missing size should never happen but must not panic if it somehow
/// does.
fn non_negative_u64(value: Option<i64>) -> u64 {
    value
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0)
}

/// Converts an S3-reported timestamp to [`SystemTime`], falling back to the
/// Unix epoch for the (never expected in practice) case of a timestamp
/// outside `SystemTime`'s representable range.
fn system_time_or_epoch(value: Option<AwsDateTime>) -> SystemTime {
    value
        .and_then(|value| SystemTime::try_from(value).ok())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> S3ObjectStore {
        S3ObjectStore::new(S3Config::localstack("test-bucket", "http://127.0.0.1:4566"))
    }

    #[test]
    fn if_not_exists_put_sets_conditional_header() {
        let req = store().put_request("k", b"v", PutOptions::if_not_exists());
        assert_eq!(req.get_if_none_match().as_deref(), Some("*"));
    }

    #[test]
    fn overwrite_put_sets_no_conditional_header() {
        let req = store().put_request("k", b"v", PutOptions::overwrite());
        assert_eq!(req.get_if_none_match(), &None);
    }

    #[test]
    fn put_request_targets_the_configured_bucket_and_key() {
        let req = store().put_request("entries/0001", b"leaf", PutOptions::default());
        assert_eq!(req.get_bucket().as_deref(), Some("test-bucket"));
        assert_eq!(req.get_key().as_deref(), Some("entries/0001"));
    }

    #[test]
    fn non_negative_u64_saturates_missing_or_negative_to_zero() {
        assert_eq!(non_negative_u64(Some(42)), 42);
        assert_eq!(non_negative_u64(Some(-1)), 0);
        assert_eq!(non_negative_u64(None), 0);
    }

    #[test]
    fn system_time_or_epoch_falls_back_on_missing_timestamp() {
        assert_eq!(system_time_or_epoch(None), SystemTime::UNIX_EPOCH);
    }
}
