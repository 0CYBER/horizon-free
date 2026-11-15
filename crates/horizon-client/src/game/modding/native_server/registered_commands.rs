//! The mods' commands as the session holds them (`register_commands`). As on the reference
//! loader, the mods are asked whenever the session's command tree is built: when a world opens,
//! before its functions load, and on each `/reload`. The tree lives on `Game`, so a function
//! line naming a mod command parses with no hub; a hub, once there is one, takes that same tree.

use super::{CommandArguments, Game, command_arguments, command_tree, register_dispatcher};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

/// The commands the mods registered, as the session's tree holds them.
pub(in crate::game) struct NativeCommands {
    /// Each root's traversal level.
    pub(super) levels: HashMap<String, u8>,
    /// The mods' tree, without Horizon's moderation commands.
    pub(super) tree: Value,
    pub(super) arguments: CommandArguments,
    /// The usages the mods registered, for the hub's tree with the moderation commands.
    usages: Vec<mod_api::Command>,
}

/// The session's state of the mods' commands.
#[derive(Default)]
pub(in crate::game) struct NativeCommandState {
    /// A mod asked for its commands again (`refresh_commands`): done at the next server tick.
    pub(in crate::game) refresh: bool,
    pub(in crate::game) registered: Option<NativeCommands>,
    /// Bumped each time the mods are asked; a hub holds the one it was given.
    generation: u64,
}

impl NativeCommandState {
    /// The traversal level of a mod's root (`line`, without its slash), when registered.
    /// The parsed executable node enforces its own usage level separately.
    pub(in crate::game) fn level(&self, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        self.registered.as_ref()?.levels.get(name).copied()
    }
}

impl Game<'_> {
    /// `Commands`' constructor with `RegisterCommandsEvent`: asks the mods for their commands and
    /// builds the session's tree, vanilla's for `features` with the mods' on top. Called before
    /// functions are admitted (world open and each reload), so both function checks see them.
    pub(in crate::game) fn build_native_commands(&mut self, features: &[String]) {
        // A server hook: a client's tree is the one its server sends.
        if !self.is_logical_server() {
            return;
        }
        let mut usages = Vec::new();
        self.native_register_commands(&mut usages);
        let vanilla = horizon_client::command::vanilla::session_dispatcher(
            &self.level.rule_registry,
            features,
        );
        let arguments = command_arguments();
        let hub = self.lan.as_ref().map(|host| &host.hub);
        let (tree, levels, problems) = command_tree(
            &usages,
            |name| hub.is_some_and(|hub| hub.vanilla_command(name)) || vanilla.find(name).is_some(),
            &arguments,
        );
        for p in problems {
            log::warn!("Horizon: a native mod's command was left out: {p}");
        }
        let state = &mut self.native_commands;
        state.generation = state.generation.wrapping_add(1);
        state.registered = Some(NativeCommands {
            levels,
            tree,
            arguments,
            usages,
        });
        self.ui.dispatcher.0 = Arc::new(vanilla);
        self.restore_native_commands();
    }

    /// Gives the hub the session's mod commands, with Horizon's moderation commands, when it
    /// lacks them; first asks the mods again when one asked (`native_refresh_commands`), and then
    /// sends every player their tree.
    pub(in crate::game) fn register_native_commands(&mut self) {
        let refresh = std::mem::take(&mut self.native_commands.refresh);
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        if refresh || self.native_commands.registered.is_none() {
            self.build_native_commands(&self.level.enabled_features.clone());
        } else if host.native_commands_sent == Some(self.native_commands.generation) {
            return;
        }
        let (Some(commands), Some(host)) = (&self.native_commands.registered, self.lan.as_mut())
        else {
            return;
        };
        // Horizon's own moderation commands go in the same tree, so operators' clients complete
        // them instead of showing them as unknown; the game still runs them itself
        // (`run_guest_command`). A mod's command of the same name wins.
        let extras = super::super::tempban::moderation_commands(
            host.hub.chat_filter_on(),
            self.server_lists.is_some(),
        );
        let usages = [commands.usages.as_slice(), &extras].concat();
        let (tree, _, _) = command_tree(
            &usages,
            |name| host.hub.vanilla_command(name),
            &commands.arguments,
        );
        let replacing = host.native_commands_sent.is_some();
        host.native_commands_sent = Some(self.native_commands.generation);
        host.hub
            .set_selector_tokens(super::super::native_entity_selectors::selector_tokens());
        if refresh || replacing {
            host.hub.store_mod_commands(0, tree);
            self.resend_commands_filtered();
        } else {
            host.hub.set_mod_commands(0, tree);
        }
    }

    /// Restore mod nodes after vanilla rebuilds the session's command tree.
    pub(in crate::game) fn restore_native_commands(&mut self) {
        if let Some(commands) = &self.native_commands.registered {
            register_dispatcher(
                Arc::make_mut(&mut self.ui.dispatcher.0),
                &commands.tree,
                "native_mod_command",
                &commands.arguments,
            );
        }
    }
}
