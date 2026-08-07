//! [`BackendError`]: failures building a [`Backend`](crate::Backend) from a
//! [`BackendConfig`](crate::BackendConfig) (spec §9.4).

use crate::config::Provider;

/// Failure building a [`Backend`](crate::Backend) (rule
/// `thiserror-for-libs-eyre-for-bins`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// `cfg.provider` names a backend [`build_backend`](crate::build_backend)
    /// does not implement yet.
    ///
    /// [`Provider::Aws`] and [`Provider::Localstack`] parse out of
    /// [`BackendConfig`](crate::BackendConfig) successfully -- config
    /// validation does not depend on which providers happen to be wired --
    /// but `build_backend` cannot yet produce a working
    /// [`Backend`](crate::Backend) for them. `cloud-backend-factory-aws`
    /// fills in both arms.
    #[error("provider {provider} is not implemented yet (lands in cloud-backend-factory-aws)")]
    Unimplemented {
        /// The provider `build_backend` could not wire up.
        provider: Provider,
    },
}
