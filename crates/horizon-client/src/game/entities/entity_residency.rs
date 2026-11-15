//! One storage-family dispatch for the server's residency journal.
use super::{Game, commands::Target};
use horizon_client::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner,
        table::{Location, SourceEntity},
    },
};
use mod_api::DimensionKey;

/// Assign a stored zero-id entity through its residency boundary exactly once.
macro_rules! assign_resident_id {
    ($list:expr, $index:expr, $entity:ident, $next:expr, $lifetime:expr) => {{
        let index = $index;
        if $list[index].id == 0 {
            let id = $list.update(index, |$entity| {
                $crate::game::entity::assign_stored_id($next, &mut $entity.id, $lifetime)
            });
            $list.id_assigned(index);
            id
        } else {
            $list[index].id
        }
    }};
}
pub(crate) use assign_resident_id;

macro_rules! assign_resident_ids {
    ($list:expr, $entity:ident, $next:expr, $lifetime:expr) => {
        for index in 0..$list.len() {
            $crate::game::entity_residency::assign_resident_id!(
                $list, index, $entity, $next, $lifetime
            );
        }
    };
}
pub(crate) use assign_resident_ids;

// Projectiles, knots and general rows have distinct Game/Parked field paths;
// keep binding and resolution on this one family list.
macro_rules! resident_owners {
    ($then:ident, $this:ident, $($projectiles:ident).+, $($knots:ident).+, $($general:ident).+) => {
        $then! {
            ($this, $($projectiles).+)
            Mob => Mob: mobs,
            Item => Item: dropped,
            Orb => Orb: orbs,
            Stand => Stand: armor_stands,
            Frame => Frame: item_frames,
            Painting => Painting: paintings,
            Boat => Boat: boats,
            Cart => Cart: carts,
            Other => Other: others,
            Fangs => Fangs: fangs.list,
            Crystal => Crystal: crystals,
            Tnt => Tnt: tnts,
            Cushion => Cushion: cushions,
            Knot => Knot: $($knots).+,
            Falling => Falling: falling,
            Rocket => Rocket: rockets,
            Trident => Trident: tridents,
            Spawner => Spawner: ominous_spawners,
            Cloud => Cloud: breath_clouds,
            General => General: $($general).+,
        }
    };
}
pub(crate) use resident_owners;

macro_rules! resident_visible {
    (Mob, $entity:expr) => {
        !$entity.removed
    };
    ($kind:ident, $entity:expr) => {
        true
    };
}
pub(crate) use resident_visible;

macro_rules! bind_resident_list {
    (Mob, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency_with_subtype(
            $journal,
            $owner,
            |entity| (entity.id, entity.join_seq()),
            |entity| {
                if matches!(
                    entity.species(),
                    horizon_client::mobs::Species::Villager
                        | horizon_client::mobs::Species::WanderingTrader
                ) {
                    horizon_client::join_seq::residency::EntitySubtype::Merchant
                } else {
                    horizon_client::join_seq::residency::EntitySubtype::Ordinary
                }
            },
        )
    };
    (Other, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency_with_subtype(
            $journal,
            $owner,
            |entity| (entity.id, entity.join_seq()),
            |entity| {
                use horizon_client::join_seq::residency::EntitySubtype;
                match entity.kind.as_str() {
                    "minecraft:block_display" => EntitySubtype::BlockDisplay,
                    "minecraft:item_display" => EntitySubtype::ItemDisplay,
                    "minecraft:text_display" => EntitySubtype::TextDisplay,
                    _ => EntitySubtype::Ordinary,
                }
            },
        )
    };
    (Cart, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency_with_subtype(
            $journal,
            $owner,
            |entity| (entity.id, entity.join_seq()),
            |entity| {
                use horizon_client::join_seq::residency::EntitySubtype;
                use $crate::game::minecarts::CartKind;
                match entity.kind {
                    CartKind::Chest => EntitySubtype::ChestCart,
                    CartKind::Furnace => EntitySubtype::FurnaceCart,
                    CartKind::Hopper => EntitySubtype::HopperCart,
                    CartKind::Tnt => EntitySubtype::TntCart,
                    CartKind::CommandBlock => EntitySubtype::CommandCart,
                    _ => EntitySubtype::Ordinary,
                }
            },
        )
    };
    ($kind:ident, $list:expr, $journal:expr, $owner:expr) => {
        $list.bind_residency($journal, $owner, |entity| (entity.id, entity.join_seq()))
    };
}
pub(crate) use bind_resident_list;

/// Physical lifetime access, not a cached publication or a delivery promise.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SlotAccess<R> {
    Done(R),
    Taken,
    Missing,
    UnsupportedKind(Kind),
}

macro_rules! resident_lifetime_slot {
    ($kind:ident, $list:expr, $join:expr, $visit:expr) => {{
        let Some(index) = $list.locate($join) else {
            return SlotAccess::Missing;
        };
        SlotAccess::Done($list.update(index, |entity| {
            $visit(
                horizon_mod_runtime::attachments::HasAttachmentLifetime::attachment_lifetime(
                    entity,
                ),
            )
        }))
    }};
}
pub(crate) use resident_lifetime_slot;

macro_rules! define_resident_access {
    (($this:ident, $($projectiles:ident).+) $($kind:ident => $target:ident: $($field:ident).+,)*) => {
        pub(super) fn bind_resident_owners(&mut $this, journal: &Journal, dimension: DimensionKey, track_saved_uuids: bool) {
            if track_saved_uuids {
                $this.persist.uuids.bind(journal, dimension);
            }
            $($crate::game::entity_residency::bind_resident_list!($kind, $this$(.$field)+, journal, Owner { dimension, kind: Kind::$kind });)*
            $this$(.$projectiles)+.bind_residency(journal, Owner { dimension, kind: Kind::Projectile }, |entity| (entity.id, entity.join_seq()));
            $this.dragon.bind_residency_specialized(journal, Owner { dimension, kind: Kind::Dragon }, |entity| (entity.id, entity.join_seq()), horizon_client::join_seq::residency::Source::Ordinary, Some((Kind::DragonPart, 8, |dragon, offset| dragon.parts.get(usize::from(offset)).map(|part| part.3))), None);
        }
        pub(super) fn with_resident_owner_slot<R>(
            &mut $this,
            at: Location,
            visit: impl FnOnce(&mut horizon_mod_runtime::attachments::AttachmentLifetime) -> R,
        ) -> $crate::game::entity_residency::SlotAccess<R> {
            use $crate::game::entity_residency::SlotAccess;
            match at.owner.kind {
                $(Kind::$kind => $crate::game::entity_residency::resident_lifetime_slot!($kind, $this$(.$field)+, at.join, visit),)*
                Kind::Projectile => $crate::game::entity_residency::resident_lifetime_slot!(Projectile, $this$(.$projectiles)+, at.join, visit),
                Kind::Dragon => {
                    $this.dragon.locate(at.join).map_or(SlotAccess::Missing, |_| {
                        use horizon_mod_runtime::attachments::HasAttachmentLifetime;
                        SlotAccess::Done(visit($this.dragon.as_mut().unwrap().attachment_lifetime()))
                    })
                },
                Kind::DragonPart => {
                    use horizon_client::join_seq::residency::Source;
                    use horizon_mod_runtime::attachments::HasAttachmentLifetime;
                    let Source::DragonPart(offset) = at.source else { return SlotAccess::Missing; };
                    if $this.dragon.locate(at.join).is_none() { return SlotAccess::Missing; }
                    let Some(part) = $this.dragon.as_mut().and_then(|dragon| dragon.parts.get_mut(usize::from(offset))) else { return SlotAccess::Missing; };
                    SlotAccess::Done(visit(part.attachment_lifetime()))
                },
                Kind::Player | Kind::Hook => SlotAccess::UnsupportedKind(at.owner.kind),
            }
        }
        pub(super) fn locate_resident_attachment_owner(&$this, at: Location) -> Option<SourceEntity<(Target, bool)>> {
            let source = match at.owner.kind {
                $(Kind::$kind => {
                    let index = $this$(.$field)+.locate(at.join)?;
                    let entity = &$this$(.$field)+[index];
                    let visible = $crate::game::entity_residency::resident_visible!($kind, entity);
                    let id = entity.id;
                    Some(SourceEntity { id, value: (Target::$target(id), visible) })
                })*
                Kind::Projectile => {
                    let index = $this$(.$projectiles)+.locate(at.join)?;
                    let id = $this$(.$projectiles)+[index].id;
                    Some(SourceEntity { id, value: (Target::Projectile(id), true) })
                }
                Kind::Dragon => {
                    $this.dragon.locate(at.join)?;
                    let id = $this.dragon.as_ref()?.id;
                    Some(SourceEntity { id, value: (Target::Dragon(id), true) })
                }
                Kind::DragonPart => {
                    let horizon_client::join_seq::residency::Source::DragonPart(offset) = at.source else { return None; };
                    $this.dragon.locate(at.join)?;
                    let dragon = $this.dragon.as_ref()?;
                    if dragon.id == 0 { return None; }
                    dragon.parts.get(usize::from(offset))?;
                    let id = dragon.id.checked_add(u32::from(offset) + 1)?;
                    let visible = true;
                    Some(SourceEntity { id, value: (Target::DragonPart(id), visible) })
                }
                Kind::Player | Kind::Hook => None,
            };
            source
        }
        pub(super) fn locate_resident_owner(&$this, at: Location) -> Option<SourceEntity<(Target, bool)>> {
            let source = $this.locate_resident_attachment_owner(at).map(|mut source| {
                if let Target::DragonPart(_) = source.value.0 {
                    source.value.1 &= $this.dragon.as_ref().is_some_and(|dragon| !$this.persist.removals.contains(&Target::Dragon(dragon.id)));
                }
                source
            });
            // Logical visibility is not physical absence: filtering here would
            // incorrectly evict reversible conversion rows or report Taken.
            source.map(|mut source| {
                source.value.1 &= !$this.persist.removals.contains(&source.value.0);
                source.value.1 &= !$this.persist.destroying_vehicles.contains(&source.value.0);
                source
            })
        }
    };
}
pub(crate) use define_resident_access;

impl Game<'_> {
    resident_owners!(
        define_resident_access,
        self,
        creatures.projectiles,
        resident_entities.knots,
        resident_entities.general.rows
    );
}

#[cfg(test)]
#[path = "entity_residency/all_family_binding_tests.rs"]
mod all_family_binding_tests;
#[cfg(test)]
#[path = "entity_residency/lifetime_tests.rs"]
mod lifetime_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{
        join_seq::{RestoreOrder, residency::table::EntityLookup},
        mobs::Mob,
    };
    use horizon_core::{geometry::Vec3, position::BlockPos};

    #[test]
    #[ignore = "daily"]
    fn resident_slot_access_routes_live_parked_taken_and_disjoint() {
        use crate::game::Dim;
        use std::num::NonZeroU32;
        let id = |value| NonZeroU32::new(value).unwrap();
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| 7),
                SlotAccess::Done(7)
            );
            assert_eq!(
                game.with_resident_slot(&DimensionKey::NETHER, id(410), |_| panic!(
                    "wrong dimension"
                )),
                SlotAccess::<()>::Missing
            );
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(999), |_| panic!("missing")),
                SlotAccess::<()>::Missing
            );
            let join = game.mobs[0].join_seq();
            let extracted = game.mobs.extract(join).unwrap();
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| panic!("extracted")),
                SlotAccess::<()>::Taken
            );
            game.mobs.push(Mob::new(411, kind.clone(), Vec3::ZERO, 0.0));
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(411), |_| 8),
                SlotAccess::Done(8)
            );
            game.mobs.put_back(0, extracted);
            let lifted = game.mobs.lift();
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| panic!("lifted")),
                SlotAccess::<()>::Taken
            );
            game.mobs.push(Mob::new(412, kind, Vec3::ZERO, 0.0));
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(412), |_| 9),
                SlotAccess::Done(9)
            );
            game.mobs.restore(lifted, RestoreOrder::SurvivorsFirst);
            game.mobs.update(0, |mob| mob.removed = true);
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| 10),
                SlotAccess::Done(10)
            );
            game.summon_with_nbt("minecraft:tnt", [0.5, 70.0, 0.5], "{}")
                .unwrap();
            game.lan_assign_ids();
            let tnt = id(game.tnts[0].id);
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, tnt, |slot| slot.is_bound()),
                SlotAccess::Done(false)
            );
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, tnt, |slot| slot.is_bound()),
                SlotAccess::Done(false)
            );
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| 11),
                SlotAccess::Done(11)
            );
            assert_eq!(game.dim, Dim::Nether);
            game.enter_level(Dim::Overworld).unwrap();
            game.mobs.remove(0);
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| panic!("removed")),
                SlotAccess::<()>::Missing
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resident_slot_access_cushion_reentry_and_panic_preserve_actual_lifetime() {
        use horizon_mod_runtime::attachments::{
            Attachments, HasAttachmentLifetime, index::AttachmentIndex,
        };
        use std::{
            num::NonZeroU32,
            sync::{Arc, Mutex},
        };
        with_game(|game| {
            game.reset_entity_residency();
            game.summon_with_nbt("minecraft:cushion", [0.5, 70.0, 0.5], "{}")
                .unwrap();
            game.lan_assign_ids();
            let id = NonZeroU32::new(game.cushions[0].id).unwrap();
            let holder = game.entity(id.get()).unwrap();
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            let journal = game.resident_entities.table.borrow().producer();
            let result = game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| {
                // Reenter the same structural journal: a retained drain borrow panics.
                journal.drain_into(&mut Vec::new());
                slot.bind(&mut values, &index, holder);
                slot.registration().unwrap()
            });
            let SlotAccess::Done(registration) = result else {
                panic!("resident slot unavailable")
            };
            game.cushions.update(0, |mob| {
                assert!(
                    registration
                        .same_registration(&mob.attachment_lifetime().registration().unwrap())
                )
            });
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| {
                    assert!(registration.same_registration(&slot.registration().unwrap()));
                    panic!("visitor panic");
                });
            }));
            assert!(panic.is_err());
            game.cushions.update(0, |mob| {
                assert!(
                    registration
                        .same_registration(&mob.attachment_lifetime().registration().unwrap())
                )
            });
            assert!(game.resident_entities.table.try_borrow_mut().is_ok());
            game.enter_level(crate::game::Dim::Nether).unwrap();
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| slot.is_bound()),
                SlotAccess::Done(true)
            ));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn game_residency_tracks_lift_spawn_restore_and_epoch_rebind() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            let join = game.mobs[0].join_seq();
            let taken = game.mobs.lift();
            assert_eq!(game.server_resident_entity(410), EntityLookup::Taken);
            game.mobs.push(Mob::new(411, kind, Vec3::ZERO, 0.0));
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found {
                    value: Target::Mob(411),
                    ..
                }
            ));
            game.mobs.restore(taken, RestoreOrder::SurvivorsFirst);
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, value: Target::Mob(410) } if location.join == join)
            );
            game.reset_entity_residency();
            assert!(matches!(
                game.server_resident_entity(410),
                EntityLookup::Found { .. }
            ));
            game.mobs.remove(0);
            assert_eq!(game.server_resident_entity(410), EntityLookup::Missing);
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn initial_non_overworld_owner_is_bound_before_level_notification() {
        use crate::game::Dim;
        with_game(|game| {
            game.dim = Dim::Nether;
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind, Vec3::ZERO, 0.0));
            game.native_level_opened(Dim::Nether);
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, value: Target::Mob(410) } if location.owner.dimension == DimensionKey::NETHER)
            );
            game.native_lifecycle_world_closed();
            assert_eq!(game.server_resident_entity(410), EntityLookup::Missing);
            game.native_lifecycle_open_started();
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, .. } if location.owner.dimension == DimensionKey::NETHER)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn parked_owners_rebind_and_route_without_trial_dimension_swaps() {
        use crate::game::Dim;
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            game.enter_level(Dim::Nether).unwrap();
            game.mobs.push(Mob::new(411, kind, Vec3::ZERO, 0.0));
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, .. } if location.owner.dimension == DimensionKey::OVERWORLD)
            );
            game.reset_entity_residency();
            assert!(matches!(
                game.server_resident_entity(410),
                EntityLookup::Found { .. }
            ));
            assert!(
                matches!(game.server_resident_entity(411), EntityLookup::Found { location, .. } if location.owner.dimension == DimensionKey::NETHER)
            );
            assert_eq!(game.level_entity(410), None);
            assert_eq!(game.level_entity(411), Some(Target::Mob(411)));
            assert_eq!(
                game.in_target_level(Target::Mob(410), |game| game.dim),
                Some(Dim::Overworld)
            );
            assert_eq!(game.dim, Dim::Nether);
            game.levels.clear();
            assert_eq!(game.server_resident_entity(410), EntityLookup::Missing);
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn logical_removal_is_not_lifted_absence_or_physical_retirement() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            let taken = game.mobs.lift();
            game.mobs.push(Mob::new(411, kind, Vec3::ZERO, 0.0));
            game.mobs[0].removed = true;
            assert_eq!(game.server_resident_entity(411), EntityLookup::Missing);
            // Conversion can restore a resident after a pre hook. A read while
            // removed must not destroy its physical residency row.
            game.mobs[0].removed = false;
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            game.persist.removals.push(Target::Mob(411));
            assert_eq!(game.server_resident_entity(411), EntityLookup::Missing);
            game.persist.removals.clear();
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            game.mobs.restore(taken, RestoreOrder::SurvivorsFirst);
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn late_tracking_admission_publishes_the_previously_zero_id() {
        with_game(|game| {
            game.reset_entity_residency();
            game.add_primed_tnt(BlockPos::new(0, 70, 0), 0.0, 80, None, None);
            let mut tnt = game.tnts.remove(0);
            let retired = tnt.id;
            tnt.id = 0;
            game.tnts.push(tnt);
            assert_eq!(game.server_resident_entity(retired), EntityLookup::Missing);
            assert_eq!(game.server_resident_entity(0), EntityLookup::Missing);
            game.lan_assign_ids();
            let id = game.tnts[0].id;
            assert_ne!(id, 0);
            assert!(
                matches!(game.server_resident_entity(id), EntityLookup::Found { value: Target::Tnt(found), .. } if found == id)
            );
            game.lan_assign_ids();
            assert_eq!(game.tnts[0].id, id);
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }
}
