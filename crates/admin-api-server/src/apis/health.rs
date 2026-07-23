use async_trait::async_trait;
use axum::extract::*;
use axum_extra::extract::CookieJar;
use bytes::Bytes;
use headers::Host;
use http::Method;
use serde::{Deserialize, Serialize};

use crate::{models, types::*};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
#[allow(clippy::large_enum_variant)]
pub enum GetHealthzResponse {
    /// Process is alive.
    Status200_ProcessIsAlive(models::HealthzResponse),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
#[allow(clippy::large_enum_variant)]
pub enum GetReadyzResponse {
    /// Service is ready to serve.
    Status200_ServiceIsReadyToServe(models::ReadyzResponse),
    /// Service is not ready; body lists the failing checks.
    Status503_ServiceIsNotReady(models::ReadyzResponse),
}

/// Health
#[async_trait]
#[allow(clippy::ptr_arg)]
pub trait Health<E: std::fmt::Debug + Send + Sync + 'static = ()>: super::ErrorHandler<E> {
    /// Liveness probe.
    ///
    /// GetHealthz - GET /healthz
    async fn get_healthz(
        &self,

        method: &Method,
        host: &Host,
        cookies: &CookieJar,
    ) -> Result<GetHealthzResponse, E>;

    /// Readiness probe.
    ///
    /// GetReadyz - GET /readyz
    async fn get_readyz(
        &self,

        method: &Method,
        host: &Host,
        cookies: &CookieJar,
    ) -> Result<GetReadyzResponse, E>;
}
