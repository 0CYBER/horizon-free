//! The game's side of the host traits: `horizon-mod-api/src/host/<area>.rs` declares each
//! area's methods and access levels. Build-time generated impls forward to the inherent
//! `Game` functions with the same names. `Host`, `ServerHost` and `ClientHost` are the unions of those traits, so
//! a mod reaches every area through them and the handles.
// The generated impls convert error types that are already equal; TODO(arismoko/horizon#2273): make the generator skip it.
#![allow(clippy::useless_conversion, clippy::unit_arg)]
use super::Game;
use mod_api::__private::game::*;
use mod_api::*;

include!(concat!(env!("OUT_DIR"), "/game_host_modules.rs"));

impl Host for Game<'_> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}

impl ServerHost for Game<'_> {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(BackendMutRef::of(self))
    }
}

impl ClientHost for Game<'_> {
    fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {
        Some(ClientBackendMutRef::of(self))
    }
}

impl Backend for Game<'_> {}
impl BackendMut for Game<'_> {}
impl ClientBackendMut for Game<'_> {}
