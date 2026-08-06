//! Client construction: [`S3Config`] carries the injected endpoint/
//! credentials so `LocalStack` and (hypothetically) real AWS use the same
//! code path (ticket aws-backend AC) -- see
//! [`S3ObjectStore::new`](crate::S3ObjectStore::new).

use aws_sdk_s3::config::{BehaviorVersion, Builder, Credentials, Region};
use aws_sdk_s3::Client;

/// Long-term static credentials for a non-IAM endpoint.
///
/// `LocalStack` accepts any non-empty access key / secret pair; real AWS
/// never sees this struct when [`S3Config::credentials`] is `None`, since
/// the SDK's standard credential provider chain (environment, IAM role, ...)
/// applies instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCredentials {
    /// Access key ID.
    pub access_key_id: String,
    /// Secret access key.
    pub secret_access_key: String,
}

/// Configuration for [`S3ObjectStore::new`](crate::S3ObjectStore::new).
///
/// The same struct -- and the same client-construction code path -- serves
/// `LocalStack` today and real AWS in the future (ticket aws-backend AC:
/// "Client construction accepts injected endpoint/credentials config"):
/// `endpoint_url`/`credentials` are `Some` for `LocalStack` (or any
/// S3-compatible endpoint under test) and `None` for real AWS, where the
/// SDK's standard endpoint resolution and credential provider chain apply
/// instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Bucket every [`S3ObjectStore`](crate::S3ObjectStore) operation
    /// targets.
    ///
    /// Bucket lifecycle/schema (creation, versioning, Object Lock
    /// enablement) is out of this crate's scope (ticket aws-backend Out of
    /// Scope: "S3 bucket layout/schema", storage-facade epic) -- the bucket
    /// must already exist with versioning and Object Lock enabled for
    /// [`ObjectLock`](cloud_types::ObjectLock) methods to behave per spec
    /// §9.5 (see the crate-level docs).
    pub bucket: String,
    /// AWS region string. `LocalStack` accepts any region-shaped string.
    pub region: String,
    /// Explicit endpoint URL (`LocalStack`: `http://127.0.0.1:4566`). `None`
    /// uses the SDK's standard endpoint resolution (real AWS).
    pub endpoint_url: Option<String>,
    /// Explicit static credentials. `None` uses the SDK's standard
    /// credential provider chain (real AWS: IAM role / environment / ...).
    pub credentials: Option<StaticCredentials>,
    /// Path-style addressing (`http://host/bucket/key` instead of
    /// `http://bucket.host/key`) -- required by `LocalStack`, which does not
    /// terminate per-bucket virtual-host DNS.
    pub force_path_style: bool,
}

impl S3Config {
    /// Convenience constructor for a `LocalStack` target: dummy static
    /// credentials, `us-east-1`, and path-style addressing -- the same
    /// pattern `crates/dev-replicator`'s test helpers use (see
    /// `dev-replicator/tests/integration.rs::s3_client`).
    #[must_use]
    pub fn localstack(bucket: impl Into<String>, endpoint_url: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            region: "us-east-1".to_string(),
            endpoint_url: Some(endpoint_url.into()),
            credentials: Some(StaticCredentials {
                access_key_id: "test".to_string(),
                secret_access_key: "test".to_string(),
            }),
            force_path_style: true,
        }
    }
}

/// Builds an `aws_sdk_s3::Client` from `config`.
///
/// The one place a vendor SDK config type is constructed (rule
/// no-sdk-types-in-domain, spec §22.8): this stays inside the backend crate
/// and never crosses the `cloud-types` trait boundary.
pub fn build_client(config: &S3Config) -> Client {
    let mut builder = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(config.region.clone()))
        .force_path_style(config.force_path_style);
    if let Some(endpoint_url) = &config.endpoint_url {
        builder = builder.endpoint_url(endpoint_url.clone());
    }
    if let Some(creds) = &config.credentials {
        builder = builder.credentials_provider(Credentials::new(
            creds.access_key_id.clone(),
            creds.secret_access_key.clone(),
            None,
            None,
            "cloud-aws-static",
        ));
    }
    Client::from_conf(builder.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localstack_config_sets_dummy_credentials_and_path_style() {
        let cfg = S3Config::localstack("my-bucket", "http://127.0.0.1:4566");
        assert_eq!(cfg.bucket, "my-bucket");
        assert_eq!(cfg.endpoint_url.as_deref(), Some("http://127.0.0.1:4566"));
        assert!(cfg.force_path_style);
        assert!(cfg.credentials.is_some());
    }

    #[test]
    fn build_client_does_not_panic_with_or_without_endpoint() {
        // Request-shaping / construction smoke test -- no network involved:
        // `Client::from_conf` only assembles local config.
        let with_endpoint = S3Config::localstack("bucket", "http://127.0.0.1:4566");
        let _client = build_client(&with_endpoint);

        let real_aws_shaped = S3Config {
            bucket: "bucket".to_string(),
            region: "us-east-1".to_string(),
            endpoint_url: None,
            credentials: None,
            force_path_style: false,
        };
        let _client = build_client(&real_aws_shaped);
    }
}
