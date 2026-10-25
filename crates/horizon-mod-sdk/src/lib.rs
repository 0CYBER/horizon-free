//! Write Horizon mods in Rust (`docs/mods.md`).
//!
//! A mod is a type implementing [`Mod`], exported with [`export_mod!`] from
//! a `cdylib` crate built for `wasm32-unknown-unknown`;
//! `cargo horizon-mod build` turns it into the component the game loads.
//!
//! ```ignore
//! use horizon_mod_sdk::{Mod, Registration, chat};
//!
//! struct Hello;
//! impl Mod for Hello {
//!     fn init() -> Result<(Self, Registration), String> {
//!         Ok((Hello, Registration::new().command("hello")))
//!     }
//!     fn command(&mut self, _name: &str, _args: &str) -> Result<Option<String>, String> {
//!         chat::show("Hello from a mod!");
//!         Ok(None)
//!     }
//! }
//! horizon_mod_sdk::export_mod!(Hello);
//! ```
//!
//! Every interface but `log` is a capability: list the ones the mod uses in
//! `horizon-mod.toml` (`game-read`, `chat`, `hud`, `storage`; a server mod
//! `storage` and `players`). Server mods: [`server`].

wit_bindgen::generate!({
    path: "wit",
    world: "horizon-mod",
    pub_export_macro: true,
    export_macro_name: "__export_horizon_mod",
    default_bindings_module: "horizon_mod_sdk",
});

pub use exports::horizon::mods::events::{Guest as __Guest, Registration};
pub use horizon::mods::{chat, game, hud, log, storage};

/// The game's calls into a mod. Only the hooks [`Registration`] asks for
/// are called; the others may stay as their defaults.
pub trait Mod: Sized + 'static {
    /// Once, after loading: the mod's state and the hooks it wants. An error
    /// keeps the mod off and is shown to the player.
    fn init() -> Result<(Self, Registration), String>;
    /// Every game tick, when [`Registration::tick`] is set.
    fn tick(&mut self, _tick: u64) {}
    /// Every rendered frame, when [`Registration::frame`] is set.
    fn frame(&mut self, _seconds: f32) {}
    /// `/name args` for a command the mod registered. `Ok(Some(text))` is
    /// shown as feedback, `Err(text)` in red.
    fn command(&mut self, _name: &str, _args: &str) -> Result<Option<String>, String> {
        Err("This command does nothing".into())
    }
    /// A chat line the player received, when [`Registration::chat`] is set.
    fn chat(&mut self, _text: &str) {}
}

impl Registration {
    /// No commands and no hooks.
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            tick: false,
            frame: false,
            chat: false,
        }
    }
    /// Adds `/name`.
    pub fn command(mut self, name: &str) -> Self {
        self.commands.push(name.into());
        self
    }
    pub fn on_tick(mut self) -> Self {
        self.tick = true;
        self
    }
    pub fn on_frame(mut self) -> Self {
        self.frame = true;
        self
    }
    pub fn on_chat(mut self) -> Self {
        self.chat = true;
        self
    }
}
impl Default for Registration {
    fn default() -> Self {
        Self::new()
    }
}

/// `log::write` at info level.
pub fn info(message: &str) {
    log::write(log::Level::Info, message);
}
/// `log::write` at warn level.
pub fn warn(message: &str) {
    log::write(log::Level::Warn, message);
}

/// Exports a [`Mod`] as the component's `events` interface. The state lives
/// in a thread local: a mod instance is single-threaded.
#[macro_export]
macro_rules! export_mod {
    ($t:ty) => {
        const _: () = {
            use ::std::cell::RefCell;
            ::std::thread_local! {
                static STATE: RefCell<Option<$t>> = const { RefCell::new(None) };
            }
            fn with(f: impl FnOnce(&mut $t)) {
                STATE.with(|s| {
                    if let Some(m) = s.borrow_mut().as_mut() {
                        f(m)
                    }
                })
            }
            struct Export;
            impl $crate::__Guest for Export {
                fn init() -> Result<$crate::Registration, String> {
                    let (m, registration) = <$t as $crate::Mod>::init()?;
                    STATE.with(|s| *s.borrow_mut() = Some(m));
                    Ok(registration)
                }
                fn tick(tick: u64) {
                    with(|m| $crate::Mod::tick(m, tick))
                }
                fn frame(seconds: f32) {
                    with(|m| $crate::Mod::frame(m, seconds))
                }
                fn command(name: String, args: String) -> Result<Option<String>, String> {
                    let mut out = Err("The mod is not running".to_string());
                    with(|m| out = $crate::Mod::command(m, &name, &args));
                    out
                }
                fn chat(text: String) {
                    with(|m| $crate::Mod::chat(m, &text))
                }
            }
            // Only WebAssembly has the component's exports: a host build (cargo test or
            // clippy on the workspace) checks the mod without them, which macOS
            // couldn't link (its exported-symbols list cuts names at the `#`).
            #[cfg(target_arch = "wasm32")]
            $crate::__export_horizon_mod!(Export);
            #[cfg(not(target_arch = "wasm32"))]
            let _ = <Export as $crate::__Guest>::init;
        };
    };
}

/// Server mods (manifest `side = "server"`, `api = "0.3"`, experimental): a
/// type implementing [`ServerMod`](server::ServerMod), exported with
/// [`export_server_mod!`]. It runs where the world is simulated, and reads
/// and messages the players with capability `players`.
pub mod server {
    wit_bindgen::generate!({
        path: "wit",
        world: "horizon-server-mod",
        pub_export_macro: true,
        export_macro_name: "__export_horizon_server_mod",
        default_bindings_module: "horizon_mod_sdk::server",
        with: {
            "horizon:mods/log@0.1.0": crate::log,
            "horizon:mods/storage@0.1.0": crate::storage,
        },
    });

    pub use exports::horizon::mods::server_events::{BlockPos, Guest as __Guest, Registration};
    pub use horizon::mods::players::{self, GameMode, Player};

    /// The server's calls into a mod. Only the hooks [`Registration`] asks
    /// for are called; the others may stay as their defaults.
    pub trait ServerMod: Sized + 'static {
        /// Once, after loading: the mod's state and the hooks it wants. An
        /// error keeps the mod off and is logged.
        fn init() -> Result<(Self, Registration), String>;
        /// Every server tick, when [`Registration::tick`] is set.
        fn tick(&mut self, _tick: u64) {}
        /// When [`Registration::joins`] is set.
        fn player_joined(&mut self, _player: &Player) {}
        fn player_left(&mut self, _player: &Player) {}
        /// A chat message a player sent, when [`Registration::chat`] is set.
        fn chat(&mut self, _player: &Player, _text: &str) {}
        /// `/name args` typed by `player`. `Ok(Some(text))` is their
        /// feedback, `Err(text)` their error in red.
        fn command(
            &mut self,
            _player: &Player,
            _name: &str,
            _args: &str,
        ) -> Result<Option<String>, String> {
            Err("This command does nothing".into())
        }
        /// When [`Registration::blocks`] is set: `state` is what was there.
        fn block_broken(&mut self, _player: &Player, _pos: BlockPos, _state: &str) {}
        fn block_placed(&mut self, _player: &Player, _pos: BlockPos, _state: &str) {}
    }

    impl Registration {
        /// No commands and no hooks.
        pub fn new() -> Self {
            Self {
                commands: Vec::new(),
                tick: false,
                joins: false,
                chat: false,
                blocks: false,
            }
        }
        /// Adds `/name`.
        pub fn command(mut self, name: &str) -> Self {
            self.commands.push(name.into());
            self
        }
        pub fn on_tick(mut self) -> Self {
            self.tick = true;
            self
        }
        pub fn on_joins(mut self) -> Self {
            self.joins = true;
            self
        }
        pub fn on_chat(mut self) -> Self {
            self.chat = true;
            self
        }
        pub fn on_blocks(mut self) -> Self {
            self.blocks = true;
            self
        }
    }
    impl Default for Registration {
        fn default() -> Self {
            Self::new()
        }
    }
}

/// Exports a [`ServerMod`](server::ServerMod) as the component's
/// `server-events` interface, as [`export_mod!`] does for a client mod.
#[macro_export]
macro_rules! export_server_mod {
    ($t:ty) => {
        const _: () = {
            use ::std::cell::RefCell;
            use $crate::server::{BlockPos, Player, Registration, ServerMod};
            ::std::thread_local! {
                static STATE: RefCell<Option<$t>> = const { RefCell::new(None) };
            }
            fn with(f: impl FnOnce(&mut $t)) {
                STATE.with(|s| {
                    if let Some(m) = s.borrow_mut().as_mut() {
                        f(m)
                    }
                })
            }
            struct Export;
            impl $crate::server::__Guest for Export {
                fn init() -> Result<Registration, String> {
                    let (m, registration) = <$t as ServerMod>::init()?;
                    STATE.with(|s| *s.borrow_mut() = Some(m));
                    Ok(registration)
                }
                fn tick(tick: u64) {
                    with(|m| m.tick(tick))
                }
                fn player_joined(player: Player) {
                    with(|m| m.player_joined(&player))
                }
                fn player_left(player: Player) {
                    with(|m| m.player_left(&player))
                }
                fn chat(player: Player, text: String) {
                    with(|m| m.chat(&player, &text))
                }
                fn command(
                    player: Player,
                    name: String,
                    args: String,
                ) -> Result<Option<String>, String> {
                    let mut out = Err("The mod is not running".to_string());
                    with(|m| out = m.command(&player, &name, &args));
                    out
                }
                fn block_broken(player: Player, pos: BlockPos, state: String) {
                    with(|m| m.block_broken(&player, pos, &state))
                }
                fn block_placed(player: Player, pos: BlockPos, state: String) {
                    with(|m| m.block_placed(&player, pos, &state))
                }
            }
            #[cfg(target_arch = "wasm32")]
            $crate::server::__export_horizon_server_mod!(Export);
            #[cfg(not(target_arch = "wasm32"))]
            let _ = <Export as $crate::server::__Guest>::init;
        };
    };
}
