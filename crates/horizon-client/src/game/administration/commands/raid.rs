//! `RaidCommand`: `/raid start|stop|check|spawnleader|setomen|glow|sound`.
use super::{Game, R, Text, err};
use horizon_client::{
    command::{Arg, Context},
    mobs::raider,
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    pub(in crate::game) fn cmd_raid(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let Some(player) = self.player.as_ref().map(|p| p.position) else {
            return Ok(0);
        };
        let at = [
            player.x.floor() as i32,
            player.y.floor() as i32,
            player.z.floor() as i32,
        ];
        let fail = |game: &mut Self, message: &str| {
            game.command_failure(Text::literal(message));
        };
        match id {
            "raid.start" => {
                if self.raiding_nearby(at).is_some() {
                    fail(self, "Raid already started close by");
                    return Ok(-1);
                }
                let level = Self::int(ctx, "omenlvl").unwrap_or(0);
                match self.raid_command_create(at, level) {
                    true => self.feedback(Text::literal("Created a raid in your local village")),
                    false => fail(self, "Failed to create a raid in your local village"),
                }
                Ok(1)
            }
            "raid.stop" => match self.raiding_nearby(at) {
                Some(raid) => {
                    self.raid_command_stop(raid);
                    self.feedback(Text::literal("Stopped raid"));
                    Ok(1)
                }
                None => {
                    fail(self, "No raid here");
                    Ok(-1)
                }
            },
            // RaidCommand.getRaid is getRaidAt: a raid still counting down
            // to its first wave is found too (#11372).
            "raid.check" => match self.raiding_nearby(at) {
                Some(raid) => {
                    self.feedback(Text::literal("Found a started raid!"));
                    let status = self.raid_status_text(raid);
                    self.feedback(Text::literal(&status));
                    Ok(1)
                }
                None => {
                    fail(self, "Found no started raids");
                    Ok(0)
                }
            },
            "raid.spawnleader" => {
                let kind = "minecraft:pillager";
                match self.spawn_mob(
                    kind,
                    [player.x, player.y, player.z],
                    true,
                    super::super::creatures::SpawnSite::Finalizing(
                        horizon_client::mobs::spawn_rules::Reason::Command,
                    ),
                ) {
                    Some(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            raider::set_patrol_leader(m, true);
                            raider::finalize(m, raider::Reason::Other);
                        }
                        self.feedback(Text::literal("Spawned a raid captain"));
                        Ok(1)
                    }
                    None => {
                        fail(self, "Pillager failed to spawn");
                        Ok(0)
                    }
                }
            }
            "raid.setomen" => {
                let level = Self::int(ctx, "level").unwrap_or(0);
                let Some(raid) = self.raiding_nearby(at) else {
                    fail(self, "No raid found here");
                    return Ok(1);
                };
                if level > horizon_client::raid::MAX_OMEN_LEVEL {
                    fail(
                        self,
                        &format!(
                            "Sorry, the max raid omen level you can set is {}",
                            horizon_client::raid::MAX_OMEN_LEVEL
                        ),
                    );
                } else {
                    let before = self.raid_set_omen(raid, level);
                    self.feedback(Text::literal(format!(
                        "Changed village's raid omen level from {before} to {level}"
                    )));
                }
                Ok(1)
            }
            "raid.glow" => {
                if let Some(raid) = self.raiding_nearby(at) {
                    self.raid_glow(raid);
                }
                Ok(1)
            }
            // RaidCommand.playSound: only for the text "local", at volume 2,
            // with a seed from the level's random (playSeededSound).
            "raid.sound" => {
                // ComponentArgument: `local`, "local" and 'local' are all the
                // text "local".
                let kind = match ctx.get("type") {
                    Some(Arg::Str(s)) => s.trim(),
                    _ => "",
                };
                if matches!(kind, "local" | "\"local\"" | "'local'") {
                    let seed = self.world.random.next_long();
                    let at = [player.x + 5.0, player.y, player.z];
                    self.audio
                        .play_seeded("minecraft:event.raid.horn", Some(at), 2.0, 1.0, seed);
                }
                Ok(1)
            }
            _ => Err(err("commands.help.failed", &[])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Game;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{command::vanilla, effects::EffectInstance, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;

    /// Runs a line of the dev tree (`/raid` is registered only there).
    fn run_dev(game: &mut Game<'_>, line: &str) -> i32 {
        let d = vanilla::build_with(true);
        let parse = d.parse(line, &game.ui.registries);
        let (ctx, _) = parse.executable().expect(line);
        let (ctx, root) = (ctx.clone(), parse.context.clone());
        game.run_chain(&root, &ctx).expect(line)
    }

    #[test]
    #[ignore = "daily"]
    fn raid_check_finds_a_raid_counting_down_and_the_horn_is_local_only() {
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            // RaidCommand needs the owner's permission level (3).
            game.ui.registries.permission = 4;
            game.chat_capture = Some(Vec::new());
            game.audio.capture = Some(Vec::new());
            let (id, _) = game.raiding.raids.create_or_extend([0, 64, 0], 2, None, 3);
            assert!(!game.raiding.raids.get(id).unwrap().started);

            assert_eq!(run_dev(game, "raid check"), 1);
            let lines: Vec<String> = game
                .chat_capture
                .take()
                .unwrap()
                .iter()
                .map(|t| t.plain())
                .collect();
            assert_eq!(
                lines,
                [
                    "Found a started raid!",
                    "Num groups spawned: 0 Raid omen level: 0 Num mobs: 0 Raid health: 0.0 / 0.0",
                ]
            );

            run_dev(game, "raid sound global");
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            run_dev(game, "raid sound local");
            let played: Vec<_> = game
                .audio
                .capture
                .take()
                .unwrap()
                .into_iter()
                .map(|s| (s.event, s.position, s.volume))
                .collect();
            assert_eq!(
                played,
                [(
                    "minecraft:event.raid.horn".to_owned(),
                    [5.5, 64.0, 0.5],
                    2.0
                )]
            );
        });
    }

    /// `RaidCommand.start` goes through `Raids.createOrExtendRaid`, which
    /// absorbs the player's Raid Omen (`Raid.absorbRaidOmen`: the
    /// `raid_trigger` statistic) before it sets the level from the command.
    /// The omen stays: nothing in vanilla removes it there.
    #[test]
    #[ignore = "daily"]
    fn raid_start_absorbs_the_players_raid_omen() {
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.ui.registries.permission = 4;
            game.chat_capture = Some(Vec::new());
            let stat = |g: &Game<'_>| g.stats.get("custom", "minecraft:raid_trigger");

            // No Raid Omen: absorbRaidOmen finds no effect and awards nothing.
            assert_eq!(run_dev(game, "raid start 2"), 1);
            assert_eq!(stat(game), 0);
            run_dev(game, "raid stop");

            game.add_player_effect(EffectInstance::new("raid_omen", 600, 1));
            assert_eq!(run_dev(game, "raid start 3"), 1);
            // The omen's amplifier + 1 is absorbed, then the command's level wins
            // (`Raid.setRaidOmenLevel`).
            let (_, raid) = game.raiding.raids.nearby([0, 64, 0]).expect("raid here");
            assert_eq!(raid.omen_level, 3);
            assert_eq!(stat(game), 1);
            assert!(game.potions.effects.get("raid_omen").is_some());
        });
    }

    /// #15334: a killed captain drops `Raid.getOminousBannerInstance`, not a
    /// plain white banner. Expected values are vanilla's
    /// `Raid.getBannerComponentPatch` (26.3).
    #[test]
    #[ignore = "daily"]
    fn a_killed_captain_drops_the_ominous_banner_it_wears() {
        use horizon_core::nbt::Tag;
        let ominous_patch = || {
            let layer = |pattern: &str, color: &str| {
                Tag::compound([
                    ("pattern", Tag::String(pattern.into())),
                    ("color", Tag::String(color.into())),
                ])
            };
            [
                (
                    "minecraft:banner_patterns",
                    Tag::List(vec![
                        layer("minecraft:rhombus", "cyan"),
                        layer("minecraft:stripe_bottom", "light_gray"),
                        layer("minecraft:stripe_center", "gray"),
                        layer("minecraft:border", "light_gray"),
                        layer("minecraft:stripe_middle", "black"),
                        layer("minecraft:half_horizontal", "light_gray"),
                        layer("minecraft:circle", "light_gray"),
                        layer("minecraft:border", "black"),
                    ]),
                ),
                (
                    "minecraft:tooltip_display",
                    Tag::compound([(
                        "hidden_components",
                        Tag::List(vec![Tag::String("minecraft:banner_patterns".into())]),
                    )]),
                ),
                (
                    "minecraft:item_name",
                    Tag::compound([(
                        "translate",
                        Tag::String("block.minecraft.ominous_banner".into()),
                    )]),
                ),
                ("minecraft:rarity", Tag::String("uncommon".into())),
            ]
        };
        let check = |wrong: &mut Vec<String>, what: &str, stack: &horizon_client::items::ItemStack| {
            if &*stack.item != "minecraft:white_banner" || stack.count != 1 {
                wrong.push(format!("{what}: {} x{}", stack.item, stack.count));
            }
            let components = horizon_client::item_data::encode_components(stack);
            for (key, want) in ominous_patch() {
                let got = components.as_ref().and_then(|c| c.get(key));
                if got != Some(&want) {
                    wrong.push(format!("{what}: {key} is {got:?}"));
                }
            }
            // What draws the stack (its icon, the item entity) and what
            // washing, the loom and shields read: the layers themselves.
            let layers = horizon_client::banner::layers(stack).len();
            if layers != 8 {
                wrong.push(format!("{what}: {layers} layers in memory"));
            }
        };

        with_game(|game| {
            let mut wrong = Vec::new();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.ui.registries.permission = 4;
            run_dev(game, "raid spawnleader");
            let captain = game
                .mobs
                .iter()
                .find(|m| &*m.kind.id == "minecraft:pillager")
                .expect("the captain");
            // PatrollingMonster.finalizeSpawn: setItemSlot(HEAD, the banner), as
            // getItemBySlot reads it and as SetEquipment sends it.
            let head = horizon_items::equipment::HEAD;
            let worn = captain.extra.slot_stack(head).cloned().expect("a head stack");
            check(&mut wrong, "worn", &worn);
            let read = horizon_client::mobs::finalize::item_stack(captain, head).expect("a head stack");
            check(&mut wrong, "read", &read);
            let before = game.dropped.len();
            assert!(game.run_command("kill @e[type=minecraft:pillager]"));
            let banners: Vec<_> = game
                .dropped
                .iter()
                .skip(before)
                .filter(|e| &*e.stack.item == "minecraft:white_banner")
                .map(|e| e.stack.clone())
                .collect();
            assert_eq!(banners.len(), 1, "the head drop (chance 2.0)");
            check(&mut wrong, "dropped", &banners[0]);
            assert!(wrong.is_empty(), "{wrong:#?}");
        });
    }
}
