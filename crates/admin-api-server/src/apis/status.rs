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
pub enum GetStatusResponse {
    /// Current service status.
    Status200_CurrentServiceStatus(models::StatusResponse),
    /// Unexpected error.
    Status0_UnexpectedError(models::ErrorResponse),
}

/// Status
#[async_trait]
#[allow(clippy::ptr_arg)]
pub trait Status<E: std::fmt::Debug + Send + Sync + 'static = ()>: super::ErrorHandler<E> {
    /// Service status.
    ///
    /// GetStatus - GET /status
    async fn get_status(
        &self,

        method: &Method,
        host: &Host,
        cookies: &CookieJar,
    ) -> Result<GetStatusResponse, E>;
}
