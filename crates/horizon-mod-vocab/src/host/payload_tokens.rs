//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The typed payload token check both sides share, reached through [`Host`]: what
/// [`ToServer::send`] and [`ToClient::send`] run before they encode.
#[horizon_mod_api_macros::host_area]
pub trait PayloadTokensHost {
    /// Side: both.
    /// Whether `token` is one this load's registrar handed out: the frozen declaration at its
    /// slot has its id, protocol, direction and schema, and its owning mod is on. Anything
    /// else (forged, from another load, of another type) is `StaleToken`, and nothing is sent.
    #[access(Metadata)]
    fn check_payload_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError>;
}
