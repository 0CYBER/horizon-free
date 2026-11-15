//! The host's typed payload token check: what every typed send and reply runs first.
use super::Game;
use mod_api::{PayloadSendError, PayloadToken};

impl Game<'_> {
    /// Check `token` against this load's frozen typed declarations.
    pub(super) fn check_payload_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError> {
        mod_api::__private::game::check_payload_token(token)
    }
}
