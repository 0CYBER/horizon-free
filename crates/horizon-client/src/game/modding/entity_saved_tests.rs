//! `entity_set_saved`: an entity a mod kept out of the saves is not written, by the autosave or by
//! its chunk's unload, until it is put back.
use std::{collections::HashSet, sync::Arc};

use horizon_client::chunk::Chunk;
use horizon_core::nbt::Compound;

use super::{Dim, Game};

#[test]
#[ignore = "daily"]
fn an_entity_kept_out_of_the_saves_is_not_written_until_put_back() {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_joins_skip();
        let dimension = game.dim.id().to_owned();
        let keys = HashSet::from([(0, 0)]);
        let summon = |game: &mut Game<'_>, kind: &str| {
            let id = game
                .summon(&dimension, kind, [1.5, 70.0, 1.5], Compound::new())
                .expect("a summoned entity");
            game.entity(id).expect("its handle")
        };
        let kept = summon(game, "minecraft:text_display");
        summon(game, "minecraft:item_display");
        let pig = summon(game, "minecraft:pig");
        let saved = |game: &mut Game<'_>| -> Vec<String> {
            let mut kinds: Vec<String> = game
                .entity_chunks(&keys, false)
                .iter()
                .flat_map(|(_, tags)| tags)
                .filter_map(horizon_client::entity_nbt::kind)
                .map(|kind| kind.to_string())
                .collect();
            kinds.sort();
            kinds
        };
        assert_eq!(saved(game).len(), 3);
        assert!(game.entity_set_saved(kept, false));
        assert!(game.entity_set_saved(pig, false));
        assert_eq!(saved(game), ["minecraft:item_display"]);
        assert!(game.entity_set_saved(pig, true));
        assert_eq!(saved(game), ["minecraft:item_display", "minecraft:pig"]);
        // A handle that is gone is refused.
        game.mobs.retain(|mob| mob.id != pig.id());
        assert!(!game.entity_set_saved(pig, false));
        // Unloading the chunk writes what is left, and takes the kept entity away unwritten.
        let unloaded = game.entity_chunks(&keys, true);
        assert_eq!(unloaded.iter().map(|(_, tags)| tags.len()).sum::<usize>(), 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_rider_does_not_carry_a_kept_out_vehicle_into_its_file() {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_joins_skip();
        let dimension = game.dim.id().to_owned();
        let id = game
            .summon(&dimension, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new())
            .expect("a summoned entity");
        let pig = game.entity(id).expect("its handle");
        // `ServerPlayer.saveParentVehicle`: the ridden pig is the root vehicle.
        assert!(game.parent_vehicle_tag(Some(id), &[]).is_some());
        assert!(game.entity_set_saved(pig, false));
        assert!(game.parent_vehicle_tag(Some(id), &[]).is_none());
        assert!(game.entity_set_saved(pig, true));
        assert!(game.parent_vehicle_tag(Some(id), &[]).is_some());
    });
}

#[test]
#[ignore = "daily"]
fn an_entity_kept_out_of_the_saves_stays_out_after_a_change_of_level() {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_joins_skip();
        let dimension = game.dim.id().to_owned();
        let id = game
            .summon(&dimension, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new())
            .expect("a summoned entity");
        let pig = game.entity(id).expect("its handle");
        assert!(game.entity_set_saved(pig, false));
        let tag = {
            let keys = HashSet::from([(0, 0)]);
            let found = game.entity_chunks(&keys, false);
            assert!(found.iter().all(|(_, tags)| tags.is_empty()));
            // Written once with the flag off, to carry the entity's own tag across.
            assert!(game.entity_set_saved(pig, true));
            let tags: Vec<_> = game.entity_chunks(&keys, false).into_iter().flat_map(|(_, t)| t).collect();
            assert_eq!(tags.len(), 1);
            assert!(game.entity_set_saved(pig, false));
            tags.into_iter().next().unwrap()
        };
        let state = game.take_transferred_entity_state(id);
        game.mobs.retain(|mob| mob.id != id);
        game.detach_entity(id);
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let moved = game
            .load_mob_transferred(tag, None, Some(state))
            .expect("the pig arrives");
        let keys = HashSet::from([(0, 0)]);
        assert!(
            game.entity_chunks(&keys, false)
                .iter()
                .all(|(_, tags)| tags.is_empty()),
            "the arrival is still kept out"
        );
        let moved = game.entity(moved).expect("its handle");
        assert!(game.entity_set_saved(moved, true));
        let saved: usize = game.entity_chunks(&keys, false).iter().map(|(_, t)| t.len()).sum();
        assert_eq!(saved, 1);
    });
}
