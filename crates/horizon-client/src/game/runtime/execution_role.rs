//! A Game's execution role is fixed at construction. Connecting or disconnecting
//! changes `remote`, not this role. A client never simulates a world itself: single
//! player's runs on its integrated server thread.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExecutionRole {
    Client,
    IntegratedServer,
    DedicatedServer,
}

impl ExecutionRole {
    /// Only a client owns presentation work (audio, loading UI, chunk meshes).
    pub fn is_client(self) -> bool {
        self == Self::Client
    }

    /// Select the phase once at an owning data-load boundary.
    pub fn data_load_origin(self) -> horizon_world::native_load_context::DataLoadOrigin {
        use horizon_world::native_load_context::DataLoadOrigin;
        match self {
            Self::Client => DataLoadOrigin::ClientPreload,
            Self::IntegratedServer | Self::DedicatedServer => DataLoadOrigin::ServerDataLoad,
        }
    }

    /// The side a menu, recipe or player-list loop of this game's runs on.
    pub fn loop_side(self) -> horizon_world::native_load_context::LoopSide {
        use horizon_world::native_load_context::LoopSide;
        if self.is_logical_server() {
            LoopSide::Server
        } else {
            LoopSide::Client
        }
    }

    /// A dedicated server: the logical server with no local player, whose console,
    /// event stream and user lists exist. The integrated server thread keeps
    /// `options.dedicated` set too, so only the role tells the two apart.
    pub fn is_dedicated_server(self) -> bool {
        self == Self::DedicatedServer
    }

    /// Single-player worlds (on the integrated server thread) keep the owner's state in
    /// `Level` and write `players/data/<singleplayer_uuid>.dat` with the world; a dedicated
    /// server saves each connected player separately.
    pub fn writes_singleplayer_player(self) -> bool {
        self != Self::DedicatedServer
    }

    /// A logical server simulates the world: the integrated server thread or a
    /// dedicated server. A client — connected, or at a screen with no world —
    /// does not, whatever `remote` happens to say.
    pub fn is_logical_server(self) -> bool {
        self != Self::Client
    }
}

use super::Game;
use mod_api::HostSide;

/// The hook host of `game`'s side.
pub(super) fn host_side<'a>(game: &'a mut Game<'_>) -> HostSide<'a> {
    if game.is_logical_server() {
        HostSide::Server(game)
    } else {
        HostSide::Client(game)
    }
}

impl Game<'_> {
    /// This game's side: a server simulates the world, a client does not. The
    /// one source for a helper's side; `remote` says whether a client is
    /// connected, not which side this game is.
    pub(super) fn is_logical_server(&self) -> bool {
        self.options.execution_role.is_logical_server()
    }

    /// Whether this game is a dedicated server, not the integrated server thread (which
    /// keeps `options.dedicated` set too).
    pub(super) fn is_dedicated_server(&self) -> bool {
        self.options.execution_role.is_dedicated_server()
    }
}
