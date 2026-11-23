//! The mods' commands as the session holds them (`register_commands`). As on the reference
//! loader, the mods are asked whenever the session's command tree is built: when a world opens,
//! before its functions load, and on each `/reload`. The tree lives on `Game`, so a function
//! line naming a mod command parses with no hub; a hub, once there is one, takes that same tree.

use super::{CommandArguments, Game, command_arguments, command_tree, register_dispatcher};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

/// The commands the mods registered, as the session's tree holds them.
pub(in crate::game) struct NativeCommands {
    /// Each root's traversal level.
    pub(super) levels: HashMap<String, u8>,
    /// Each root's text for a player below its level (`Command::denied_message`).
    pub(super) denied: HashMap<String, String>,
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

    /// The text a player below the level of `line`'s root is answered with, when a usage of that
    /// root set one (`Command::denied_message`); `None` keeps vanilla's unknown-command answer.
    pub(in crate::game) fn denied(&self, line: &str) -> Option<&str> {
        let name = line.split(' ').next().unwrap_or_default();
        self.registered
            .as_ref()?
            .denied
            .get(name)
            .map(String::as_str)
    }

    /// The denied text for a player of `permission` typing `line`: only when the player is below
    /// the root's traversal level, which is its lowest usage level. A player who reaches the root
    /// is refused further in by the dispatcher, with vanilla's answer.
    pub(in crate::game) fn denied_for(&self, line: &str, permission: u8) -> Option<&str> {
        if permission >= self.level(line)? {
            return None;
        }
        self.denied(line)
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
            denied: denied_texts(&usages),
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
/// The root of each usage that sets a denied text: the first usage of a root that sets one
/// answers for it, so a usage's own level still decides who runs the command.
fn denied_texts(usages: &[mod_api::Command]) -> HashMap<String, String> {
    let mut texts = HashMap::new();
    for usage in usages {
        let Some(text) = &usage.denied else {
            continue;
        };
        let root = usage.usage.split(' ').next().unwrap_or_default();
        texts.entry(root.to_owned()).or_insert_with(|| text.clone());
    }
    texts
}

impl Game<'_> {
    /// `Commands::command_info`: the command `label` names, a leading slash dropped. A native
    /// mod's registered usage first, else vanilla's root command in the session's tree (the mods'
    /// nodes are there too, but a mod's usage was already tried).
    pub(in crate::game) fn native_command_info(&self, label: &str) -> Option<mod_api::CommandInfo> {
        let label = label.strip_prefix('/').unwrap_or(label);
        let empty = HashMap::new();
        let (usages, levels) = match &self.native_commands.registered {
            Some(commands) => (commands.usages.as_slice(), &commands.levels),
            None => (&[][..], &empty),
        };
        mod_command_info(usages, levels, label)
            .or_else(|| vanilla_command_info(&self.ui.dispatcher.0, label))
    }

    /// `Commands::command_names`: the root names of the mods' usages and of vanilla's roots
    /// (aliases included), sorted, each once.
    pub(in crate::game) fn native_command_names(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        if let Some(commands) = &self.native_commands.registered {
            // Only the roots the tree registered; a usage `command_tree` left out is not one.
            names.extend(commands.levels.keys().cloned());
        }
        let dispatcher = &self.ui.dispatcher.0;
        names.extend(
            dispatcher.nodes[0]
                .children
                .iter()
                .map(|&child| dispatcher.nodes[child].name().to_owned()),
        );
        names.into_iter().collect()
    }
}

/// The native mods' command whose root is `label`: its usages in registration order, and its
/// level from `levels` (the registered roots, as `command_tree` returned them). None when
/// `label` is not a registered root, so a usage the tree left out never answers.
fn mod_command_info(
    usages: &[mod_api::Command],
    levels: &HashMap<String, u8>,
    label: &str,
) -> Option<mod_api::CommandInfo> {
    let level = *levels.get(label)?;
    let mine: Vec<&mod_api::Command> = usages
        .iter()
        .filter(|command| command.usage.split_whitespace().next() == Some(label))
        .collect();
    Some(mod_api::CommandInfo::new(
        label.to_owned(),
        Vec::new(),
        mine.iter().map(|command| command.usage.clone()).collect(),
        level,
        false,
    ))
}

/// Vanilla's root command `label` names in `d`, an alias (a root literal that only redirects to
/// another root command, `tp`) resolved to its target. Its aliases are the roots redirecting to it.
fn vanilla_command_info(
    d: &horizon_client::command::Dispatcher,
    label: &str,
) -> Option<mod_api::CommandInfo> {
    let roots = &d.nodes[0].children;
    let mut node = d.find(label)?;
    if let Some(target) = d.nodes[node]
        .redirect
        .filter(|target| roots.contains(target))
    {
        node = target;
    }
    let aliases = roots
        .iter()
        .filter(|&&root| root != node && d.nodes[root].redirect == Some(node))
        .map(|&root| d.nodes[root].name().to_owned())
        .collect();
    let usages = d
        .smart_usage(0, 4)
        .into_iter()
        .filter(|(root, _)| *root == node)
        .map(|(_, usage)| usage)
        .collect();
    Some(mod_api::CommandInfo::new(
        d.nodes[node].name().to_owned(),
        aliases,
        usages,
        d.nodes[node].permission,
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_shows_only_when_the_whole_root_is_above_the_player() {
        let state = NativeCommandState {
            registered: Some(NativeCommands {
                levels: HashMap::from([("music".to_owned(), 0), ("mod".to_owned(), 2)]),
                denied: HashMap::from([
                    ("music".to_owned(), "No.".to_owned()),
                    ("mod".to_owned(), "Staff only.".to_owned()),
                ]),
                tree: Value::Null,
                arguments: CommandArguments::default(),
                usages: Vec::new(),
            }),
            ..NativeCommandState::default()
        };
        // A root with a public and a protected usage has level 0: a level-0 guest typing
        // `music stop` is refused by the dispatcher, not by the text.
        assert_eq!(state.denied_for("music stop", 0), None);
        assert_eq!(state.denied_for("mod ban", 1), Some("Staff only."));
        assert_eq!(state.denied_for("mod ban", 2), None);
        assert_eq!(state.denied_for("unknown", 0), None);
    }

    #[test]
    fn the_first_denied_text_of_a_root_answers_for_it() {
        let usages = [
            mod_api::Command::op("music <name:word>", 2),
            mod_api::Command::op("music stop", 2).denied_message("Ask a moderator."),
            mod_api::Command::new("music").denied_message("Not for you."),
            mod_api::Command::new("other"),
        ];
        let texts = denied_texts(&usages);
        assert_eq!(texts.len(), 1);
        assert_eq!(
            texts.get("music").map(String::as_str),
            Some("Ask a moderator.")
        );
        assert_eq!(usages[0].denied, None);
    }

    #[test]
    fn a_mod_command_answers_with_its_usages_and_the_lowest_level() {
        let usages = [
            mod_api::Command::new("warp <name:word>"),
            mod_api::Command::op("warp set <name:word>", 2),
            mod_api::Command::op("warpadmin", 3),
        ];
        let levels = HashMap::from([("warp".to_owned(), 0), ("warpadmin".to_owned(), 3)]);
        let info = mod_command_info(&usages, &levels, "warp").unwrap();
        assert_eq!(info.name, "warp");
        assert_eq!(info.usages, ["warp <name:word>", "warp set <name:word>"]);
        assert_eq!(info.level, 0);
        assert!(info.aliases.is_empty());
        assert!(!info.vanilla);
        assert_eq!(
            mod_command_info(&usages, &levels, "warpadmin")
                .unwrap()
                .level,
            3
        );
        assert_eq!(mod_command_info(&usages, &levels, "missing"), None);
    }

    #[test]
    fn a_usage_the_tree_left_out_is_not_answered() {
        let usages = [
            mod_api::Command::new("foo <x:bogus>"),
            mod_api::Command::new("give <x:word>"),
        ];
        let arguments = command_arguments();
        let (_, levels, problems) = command_tree(&usages, |name| name == "give", &arguments);
        assert_eq!(problems.len(), 2);
        assert_eq!(mod_command_info(&usages, &levels, "foo"), None);
        assert_eq!(mod_command_info(&usages, &levels, "give"), None);
    }

    #[test]
    fn a_vanilla_alias_resolves_to_its_target_with_the_alias_named() {
        let mut d = horizon_client::command::Dispatcher::new();
        let teleport = d.literal(0, "teleport");
        d.requires(teleport, 2);
        let alias = d.literal(0, "tp");
        d.redirect(alias, teleport);
        let info = vanilla_command_info(&d, "tp").unwrap();
        assert_eq!(info.name, "teleport");
        assert_eq!(info.aliases, ["tp"]);
        assert_eq!(info.level, 2);
        assert!(info.vanilla);
        assert_eq!(
            vanilla_command_info(&d, "teleport").unwrap().aliases,
            ["tp"]
        );
        assert_eq!(vanilla_command_info(&d, "missing"), None);
    }
}
