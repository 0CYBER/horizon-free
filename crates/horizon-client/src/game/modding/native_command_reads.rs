//! What a server mod reads beyond a command's arguments (NeoForge's
//! `ICommandSourceStackExtension`): where the command executes, a player's game profile and a
//! loaded advancement.
use super::Game;
use super::api_values::TryIntoApi;
use horizon_client::advancements::{Advancement, Frame, Tree};
use mod_api::{
    AdvancementDisplay, AdvancementFrame, AdvancementInfo, CommandArgument, CommandInvocation,
    GameProfile, Identifier, PlayerRef, ProfileProperty,
};

impl Game<'_> {
    /// A parsed native command with the level and position it executes at: after `execute
    /// in/at/positioned`, or the console's overworld spawn corner.
    pub(super) fn native_invocation(
        &self,
        literals: Vec<String>,
        arguments: Vec<CommandArgument>,
    ) -> CommandInvocation {
        CommandInvocation::new(
            literals,
            arguments,
            self.native_level_key(),
            self.source_position(),
        )
    }

    /// `ServerPlayer.getGameProfile`: the name `player_name` reads, and the properties the
    /// player logged in with. None for a stale or unavailable player.
    pub(super) fn player_profile(&self, player: PlayerRef) -> Option<GameProfile> {
        let name = self.player_name(player)?;
        let properties = self
            .lan
            .as_ref()
            .and_then(|host| {
                host.hub
                    .online_players()
                    .find(|guest| guest.uuid == player.uuid())
            })
            .map(|guest| {
                guest
                    .properties
                    .iter()
                    .map(|p| {
                        ProfileProperty::new(p.name.clone(), p.value.clone(), p.signature.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(GameProfile::new(player.uuid(), name, properties))
    }

    /// `ServerAdvancementManager.get(id)` over the loaded tree, read once on first use and
    /// replaced by `/reload`.
    pub(super) fn advancement(&mut self, id: &Identifier) -> Option<AdvancementInfo> {
        advancement_in(&self.advancement_tree(), id)
    }
}

/// The advancement `id` in `tree`; None when the tree has no such advancement.
fn advancement_in(tree: &Tree, id: &Identifier) -> Option<AdvancementInfo> {
    advancement_info(&tree.list[*tree.by_id.get(&id.to_string())?])
}

/// A loaded advancement as the API's snapshot. None when its id, parent, texts or icon don't
/// cross, which a tree built from valid files never holds.
fn advancement_info(advancement: &Advancement) -> Option<AdvancementInfo> {
    let display = match &advancement.display {
        Some(d) => Some(AdvancementDisplay::new(
            horizon_client::scoreboard::comp_text(&d.title)
                .try_into_api()
                .ok()?,
            horizon_client::scoreboard::comp_text(&d.description)
                .try_into_api()
                .ok()?,
            d.icon.try_into_api().ok()?,
            match d.frame {
                Frame::Task => AdvancementFrame::Task,
                Frame::Goal => AdvancementFrame::Goal,
                Frame::Challenge => AdvancementFrame::Challenge,
            },
            d.show_toast,
            d.announce,
            d.hidden,
        )),
        None => None,
    };
    let parent = match &advancement.parent {
        Some(parent) => Some(Identifier::parse(parent).ok()?),
        None => None,
    };
    Some(AdvancementInfo::new(
        Identifier::parse(&advancement.id).ok()?,
        parent,
        display,
        advancement
            .criteria
            .iter()
            .map(|c| c.name.clone())
            .collect(),
        advancement.requirements.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::advancements::{Criterion, Display, Rewards};

    fn mine_stone() -> Advancement {
        let criterion = |name: &str| Criterion {
            name: name.into(),
            trigger: "minecraft:inventory_changed".into(),
            conditions: horizon_world::lazy_json::LazyJson::new(&serde_json::json!({})),
        };
        let mut icon = horizon_client::crafting::recipe_result(&serde_json::json!({
            "id": "minecraft:wooden_pickaxe",
            "components": {"minecraft:enchantment_glint_override": true}
        }))
        .unwrap();
        icon.count = 1;
        Advancement {
            id: "minecraft:story/mine_stone".into(),
            parent: Some("minecraft:story/root".into()),
            display: Some(Display {
                icon,
                title: serde_json::json!({"translate": "advancements.story.mine_stone.title"}),
                description: serde_json::json!("Mine Stone"),
                frame: Frame::Task,
                background: None,
                show_toast: true,
                announce: true,
                hidden: false,
            }),
            criteria: vec![criterion("get_stone"), criterion("get_cobbled_deepslate")],
            requirements: vec![vec!["get_stone".into(), "get_cobbled_deepslate".into()]],
            rewards: Rewards::default(),
        }
    }

    #[test]
    fn a_loaded_advancement_crosses_with_its_parent_frame_icon_and_criteria() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:wooden_pickaxe"]).unwrap();
        let info = advancement_info(&mine_stone()).unwrap();
        assert_eq!(
            info.id,
            Identifier::parse("minecraft:story/mine_stone").unwrap()
        );
        assert_eq!(
            info.parent,
            Some(Identifier::parse("minecraft:story/root").unwrap())
        );
        assert_eq!(info.criteria, ["get_stone", "get_cobbled_deepslate"]);
        assert_eq!(
            info.requirements,
            vec![vec![
                "get_stone".to_owned(),
                "get_cobbled_deepslate".to_owned()
            ]]
        );
        let display = info.display.unwrap();
        assert_eq!(display.frame, AdvancementFrame::Task);
        assert_eq!(
            display.icon.item(),
            mod_api::__private::item_key("minecraft:wooden_pickaxe").unwrap()
        );
        let components = display
            .icon
            .components()
            .expect("the icon keeps its components");
        assert!(
            components
                .as_nbt()
                .keys()
                .any(|key| key.to_string_lossy() == "minecraft:enchantment_glint_override"),
            "{components:?}"
        );
    }

    #[test]
    fn an_unknown_advancement_reads_as_none() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:wooden_pickaxe"]).unwrap();
        let mut tree = Tree::default();
        tree.by_id.insert("minecraft:story/mine_stone".into(), 0);
        tree.list.push(mine_stone());
        let known = Identifier::parse("minecraft:story/mine_stone").unwrap();
        assert_eq!(advancement_in(&tree, &known).map(|a| a.id), Some(known));
        for unknown in ["minecraft:story/no_such", "mymod:story/mine_stone"] {
            assert_eq!(
                advancement_in(&tree, &Identifier::parse(unknown).unwrap()),
                None,
                "{unknown}"
            );
        }
    }

    /// The issue's case: a console `execute in minecraft:the_nether` reaches the mod with the
    /// nether as its level; without it, the console's overworld and spawn corner.
    #[test]
    #[ignore = "daily"]
    fn a_console_command_executes_in_the_level_execute_in_names() {
        use std::sync::{Arc, Mutex};
        type Heard = Arc<Mutex<Vec<(mod_api::CommandSource, CommandInvocation)>>>;
        struct Where(Heard);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Where {
            fn register_commands(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RegisterCommandsEvent<'_>,
            ) {
                event.commands.extend([mod_api::Command::new("where")]);
            }
            fn run_mod_command(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::RunModCommandEvent<'_>,
            ) -> mod_api::Flow {
                (self.0.lock().unwrap()).push((event.source.clone(), event.command.clone()));
                mod_api::Flow::Cancel
            }
        }
        let heard: Heard = Arc::default();
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "where",
                Box::new(Where(heard.clone())),
            )]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    game.build_registries();
                    game.register_native_commands();
                    assert!(game.native_run_command("where"));
                    assert!(game.native_run_command("execute in minecraft:the_nether run where"));
                    let heard = heard.lock().unwrap();
                    assert_eq!(heard.len(), 2);
                    for (source, _) in heard.iter() {
                        assert!(
                            matches!(source, mod_api::CommandSource::Console { .. }),
                            "{source:?}"
                        );
                    }
                    assert_eq!(heard[0].1.dimension, mod_api::DimensionKey::OVERWORLD);
                    assert_eq!(heard[0].1.position, game.server_position());
                    assert_eq!(heard[1].1.dimension, mod_api::DimensionKey::NETHER);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_profile_keeps_its_login_properties() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let stored = horizon_protocol::packets::login::Property {
                name: "textures".into(),
                value: "e30=".into(),
                signature: Some("sig".into()),
            };
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.properties = vec![stored])
                .unwrap();
            let player = game.player(12).unwrap();
            assert_eq!(
                game.player_profile(player),
                Some(GameProfile::new(
                    12,
                    "guest12".into(),
                    vec![ProfileProperty::new(
                        "textures".into(),
                        "e30=".into(),
                        Some("sig".into())
                    )],
                ))
            );
        });
    }
}
