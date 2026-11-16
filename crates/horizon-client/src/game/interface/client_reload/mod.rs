//! Prepare client listeners against the new pack selection before publishing
//! any assets, and apply them on the thread that owns the game. The initial
//! load runs the same three steps: the loader thread registers and stages,
//! and the game applies once it exists (`start_game`).
use mod_api::__private::game::*;

mod runtime_sprites;
#[cfg(test)]
pub(in crate::game) use runtime_sprites::fixture_png;
pub(in crate::game) use runtime_sprites::{collect_runtime_sprites, stage_runtime_sprites};
mod texture_atlases;
pub(in crate::game) use texture_atlases::{collect_texture_atlases, stage_texture_atlases};

#[cfg(test)]
#[path = "client_reload_tests.rs"]
mod tests;
use super::Game;
use super::packs::reload_files;
use horizon_client::datapack::DataPack;
use mod_api::{
    __private::reload::{PreparedStep, ReloadStageError, apply_listeners, prepare_listeners},
    ClientHost, ClientReloadContext, ClientReloadListener, HookId, Host,
};

/// One client reload step to run later on the game-owning thread: a mod
/// listener's prepared apply.
pub(super) type PreparedClientReload = PreparedStep<()>;

/// The registration host of the first load: no game exists yet, so it reads
/// nothing — a host read during registration panics, as its contract forbids
/// one. The reload itself reads through the game's host.
pub(super) struct StartupClientHost;
impl Host for StartupClientHost {}
mod_api::impl_host_areas!(StartupClientHost);
impl ClientHost for StartupClientHost {}
mod_api::impl_client_host_areas!(StartupClientHost);

impl Game<'_> {
    /// Registers a later reload's client listeners through the game
    /// (`native_add_client_reload_listeners`), gated on the hook being
    /// listened to.
    pub(super) fn register_client_reload_listeners(&self) -> mod_api::ClientListenerRegistry {
        let _reload = self
            .mod_phase
            .reloading()
            .expect("client listener registration on its owning thread");
        let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
        if mod_api::__private::listened(HookId::AddClientReloadListeners) {
            self.native_add_client_reload_listeners(&mut listeners);
        }
        listeners
    }
}

/// Registers the first load's client listeners: no game exists yet, so
/// [`StartupClientHost`] stands in, before any resources are loaded.
pub(super) fn register_startup_client_reload_listeners() -> mod_api::ClientListenerRegistry {
    let phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Setup);
    let _reload = phase.reloading().expect("initial client registration");
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    if mod_api::__private::listened(HookId::AddClientReloadListeners) {
        mod_api::dispatch_add_client_reload_listeners(&StartupClientHost, &mut listeners);
    }
    listeners
}

/// Orders the registered listeners, then prepares each against `root`, the
/// merged resource root the reload will publish; runs on the reload's worker,
/// off the game-owning thread.
pub(super) fn stage_client_reload(
    root: &horizon_assets::resource_source::ResourceSource,
    listeners: mod_api::ClientListenerRegistry,
    entering: mod_api::__private::ModPhase,
) -> Result<Vec<PreparedClientReload>, ReloadStageError> {
    let phase = mod_api::__private::PhaseOwner::scope(entering);
    let _reload = phase.reloading().expect("client listener prepare");
    let pack = DataPack::from_source("vanilla".into(), root.clone());
    let ordered = listeners.finish().map_err(ReloadStageError::Order)?;
    prepare_listeners(
        ordered,
        "client_reload_prepare",
        |_| None,
        |id, converter| ClientReloadContext::new(reload_files("assets", &[&pack], id, converter)),
        ClientReloadListener::prepare,
    )
}

/// Applies the prepared listeners in order on the game-owning thread, after
/// the rest of the staged pack is published and before the
/// resources-reloaded notice.
pub(super) fn apply_client_reload(
    phase: &mod_api::__private::PhaseOwner,
    prepared: Vec<PreparedClientReload>,
) {
    let _reload = phase
        .reloading()
        .expect("client apply on its owning thread");
    apply_listeners(prepared, "client_reload_apply", |()| {});
}
