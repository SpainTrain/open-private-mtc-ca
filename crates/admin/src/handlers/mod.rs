//! Admin-API operation handlers (spec §17.5 step 3).
//!
//! One module per operation group, each implementing the matching generated
//! `mtc_admin_api_server::apis::*` trait(s) against
//! [`crate::state::AppState`].

pub mod health;
