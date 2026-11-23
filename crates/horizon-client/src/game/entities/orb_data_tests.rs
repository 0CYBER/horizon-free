//! `/data merge entity <orb> {Age:42s}`: `EntityDataAccessor.setData` keeps
//! the UUID around `Entity.load`, and `ExperienceOrb.readAdditionalSaveData`
//! reads the new `Age` into the same live orb
//! (tools/test_orb_data_parity.py measures the same on vanilla 26.3).
use super::*;
use horizon_core::nbt::{Tag, parse_snbt};
use std::sync::Arc;

const ORB: &str = "00000009-0000-0000-0000-000000000001";

fn game(test: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        test(game);
    });
}

/// Runs `command`: whether it succeeded and what it said first.
fn run(game: &mut Game<'_>, command: &str) -> (bool, Option<String>) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (ok, said.first().map(horizon_client::text::Text::plain))
}

#[test]
#[ignore = "daily"]
fn a_merged_age_reaches_the_same_live_orb() {
    game(|game| {
        let saved = parse_snbt(
            "{Pos:[4.5d,100d,4.5d],Motion:[0d,0d,0d],NoGravity:1b,Value:3s,Age:0s,Health:5s,\
         Count:1,UUID:[I;9,0,0,1],Tags:[\"h009\"]}",
        )
        .unwrap()
        .0;
        game.load_orb(saved, Some([9, 0, 0, 1]));
        assert_eq!(game.orbs.len(), 1);
        // The live state a reload would lose: its id, join and tick count.
        game.orbs[0].ticks = 17;
        let (id, join) = (game.orbs[0].id, game.orbs[0].join);
        let modified = Some("Modified entity data of Experience Orb".to_owned());
        assert_eq!(
            run(game, &format!("data merge entity {ORB} {{Age:42s}}")),
            (true, modified.clone())
        );
        assert_eq!(game.orbs.len(), 1, "the same orb, no second one");
        let o = &game.orbs[0];
        assert_eq!((o.id, o.join, o.ticks), (id, join, 17), "the same live orb");
        assert_eq!((o.age, o.value, o.count, o.health), (42, 3, 1, 5));
        assert_eq!(game.persist.uuids.get(&id), Some(&[9, 0, 0, 1]));
        assert!(
            run(
                game,
                &format!("execute if data entity {ORB} {{Age:42s,Tags:[\"h009\"]}}")
            )
            .0
        );
        // Nothing to change: the same failure as before.
        assert_eq!(
            run(game, &format!("data merge entity {ORB} {{Age:42s}}")),
            (
                false,
                Some(
                    "Nothing changed. The specified properties already have these values"
                        .to_owned()
                )
            )
        );
        // The UUID stays the orb's own.
        assert_eq!(
            run(
                game,
                &format!("data merge entity {ORB} {{UUID:[I;1,2,3,4]}}")
            ),
            (true, modified)
        );
        assert_eq!(game.persist.uuids.get(&id), Some(&[9, 0, 0, 1]));
        let tag = game.command_target_nbt(commands::Target::Orb(id)).unwrap();
        assert_eq!(tag.get("UUID"), Some(&Tag::IntArray(vec![9, 0, 0, 1])));
        assert_eq!(game.orbs[0].id, id);
    });
}

/// Loads an orb from `snbt` with the UUID `[9, 0, 0, n]`; returns its id.
fn load(game: &mut Game<'_>, snbt: &str, n: i32) -> u32 {
    game.load_orb(parse_snbt(snbt).unwrap().0, Some([9, 0, 0, n]));
    game.orbs.last().unwrap().id
}
fn orb<'a>(game: &'a Game<'_>, id: u32) -> &'a horizon_client::entities::ExperienceOrb {
    game.orbs.iter().find(|o| o.id == id).unwrap()
}

/// Added controls (after the baseline): the merge changes only what the
/// saved compound says, on the same orb; the edited compound replaces a
/// stale one; and the portal cooldown is read while a live portal process
/// stays.
#[test]
#[ignore = "daily"]
fn the_merge_keeps_the_live_orb_and_its_portal_process() {
    use horizon_client::nether_portal::{PortalState, Process};
    game(|game| {
        // No Tags, so no raw compound; a moving orb with a rotation.
        let id = load(
            game,
            "{Pos:[4.5d,100d,4.5d],Motion:[0.1d,0d,0d],Rotation:[30f,10f],Value:3s,Age:0s,\
         Health:5s,Count:2}",
            1,
        );
        assert!(!game.persist.raw.contains_key(&id), "no raw compound yet");
        game.orbs.iter_mut().find(|o| o.id == id).unwrap().ticks = 17;
        let join = orb(game, id).join;
        assert!(run(game, &format!("data merge entity {ORB} {{Age:42s}}")).0);
        let o = orb(game, id);
        assert_eq!(
            (o.id, o.join, o.ticks, o.count, o.age),
            (id, join, 17, 2, 42)
        );
        assert_eq!(
            (o.velocity.x, o.rotation, o.no_gravity),
            (0.1, [30.0, 10.0], false)
        );
        assert_eq!(game.orbs.len(), 1);
        // A merge that moves it changes the same orb.
        assert!(
            run(
                game,
                &format!("data merge entity {ORB} {{Motion:[0d,0.2d,0d],NoGravity:1b}}")
            )
            .0
        );
        let o = orb(game, id);
        assert_eq!(
            (o.id, o.velocity.y, o.velocity.x, o.no_gravity, o.age),
            (id, 0.2, 0.0, true, 42)
        );
        // A stale raw compound gives way to the edited one.
        game.persist
            .raw
            .insert(id, parse_snbt("{Age:7s,Tags:[\"old\"]}").unwrap().0);
        assert!(
            run(
                game,
                &format!("data merge entity {ORB} {{Age:44s,Tags:[\"new\"]}}")
            )
            .0
        );
        assert_eq!(orb(game, id).age, 44);
        assert!(
            run(
                game,
                &format!("execute if data entity {ORB} {{Age:44s,Tags:[\"new\"]}}")
            )
            .0
        );
        assert!(
            !run(
                game,
                &format!("execute if data entity {ORB} {{Tags:[\"old\"]}}")
            )
            .0
        );
        // A live portal process stays; the cooldown is read, 0 when it is 0.
        let process = Process {
            entry: horizon_core::position::BlockPos::new(1, 2, 3),
            time: 5,
            inside: true,
        };
        game.portals.entities.insert(
            id,
            PortalState {
                cooldown: 300,
                process: Some(process),
            },
        );
        assert!(
            run(
                game,
                &format!("data merge entity {ORB} {{Age:45s,PortalCooldown:0}}")
            )
            .0
        );
        let state = game.portals.entities.get(&id).unwrap();
        let kept = state.process.map(|p| (p.entry, p.time, p.inside));
        assert_eq!(
            (state.cooldown, kept),
            (0, Some((process.entry, process.time, process.inside)))
        );
        // An orb with no portal state gets the saved positive cooldown.
        let other = load(
            game,
            "{Pos:[8.5d,100d,8.5d],Value:1s,Age:0s,Health:5s,Count:1}",
            2,
        );
        assert!(!game.portals.entities.contains_key(&other));
        assert!(
            run(
                game,
                "data merge entity 00000009-0000-0000-0000-000000000002 {PortalCooldown:120}"
            )
            .0
        );
        let state = game.portals.entities.get(&other).unwrap();
        assert_eq!((state.cooldown, state.process.is_none()), (120, true));
        assert_eq!(game.orbs.len(), 2);
    });
}

/// #16909: an orb and an arrow loaded with `TicksFrozen:60` keep it
/// (`Entity.saveWithoutId` writes it while positive; only a `LivingEntity`
/// thaws), and a loaded 0 is not written.
#[test]
#[ignore = "daily"]
fn a_loaded_ticks_frozen_is_kept_by_an_orb_and_an_arrow() {
    game(|game| {
        let orb = load(
            game,
            "{Pos:[4.5d,100d,4.5d],Value:1s,Age:0s,Health:5s,Count:1,TicksFrozen:60}",
            3,
        );
        let tag = game.command_target_nbt(commands::Target::Orb(orb)).unwrap();
        assert_eq!(tag.get("TicksFrozen"), Some(&Tag::Int(60)));
        let tag =
            parse_snbt("{Pos:[4.5d,100d,4.5d],Motion:[0d,0d,0d],Rotation:[0f,0f],TicksFrozen:60}")
                .unwrap()
                .0;
        game.load_listed("minecraft:arrow", Some([9, 0, 0, 4]), None, tag)
            .unwrap();
        let arrow = game.next_entity;
        let tag = game
            .command_target_nbt(commands::Target::Projectile(arrow))
            .unwrap();
        assert_eq!(tag.get("TicksFrozen"), Some(&Tag::Int(60)));
        let cold = load(
            game,
            "{Pos:[5.5d,100d,5.5d],Value:1s,Age:0s,Health:5s,Count:1,TicksFrozen:0}",
            4,
        );
        let tag = game
            .command_target_nbt(commands::Target::Orb(cold))
            .unwrap();
        assert_eq!(tag.get("TicksFrozen"), None);
        let short = load(
            game,
            "{Pos:[6.5d,100d,6.5d],Value:1s,Age:0s,Health:5s,Count:1,TicksFrozen:60s}",
            5,
        );
        let tag = game
            .command_target_nbt(commands::Target::Orb(short))
            .unwrap();
        assert_eq!(
            tag.get("TicksFrozen"),
            Some(&Tag::Int(60)),
            "saved as an int"
        );
    });
}

/// #16909: an orb and an arrow summoned with `PortalCooldown:250` save it back
/// as 250 (`Entity.saveWithoutId` writes the live countdown).
#[test]
#[ignore = "daily"]
fn a_summoned_portal_cooldown_is_saved_for_an_orb_and_an_arrow() {
    game(|game| {
        let orb = load(
            game,
            "{Pos:[4.5d,100d,4.5d],Value:1s,Age:0s,Health:5s,Count:1,PortalCooldown:250}",
            3,
        );
        let tag = game.command_target_nbt(commands::Target::Orb(orb)).unwrap();
        assert_eq!(tag.get("PortalCooldown"), Some(&Tag::Int(250)));
        let tag = parse_snbt(
            "{Pos:[4.5d,100d,4.5d],Motion:[0d,0d,0d],Rotation:[0f,0f],PortalCooldown:250}",
        )
        .unwrap()
        .0;
        game.load_listed("minecraft:arrow", Some([9, 0, 0, 4]), None, tag)
            .unwrap();
        let arrow = game.next_entity;
        let tag = game
            .command_target_nbt(commands::Target::Projectile(arrow))
            .unwrap();
        assert_eq!(tag.get("PortalCooldown"), Some(&Tag::Int(250)));
        // A merged cooldown survives the read path, which writes the live value.
        assert!(
            run(
                game,
                "data merge entity 00000009-0000-0000-0000-000000000004 {PortalCooldown:120}"
            )
            .0
        );
        let tag = game
            .command_target_nbt(commands::Target::Projectile(arrow))
            .unwrap();
        assert_eq!(tag.get("PortalCooldown"), Some(&Tag::Int(120)));
        // The chunk save writes the live cooldown for both as well.
        let keys = std::collections::HashSet::from([(0, 0)]);
        let index = game.orbs.iter().position(|o| o.id == orb).unwrap();
        let (_, saved) = game.save_orb(index, &keys).unwrap();
        assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(250)));
        let index = game
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == arrow)
            .unwrap();
        let (_, saved) = game.save_projectile(index, &keys).unwrap();
        assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(120)));
    });
}
