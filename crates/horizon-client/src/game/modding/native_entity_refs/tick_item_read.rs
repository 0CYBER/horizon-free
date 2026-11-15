impl Game<'_> {
    pub(super) fn unscoped_native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        if self.held_out.contains(&entity.id())
            || !self.is_logical_server()
            || self.native_lifecycle.epoch() != Some(entity.epoch())
        {
            return None;
        }
        let horizon_client::join_seq::residency::table::EntityLookup::Found { location, value } =
            self.server_resident_entity(entity.id())
        else {
            return None;
        };
        // Players are server-wide records. Ordinary body operations retain
        // their existing current-level domain; source membership is global.
        (matches!(value, Target::Player | Target::Guest(_))
            || location.owner.dimension == self.dim.key())
        .then_some(value)
    }
    #[cold]
    #[inline(never)]
    fn scoped_native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        if self
            .tick_item_view(entity.id())
            .is_some_and(|stack| stack.is_none())
        {
            return None;
        }
        self.unscoped_native_entity_target(entity)
    }
}

#[cfg(test)]
mod general_handle_tests {
    use super::tests::with_game;
    use crate::game::commands::Target;
    use crate::game::general::GeneralRow;
    use horizon_core::geometry::Vec3;
    use mod_api::{EntityRef, WorldEpoch};

    fn row(id: u32, uuid: [i32; 4], fall: f64) -> GeneralRow {
        GeneralRow {
            id,
            uuid,
            join: horizon_client::join_seq::JoinSeq::UNJOINED,
            type_slot: 0,
            position: Vec3::new(0.5, 64.0, 0.5),
            previous: Vec3::new(0.5, 64.0, 0.5),
            velocity: Vec3::ZERO,
            rotation: [0.0, 0.0],
            fall_distance: fall,
            payload_version: 1,
            payload_bytes: Vec::new(),
            mount: None,
            attachment_lifetime: Default::default(),
        }
    }

    /// arismoko/horizon#792's stale-handle row, as the id counter really behaves: a removed
    /// row's handle refuses, and the next row never reuses the id — ids come
    /// from the game's one monotonic counter (`take_entity_id`,
    /// game/network/lan_entities.rs:25: `next_entity += 1`, never decreased,
    /// and `EntityRef`'s contract says ids are never reused while the world
    /// is open), so the old handle has nothing to latch onto. Red without
    /// the resident lookup refusing a gone id: the old handle would resolve.
    #[test]
    #[ignore = "daily"]
    fn removed_general_row_refuses_its_handle_and_the_next_row_takes_a_new_id() {
        with_game(|game| {
            let uuid = [21, 22, 23, 24];
            game.resident_entities.general.rows.push(row(42, uuid, 7.0));
            let handle = game.entity(42).expect("the row became a handle");
            assert_eq!(handle.id(), 42);
            assert!(game.native_entity_target(handle).is_some());

            // The row leaves; the handle refuses.
            game.resident_entities
                .general
                .rows
                .retain(|row| row.id != 42);
            assert!(game.entity(42).is_none());
            assert!(game.native_entity_target(handle).is_none());

            // The next row is id 43 — the counter moved on, the same UUID
            // notwithstanding — and the old handle still refuses.
            game.resident_entities.general.rows.push(row(43, uuid, 7.0));
            let fresh = game.entity(43).expect("the next row became a handle");
            assert_ne!(fresh.id(), handle.id());
            assert!(
                game.native_entity_target(handle).is_none(),
                "the old handle resolved the next row"
            );
            let target = game
                .native_entity_target(fresh)
                .expect("the next row's handle resolves");
            assert!(matches!(target, Target::General(43)));
            let next = game
                .resident_entities
                .general
                .rows
                .iter()
                .find(|row| row.id == 43)
                .expect("the next row is in its list");
            assert_eq!(next.fall_distance, 7.0);
        });
    }

    /// A handle from an earlier epoch of the world — what a reload's restore
    /// of the same UUID leaves in a mod's hands — refuses by the epoch check
    /// alone, before any row is consulted.
    #[test]
    #[ignore = "daily"]
    fn general_handle_of_an_earlier_epoch_refuses_by_the_epoch_check() {
        with_game(|game| {
            game.resident_entities
                .general
                .rows
                .push(row(42, [21, 22, 23, 24], 7.0));
            let handle = game.entity(42).expect("the row became a handle");
            assert!(game.native_entity_target(handle).is_some());
            // A reload opens a new epoch; a handle minted against the old one
            // refuses even though a row of the same UUID and id is resident.
            let stale = EntityRef::new(WorldEpoch::new(handle.epoch().get() + 1), handle.id());
            assert!(game.native_entity_target(stale).is_none());
        });
    }
}
