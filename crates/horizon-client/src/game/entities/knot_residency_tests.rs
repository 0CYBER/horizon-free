//! Natural knots use the same numbered owner journal as other entities.
#![cfg(test)]
use super::{Dim, Game, commands::Target, native_entity_refs::tests::with_game};
use horizon_client::{join_seq::residency::table::EntityLookup, mobs::leash::Holder};

fn tie_player_mob(game: &mut Game<'_>, fence: [i32; 3]) -> (u32, u32) {
    game.summon_with_nbt("minecraft:pig", fence.map(f64::from), "{}")
        .unwrap();
    let mob = game.mobs.last().unwrap().id;
    assert!(game.set_leash(Target::Mob(mob), Holder::Player, None));
    assert!(game.use_knot(fence, false));
    let knot = game.knot_ids[&fence];
    assert_ne!(knot, 0);
    // This assertion is also the allocation INSTALL mutant's observable failure.
    assert_eq!(game.natural_knot(knot), Some(fence));
    (mob, knot)
}

#[test]
#[ignore = "daily"]
fn natural_knot_owner_survives_active_parked_reset_and_retires_at_last_holder() {
    with_game(|game| {
        game.reset_entity_residency();
        let fence = [1, 70, 1];
        let (first, knot) = tie_player_mob(game, fence);
        let (second, same_knot) = tie_player_mob(game, fence);
        assert_eq!(knot, same_knot);
        assert_eq!(game.resident_entities.knots.len(), 1);
        let join = game.resident_entities.knots[0].join;
        // Leashable.dropLeash -> LeashFenceKnotEntity.notifyLeasheeRemoved:
        // one departing holder leaves the knot; the last discards it immediately.
        assert!(game.detach_leash(Target::Mob(first), false));
        assert_eq!(game.natural_knot(knot), Some(fence));
        assert_eq!(game.resident_entities.knots[0].join, join);

        game.enter_level(Dim::Nether).unwrap();
        let (nether_mob, nether_knot) = tie_player_mob(game, fence);
        assert_ne!(
            knot, nether_knot,
            "ids are server-wide, not dimension-local"
        );
        assert_eq!(game.natural_knot(knot), None);
        for _ in 0..2 {
            game.reset_entity_residency();
            for (id, dim) in [(knot, Dim::Overworld), (nether_knot, Dim::Nether)] {
                assert!(
                    matches!(game.server_resident_entity(id), EntityLookup::Found { location, value: Target::Knot(found) }
                    if found == id && location.owner.dimension == dim.key())
                );
            }
        }
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(game.natural_knot(knot), Some(fence));
        assert_eq!(game.resident_entities.knots[0].join, join);
        assert!(game.detach_leash(Target::Mob(second), false));
        assert_eq!(game.server_resident_entity(knot), EntityLookup::Missing);
        assert!(!game.knot_ids.contains_key(&fence));
        assert!(game.resident_entities.knots.is_empty());
        assert!(matches!(
            game.server_resident_entity(nether_knot),
            EntityLookup::Found {
                value: Target::Knot(_),
                ..
            }
        ));
        game.enter_level(Dim::Nether).unwrap();
        // The other canonical mutation is retargeting, not dropping the lead.
        assert!(game.set_leash(Target::Mob(nether_mob), Holder::Player, None));
        assert_eq!(
            game.server_resident_entity(nether_knot),
            EntityLookup::Missing
        );
        assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
    });
}

#[test]
#[ignore = "daily"]
fn natural_knot_tick_cleanup_retires_a_removed_holder_and_fresh_tie_gets_a_new_id() {
    with_game(|game| {
        game.reset_entity_residency();
        let fence = [1, 70, 1];
        let (_, knot) = tie_player_mob(game, fence);
        // A removal without a leash notification is reconciled by the existing
        // tracking cleanup, not by a hidden holder scan during lookup.
        game.mobs[0].removed = true;
        assert_eq!(game.natural_knot(knot), Some(fence));
        game.forget_untied_knots();
        assert_eq!(game.server_resident_entity(knot), EntityLookup::Missing);
        let (_, replacement) = tie_player_mob(game, fence);
        assert_ne!(replacement, knot);
        assert_eq!(game.server_resident_entity(0), EntityLookup::Missing);
        assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
    });
}

#[test]
#[ignore = "daily"]
fn natural_knot_last_boat_notification_excludes_its_old_resident_payload() {
    with_game(|game| {
        game.reset_entity_residency();
        let fence = [1, 70, 1];
        let (mob, knot) = tie_player_mob(game, fence);
        game.summon_with_nbt("minecraft:oak_boat", [1.0, 70.0, 1.0], "{}")
            .unwrap();
        let boat = game.boats[0].id;
        assert!(game.set_leash(Target::Boat(boat), Holder::Knot(fence), None));
        assert!(game.detach_leash(Target::Mob(mob), false));
        assert_eq!(game.natural_knot(knot), Some(fence));
        // A copied boat is removed before its original payload is taken out
        // of the storage list; that old payload is not a remaining holder.
        let departing = game.boats[0].clone();
        game.boat_removed(&departing);
        assert_eq!(game.server_resident_entity(knot), EntityLookup::Missing);
        game.boats.remove(0);

        let (mob, knot) = tie_player_mob(game, fence);
        game.summon_with_nbt("minecraft:oak_boat", [1.0, 70.0, 1.0], "{}")
            .unwrap();
        let boat = game.boats[0].id;
        assert!(game.set_leash(Target::Boat(boat), Holder::Knot(fence), None));
        assert!(game.detach_leash(Target::Mob(mob), false));
        game.release_stored_boat_leash(0, false);
        assert_eq!(game.server_resident_entity(knot), EntityLookup::Missing);
        assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
    });
}
