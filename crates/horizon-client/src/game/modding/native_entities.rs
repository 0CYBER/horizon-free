//! Entities for native server mods (docs/modding/src/how-to/native-mods.md, "Entities"): spawn
//! any entity from its compound as `/summon` does, and move, show or remove
//! the entities Horizon keeps without simulating them (displays, markers,
//! interactions); `native_display_refs.rs` poses displays. They act where
//! the world is simulated, in the level in place, which each names by its
//! `dimension` (`minecraft:overworld`) as `native_set_block` does.
use super::{
    Game,
    commands::{Target, place_summoned},
};
use horizon_core::geometry::Vec3;
use horizon_core::nbt::{Compound, Tag};

/// Whether a position is one `/summon` takes.
fn position_ok(pos: [f64; 3]) -> bool {
    pos.iter().all(|v| v.is_finite())
        && pos[0].abs() < 30_000_000.0
        && pos[2].abs() < 30_000_000.0
        && (-20_000_000.0..20_000_000.0).contains(&pos[1])
}

/// The id of a summoned stack's root: its command target's, else `first`,
/// the first id handed out, when an entity no command target stands for
/// (`live`) holds it.
fn root_id(target: Option<Target>, first: u32, live: impl Fn(u32) -> bool) -> Option<u32> {
    match target {
        Some(t) => target_id(t),
        None => live(first).then_some(first),
    }
}

/// The entity id a summoned stack's root is known by.
pub(super) fn target_id(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Item(id)
        | Target::Orb(id)
        | Target::Projectile(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Crystal(id)
        | Target::Dragon(id)
        | Target::DragonPart(id)
        | Target::Tnt(id)
        | Target::Cushion(id)
        | Target::Falling(id)
        | Target::Rocket(id)
        | Target::Trident(id)
        | Target::Spawner(id)
        | Target::Cloud(id)
        | Target::Hook(id)
        | Target::Knot(id)
        | Target::General(id) => Some(id),
        Target::Player | Target::Guest(_) => None,
    }
}

#[allow(dead_code)]
impl Game<'_> {
    /// Whether this game simulates the level `dimension`.
    fn native_simulates_level(&self, dimension: &str) -> bool {
        (self.serving.is_some() || self.options.dedicated.is_some()) && dimension == self.dim.id()
    }
    /// Spawns an entity of `kind` (`minecraft:item_display`) at `pos` in the
    /// level in place, `dimension`, from `nbt` (its compound, as `/summon
    /// <kind> <pos> <nbt>` takes it; passengers included): its id. None on a
    /// client's copy of the world, for another dimension, a position outside
    /// `/summon`'s bounds, a chunk that isn't loaded, a kind Horizon can't
    /// load, one a mod kept out (`native_entity_join_level`), or with the entity ids exhausted.
    /// Inside a transaction the id comes back at once: a join a mod cancels at the root commit
    /// leaves its handle stale.
    pub(super) fn summon(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        nbt: Compound,
    ) -> Option<u32> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.native_simulates_level(dimension) || !position_ok(pos) {
            return None;
        }
        let chunk = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
        if !self.world.chunks.contains_key(&chunk) {
            return None;
        }
        let mut tag = nbt;
        tag.insert("id".into(), Tag::String(kind.into()));
        // As `/summon` does: every entity of the stack, passengers included,
        // stands at `pos`.
        let mut tag = Tag::Compound(tag);
        place_summoned(&mut tag, pos);
        let from = self.mobs.len();
        let first = self.next_entity + 1;
        let deferred = (self.tnts.len(), self.falling.len(), self.rockets.len());
        let target = self.spawn_entity_from_nbt(tag);
        // These roots take their id as they load, and a rocket has no command
        // Target variant. Select the newly appended root, never an older waiting entity.
        let root = match kind {
            "minecraft:tnt" if deferred.0 < self.tnts.len() => {
                Some(super::entity_residency::assign_resident_id!(
                    self.tnts,
                    deferred.0,
                    e,
                    &mut self.next_entity,
                    None
                ))
            }
            "minecraft:falling_block" if deferred.1 < self.falling.len() => {
                Some(super::entity_residency::assign_resident_id!(
                    self.falling,
                    deferred.1,
                    f,
                    &mut self.next_entity,
                    Some(&f.attachment_lifetime)
                ))
            }
            "minecraft:firework_rocket" if deferred.2 < self.rockets.len() => {
                Some(super::entity_residency::assign_resident_id!(
                    self.rockets,
                    deferred.2,
                    e,
                    &mut self.next_entity,
                    None
                ))
            }
            _ => None,
        };
        self.native_mobs_joined(from, false);
        // Otherwise the root is the first entity spawned, also for kinds no
        // command target stands for (experience orbs, end crystals, cushions). An id handed out to an
        // entity a mod then kept out is not given back, so the root must be
        // there.
        root.or_else(|| {
            root_id(target, first, |id| {
                self.orbs.iter().any(|o| o.id == id)
                    || self.crystals.iter().any(|c| c.id == id)
                    || self.cushions.iter().any(|c| c.id == id)
            })
        })
    }
}

/// The entities the walk does not hold: Horizon keeps these in lists of their own.
#[derive(Clone, Copy)]
enum Apart {
    Crystal(usize),
    Cushion(usize),
    Painting(usize),
    Frame(usize),
    Dragon,
}

/// One entity a mod is told of: in the walk, or apart from it.
#[derive(Clone, Copy)]
enum Slot {
    Walk(super::entity::EntityHandle),
    Apart(Apart),
}

/// Entities joining and leaving the level (`native_entity_join_level`,
/// `native_entity_leave_level`), for every kind in the entity walk and the
/// five kept apart from it. Kept out of `native_hooks.rs`, whose lines may not hold macros.
impl Game<'_> {
    /// Whether an entity may have joined since the last announcement. A join
    /// in another level may cause one harmless walk of this level.
    fn joins_pending(&self) -> bool {
        horizon_client::join_seq::JoinSeq::latest() > self.persist.joined_seq
    }
    /// Tells `native_entity_join_level` once for every entity added since the
    /// last call, of any kind, in insertion order. A mod that cancels takes
    /// the entity back out here,
    /// before it ticks again, and `/summon` of it fails (`joins_canceled`).
    /// `loaded_from_disk` is for the entities that came with a chunk. Returns
    /// whether `watch` (an id, or 0) is still in the level. Walks the lists
    /// with static dispatch and allocates nothing but the kind name of an
    /// entity it announces; nothing runs when no mod is loaded, on a client
    /// of a server (which tells nothing), or when nothing is new.
    /// Inside a transaction open in this level, construction is still told now, and the join
    /// at its root commit (`transactions/entities.rs`), with this call's `loaded_from_disk`, in
    /// insertion order and before the transaction's removals take effect; the entity is kept
    /// until then. The same holds for a call a listener makes while that commit is told: its
    /// joins are told after the batch being told. A `through` is the commit's own flush of one
    /// batch, never held.
    pub(super) fn native_joins_flush(&mut self, loaded_from_disk: bool, watch: u32) -> bool {
        self.flush_joins(loaded_from_disk, watch, &[], None)
    }
    /// [`Self::native_joins_flush`], never announcing the entities `removed` names and, with a
    /// `through`, none that joined after it: a root commit's, for the entities its transaction
    /// added and removed, each held flush keeping its own `loaded_from_disk`.
    pub(in crate::game) fn flush_joins(
        &mut self,
        loaded_from_disk: bool,
        watch: u32,
        removed: &[u32],
        through: Option<horizon_client::join_seq::JoinSeq>,
    ) -> bool {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() || !self.joins_pending() {
            return true;
        }
        self.assign_pending_join_ids();
        if through.is_none()
            && self.hold_announcement(super::transactions::Announcement::Joins {
                loaded_from_disk,
                through: horizon_client::join_seq::JoinSeq::latest(),
            })
        {
            let mut mark = self.persist.joined_seq;
            while let Some((slot, id, seq)) = self.next_unannounced(mark, &[], None) {
                mark = seq;
                let kind = self.slot_kind_and_position(slot).0;
                self.construct_joining(id, &kind);
            }
            return true;
        }
        let latest = horizon_client::join_seq::JoinSeq::latest();
        let observed = through.map_or(latest, |through| through.min(latest));
        let mut kept = true;
        while let Some((slot, id, seq)) =
            self.next_unannounced(self.persist.joined_seq, removed, through)
        {
            self.persist.joined_seq = seq;
            let (kind, at) = self.slot_kind_and_position(slot);
            let mut canceled = false;
            let dimension = self.dim.id();
            self.construct_joining(id, &kind);
            self.native_entity_join_level(
                dimension,
                id,
                &kind,
                [at.x, at.y, at.z],
                loaded_from_disk,
                &mut canceled,
            );
            let spawn = self
                .native_lifecycle
                .pending_item_spawns
                .iter()
                .position(|entry| entry.0 == id)
                .map(|index| self.native_lifecycle.pending_item_spawns.remove(index));
            if canceled {
                self.persist.joins_canceled += 1;
                self.forget_entity(id);
                // Handles are indexes: the entity is taken out before any
                // other change to its list.
                self.discard_slot(slot);
                kept &= id != watch;
            } else {
                self.remember_join(id, kind.into_owned());
                // `addEntity` files it under its section, untold.
                self.entity_section_joined(id, [at.x, at.y, at.z]);
                if let Some((id, item, count, position)) = spawn {
                    self.native_item_spawn(id, &item, count, position);
                }
            }
        }
        // A held toss is not past yet: the mark stays before it.
        if self.native_lifecycle.held_joins.is_empty() {
            self.persist.joined_seq = self.persist.joined_seq.max(observed);
        }
        kept
    }

    /// `Entity.<init>` comes before `addFreshEntity`: an entity that started on a path that
    /// doesn't tell it (`entity_initialization.rs`) is told here.
    fn construct_joining(&mut self, id: u32, kind: &str) {
        self.initialize_entity(mod_api::Holder::Entity(id), kind);
        // Constructor writes were canonical but unbound before insertion.
        // The real lifetime is now resident, before join listeners query it.
        if self
            .entity_attachments
            .get(&id)
            .is_some_and(|map| map.has_index_entries())
            && let Err(error) = self.bind_resident_entity_attachments(id)
        {
            log::warn!("constructor attachment binding failed for entity {id}: {error:?}");
        }
    }

    fn assign_pending_join_ids(&mut self) {
        self.assign_dragon_and_crystal_ids();
        use super::entity_residency::assign_resident_ids;
        assign_resident_ids!(self.tnts, t, &mut self.next_entity, None);
        assign_resident_ids!(
            self.falling,
            f,
            &mut self.next_entity,
            Some(&f.attachment_lifetime)
        );
        assign_resident_ids!(
            self.creatures.projectiles,
            p,
            &mut self.next_entity,
            Some(&p.attachment_lifetime)
        );
        assign_resident_ids!(self.tridents, t, &mut self.next_entity, None);
        assign_resident_ids!(self.rockets, r, &mut self.next_entity, None);
    }
    /// The entity inserted earliest after `mark` (and, with a `through`, no later than it), but
    /// the `skipped`.
    fn next_unannounced(
        &self,
        mark: horizon_client::join_seq::JoinSeq,
        skipped: &[u32],
        through: Option<horizon_client::join_seq::JoinSeq>,
    ) -> Option<(Slot, u32, horizon_client::join_seq::JoinSeq)> {
        use super::entity::Entity;
        use horizon_client::join_seq::{JoinSeq, Joined};
        let mut best: Option<(Slot, u32, JoinSeq)> = None;
        let mut offer = |slot: Slot, id: u32, seq: JoinSeq| {
            if skipped.contains(&id) || self.persist.removals.iter().chain(self.persist.destroying_vehicles.iter()).any(|target| {
                matches!(*target, super::commands::Target::Boat(n) | super::commands::Target::Cart(n) if n == id)
            }) {
                return;
            }
            if seq > mark
                && through.is_none_or(|through| seq <= through)
                && best.is_none_or(|(_, _, previous)| seq < previous)
            {
                best = Some((slot, id, seq));
            }
        };
        for handle in self.entity_handles() {
            let (id, seq) = super::entity::with_entity!(self, handle, e => (e.id(), e.join_seq()));
            offer(Slot::Walk(handle), id, seq);
        }
        for (i, c) in self.crystals.iter().enumerate() {
            offer(Slot::Apart(Apart::Crystal(i)), c.id, c.join);
        }
        for (i, c) in self.cushions.iter().enumerate() {
            offer(Slot::Apart(Apart::Cushion(i)), c.id, c.join);
        }
        for (i, p) in self.paintings.iter().enumerate() {
            offer(Slot::Apart(Apart::Painting(i)), p.id, p.join);
        }
        for (i, f) in self.item_frames.iter().enumerate() {
            offer(Slot::Apart(Apart::Frame(i)), f.id, f.join);
        }
        if let Some(d) = self.dragon.as_ref() {
            offer(Slot::Apart(Apart::Dragon), d.id, d.join);
        }
        // A toss still being asked (`item_toss`) ends the flush: it and what joined after wait.
        best.filter(|&(_, id, _)| !self.native_lifecycle.held_joins.contains(&id))
    }

    fn remember_join(&mut self, id: u32, kind: String) {
        let index = self
            .persist
            .live
            .binary_search_by_key(&id, |entry| entry.0)
            .unwrap_or_else(|i| i);
        self.persist.live.insert(index, (id, kind, true));
    }
    /// The type id and position a mod is told of.
    fn slot_kind_and_position(&self, slot: Slot) -> (std::borrow::Cow<'static, str>, Vec3) {
        use super::entity::Entity;
        let block = |p: [i32; 3]| {
            Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            )
        };
        match slot {
            Slot::Walk(h) => {
                super::entity::with_entity!(self, h, e => (e.native_type(), e.position()))
            }
            Slot::Apart(Apart::Crystal(i)) => {
                ("minecraft:end_crystal".into(), self.crystals[i].position)
            }
            Slot::Apart(Apart::Cushion(i)) => {
                ("minecraft:cushion".into(), self.cushions[i].position)
            }
            Slot::Apart(Apart::Painting(i)) => {
                ("minecraft:painting".into(), block(self.paintings[i].pos))
            }
            Slot::Apart(Apart::Frame(i)) => {
                let f = &self.item_frames[i];
                let kind = if f.glow {
                    "minecraft:glow_item_frame"
                } else {
                    "minecraft:item_frame"
                };
                (kind.into(), block(f.pos))
            }
            Slot::Apart(Apart::Dragon) => (
                "minecraft:ender_dragon".into(),
                self.dragon.as_ref().map_or(Vec3::ZERO, |d| d.position),
            ),
        }
    }
    /// Takes the entity out of the level, before it ever ticked.
    fn discard_slot(&mut self, slot: Slot) {
        match slot {
            Slot::Walk(h) => self.discard_entity(h),
            Slot::Apart(Apart::Crystal(i)) => drop(self.crystals.remove(i)),
            Slot::Apart(Apart::Cushion(i)) => drop(self.cushions.remove(i)),
            Slot::Apart(Apart::Painting(i)) => drop(self.paintings.remove(i)),
            Slot::Apart(Apart::Frame(i)) => drop(self.item_frames.remove(i)),
            Slot::Apart(Apart::Dragon) => self.discard_resident_dragon(),
        }
    }
    /// Entities that were moved, not added (a portal's arrival keeps being
    /// silent): everything now in the level counts as announced, and will
    /// be told when it leaves.
    pub(super) fn native_joins_skip(&mut self) {
        let observed = horizon_client::join_seq::JoinSeq::latest();
        if self.remote.is_none() && !mod_api::__private::mods().is_empty() {
            self.assign_pending_join_ids();
            while let Some((slot, id, seq)) =
                self.next_unannounced(self.persist.joined_seq, &[], None)
            {
                self.persist.joined_seq = seq;
                let kind = self.slot_kind_and_position(slot).0.into_owned();
                self.remember_join(id, kind);
            }
        }
        self.persist.joined_seq = self.persist.joined_seq.max(observed);
    }
    /// An entity taken out of the level without leaving it (an item through
    /// an End gateway): forgotten, not told of.
    pub(super) fn native_leave_skip(&mut self, id: u32) {
        self.entity_section_forget(id);
        if let Ok(i) = self.persist.live.binary_search_by_key(&id, |l| l.0) {
            self.persist.live.remove(i);
        }
    }
    /// Entity `id` has left the level: told once, if mods were told it joined.
    pub(super) fn native_entity_left(&mut self, id: u32) {
        if self.remote.is_some()
            || self.hold_announcement(super::transactions::Announcement::Left(id))
        {
            return;
        }
        self.entity_left_now(id);
    }
    /// [`Self::native_entity_left`] told now, though a transaction or a commit holds the rest:
    /// the root commit's own telling.
    pub(in crate::game) fn entity_left_now(&mut self, id: u32) {
        self.entity_section_forget(id);
        if let Ok(i) = self.persist.live.binary_search_by_key(&id, |l| l.0) {
            let (id, kind, _) = self.persist.live.remove(i);
            let dimension = self.dim.id();
            self.native_entity_leave_level(dimension, id, &kind);
        }
    }
    /// Tells `native_entity_leave_level` for every announced entity that is
    /// no longer in any list (died, despawned, picked up, burnt, blown up, gone
    /// through a portal, unloaded), lowest id first. Marks the entities still
    /// there in the registry and tells the rest: no allocation, and nothing
    /// runs when no mod is loaded or none was announced.
    pub(super) fn native_leaves_flush(&mut self) {
        self.native_removals_flush();
        if self.persist.live.is_empty()
            || self.remote.is_some()
            || self.hold_announcement(super::transactions::Announcement::Leaves)
        {
            return;
        }
        self.leaves_now();
    }
    /// [`Self::native_leaves_flush`] past its guards and told now, though a transaction or a
    /// commit holds the rest: the root commit's own telling.
    pub(in crate::game) fn leaves_now(&mut self) {
        use super::entity::Entity;
        for l in &mut self.persist.live {
            l.2 = false;
        }
        let live = &mut self.persist.live;
        let mut seen = |id: u32| {
            if let Ok(i) = live.binary_search_by_key(&id, |l| l.0) {
                live[i].2 = true;
            }
        };
        super::entity::for_each_entity!(super::entity::entity_lists_of!(self), e => {
            seen(e.id());
        });
        self.crystals.iter().for_each(|c| seen(c.id));
        self.cushions.iter().for_each(|c| seen(c.id));
        self.paintings.iter().for_each(|p| seen(p.id));
        self.item_frames.iter().for_each(|f| seen(f.id));
        if let Some(d) = self.dragon.as_ref() {
            seen(d.id);
        }
        while let Some(i) = self.persist.live.iter().position(|l| !l.2) {
            let (id, kind, _) = self.persist.live.remove(i);
            self.entity_section_forget(id);
            let dimension = self.dim.id();
            self.native_entity_leave_level(dimension, id, &kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::ServerHost;

    // NBT loads assign ids; reinstall an unbound payload as a pending owner.
    macro_rules! pending {
        ($list:expr) => {{
            let mut owner = $list.pop().unwrap();
            owner.id = 0;
            $list.push(owner);
        }};
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn pending_ids_keep_counter_order_and_assign_zero_only_once() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for (kind, nbt) in [
                ("minecraft:tnt", "{fuse:80s}"),
                (
                    "minecraft:falling_block",
                    "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
                ),
                ("minecraft:arrow", "{}"),
                ("minecraft:trident", "{}"),
                ("minecraft:firework_rocket", "{LifeTime:20}"),
            ] {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
            }
            pending!(game.tnts);
            pending!(game.falling);
            pending!(game.creatures.projectiles);
            pending!(game.tridents);
            pending!(game.rockets);
            let base = game.next_entity;
            let joined = game.persist.joined_seq;
            let announced = game.persist.live.clone();
            game.assign_pending_join_ids();
            let ids = [
                game.tnts[0].id,
                game.falling[0].id,
                game.creatures.projectiles[0].id,
                game.tridents[0].id,
                game.rockets[0].id,
            ];
            assert_eq!(ids, [base + 1, base + 2, base + 3, base + 4, base + 5]);
            let mut identities = super::super::entity::taken_pass_tests::SlotIdentities::default();
            identities.observe(
                &mut game.falling,
                game.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
            game.assign_pending_join_ids();
            identities.observe(
                &mut game.falling,
                game.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
            assert_eq!(game.next_entity, base + 5);
            assert_eq!(game.persist.joined_seq, joined);
            assert_eq!(game.persist.live, announced);
            assert_eq!(
                [
                    game.tnts[0].id,
                    game.falling[0].id,
                    game.creatures.projectiles[0].id,
                    game.tridents[0].id,
                    game.rockets[0].id
                ],
                ids
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn native_roots_keep_the_loaded_id_and_do_not_allocate_twice() {
        use horizon_client::chunk::Chunk;
        for (kind, nbt) in [
            ("minecraft:tnt", "{fuse:80s}"),
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:firework_rocket", "{LifeTime:20}"),
        ] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                game.summon_with_nbt(kind, [2.0, 70.0, 1.0], nbt).unwrap();
                match kind {
                    "minecraft:tnt" => pending!(game.tnts),
                    "minecraft:falling_block" => pending!(game.falling),
                    _ => pending!(game.rockets),
                }
                let base = game.next_entity;
                let (Tag::Compound(tag), _) = horizon_core::nbt::parse_snbt(nbt).unwrap() else {
                    unreachable!()
                };
                let id = game
                    .summon(game.dim.id(), kind, [1.0, 70.0, 1.0], tag)
                    .unwrap();
                assert_eq!(id, base + 1);
                assert_eq!(game.next_entity, base + 1);
                assert!(game.entity(id).is_some());
                let waiting = match kind {
                    "minecraft:tnt" => game.tnts[0].id,
                    "minecraft:falling_block" => game.falling[0].id,
                    _ => game.rockets[0].id,
                };
                assert_eq!(waiting, 0);
                if kind == "minecraft:falling_block" {
                    let (Tag::Compound(unknown_state), _) = horizon_core::nbt::parse_snbt(
                        "{BlockState:{Name:\"test:missing\"},Time:1}",
                    )
                    .unwrap() else {
                        unreachable!()
                    };
                    let fallback = game
                        .summon(game.dim.id(), kind, [1.0, 70.0, 1.0], unknown_state)
                        .unwrap();
                    assert_eq!(fallback, base + 2);
                    assert_eq!(game.next_entity, base + 2);
                    assert_eq!(game.falling.len(), 3);
                    assert_eq!(game.falling[0].id, 0);
                    assert_eq!(game.falling[1].id, id);
                    assert_eq!(game.falling[2].id, fallback);
                    assert_eq!(
                        game.falling[2].state,
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:sand")
                            .unwrap()
                    );
                }
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn portal_ids_follow_only_the_existing_moving_entity_filters() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for _ in 0..3 {
                game.summon_with_nbt("minecraft:trident", [1.0, 70.0, 1.0], "{}")
                    .unwrap();
                game.summon_with_nbt(
                    "minecraft:firework_rocket",
                    [1.0, 70.0, 1.0],
                    "{LifeTime:20}",
                )
                .unwrap();
            }
            for t in &mut game.tridents {
                t.id = 0;
                t.in_ground = true;
                t.previous = t.position;
            }
            game.tridents[0].in_ground = false;
            game.tridents[2].previous.x -= 1.0;
            for r in &mut game.rockets {
                r.id = 0;
                r.attached = false;
                r.attached_to = None;
            }
            game.rockets[1].attached_to = Some(123);
            game.rockets[2].attached = true;
            let base = game.next_entity;
            game.find_ridden_vehicle_portals(&[999]);
            assert_eq!(game.next_entity, base);
            game.find_entity_portals();
            assert_eq!(
                game.tridents.iter().map(|t| t.id).collect::<Vec<_>>(),
                [base + 1, 0, base + 2]
            );
            assert_eq!(
                game.rockets.iter().map(|r| r.id).collect::<Vec<_>>(),
                [base + 3, 0, 0]
            );
            game.find_entity_portals();
            assert_eq!(game.next_entity, base + 3);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn pending_assignment_refuses_a_bound_zero_owner_before_consuming_an_id() {
        use horizon_mod_runtime::attachments::{
            AttachmentType, Attachments, Holder, index::AttachmentIndex,
        };
        use std::sync::{Arc, Mutex};
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "pending_id",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        for (kind, nbt) in [
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:arrow", "{}"),
        ] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
                let mut values = Attachments::default();
                VALUE.set(&mut values, Holder::Entity(0), 7).unwrap();
                let index = Arc::new(Mutex::new(AttachmentIndex::default()));
                let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 0);
                macro_rules! bind_zero {
                    ($list:expr) => {{
                        let mut owner = $list.pop().unwrap();
                        owner.id = 0;
                        owner.attachment_lifetime.bind(&mut values, &index, holder);
                        $list.push(owner);
                    }};
                }
                if kind == "minecraft:arrow" {
                    bind_zero!(game.creatures.projectiles);
                } else {
                    bind_zero!(game.falling);
                }
                let before = game.next_entity;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.assign_pending_join_ids()
                }));
                assert!(result.is_err());
                assert_eq!(game.next_entity, before);
                assert!(game.falling.iter().all(|owner| owner.id == 0));
                assert!(game.creatures.projectiles.iter().all(|owner| owner.id == 0));
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn joins_follow_insertion_order_and_canceled_entities_never_leave() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use mod_api::{self, EntityJoinLevelEvent, EntityLeaveLevelEvent, Flow, Hooks};
        use std::sync::{Arc, Mutex};

        type Joins = Arc<Mutex<Vec<(u32, String, bool)>>>;
        struct Observer {
            joins: Joins,
            leaves: Arc<Mutex<Vec<u32>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                self.joins.lock().unwrap().push((
                    event.entity,
                    event.kind.into(),
                    event.loaded_from_disk,
                ));
                if event.kind == "minecraft:sheep" {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn entity_leave_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityLeaveLevelEvent<'_>,
            ) {
                self.leaves.lock().unwrap().push(event.entity);
            }
        }
        let joins = Joins::default();
        let leaves = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "join-order-observer",
            Box::new(Observer {
                joins: joins.clone(),
                leaves: leaves.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_joins_skip();
                let base = game.next_entity;
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let sheep = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
                let child = Mob::new(base + 1, cow.clone(), Vec3::ZERO, 0.0);
                game.next_entity += 2;
                game.mobs.push(Mob::new(base + 2, cow, Vec3::ZERO, 0.0));
                game.native_joins_flush(false, 0);
                game.mobs.push(child);
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(
                    *joins.lock().unwrap(),
                    vec![
                        (base + 2, "minecraft:cow".into(), false),
                        (base + 1, "minecraft:cow".into(), false)
                    ]
                );
                game.next_entity += 1;
                game.mobs.push(Mob::new(base + 3, sheep, Vec3::ZERO, 0.0));
                assert!(!game.native_joins_flush(true, base + 3));
                assert!(!game.mobs.iter().any(|mob| mob.id == base + 3));
                game.native_entity_left(base + 3);
                assert!(leaves.lock().unwrap().is_empty());
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&(base + 3, "minecraft:sheep".into(), true))
                );

                game.crystals.push(super::super::dragon::Crystal {
                    fall: Default::default(),
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 4,
                    position: Vec3::ZERO,
                    show_bottom: true,
                    time: 0,
                    beam: None,
                    invulnerable: false,
                });
                game.cushions.push(horizon_client::entities::Cushion {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 5,
                    position: Vec3::ZERO,
                    pos: [0; 3],
                    yaw: 0.0,
                    color: 0,
                    check_interval: 0,
                    custom_name: None,
                });
                game.paintings.push(horizon_client::entities::Painting {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 6,
                    pos: [0; 3],
                    facing: 0,
                    variant: "minecraft:kebab".into(),
                    width: 1,
                    height: 1,
                    check_interval: 0,
                });
                game.item_frames.push(horizon_client::entities::ItemFrame {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 7,
                    pos: [0; 3],
                    facing: 0,
                    glow: false,
                    item: None,
                    rotation: 0,
                    drop_chance: 1.0,
                    fixed: false,
                    invisible: false,
                    custom_name: None,
                    random: horizon_core::random::LegacyRandom::new(0),
                    check_interval: 0,
                });
                let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
                dragon.id = base + 8;
                dragon.join = horizon_client::join_seq::JoinSeq::next();
                game.dragon.replace(dragon);
                game.next_entity = base + 8;
                game.native_joins_flush(true, 0);
                game.native_joins_flush(true, 0);
                let seen = joins.lock().unwrap();
                assert_eq!(
                    &seen[3..],
                    &[
                        (base + 4, "minecraft:end_crystal".into(), true),
                        (base + 5, "minecraft:cushion".into(), true),
                        (base + 6, "minecraft:painting".into(), true),
                        (base + 7, "minecraft:item_frame".into(), true),
                        (base + 8, "minecraft:ender_dragon".into(), true)
                    ]
                );
                drop(seen);
                game.tnts.push(super::super::tnt::PrimedTnt {
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 0,
                    position: Vec3::ZERO,
                    previous: Vec3::ZERO,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    on_ground: false,
                    in_water: false,
                    stuck: Vec3::ZERO,
                    fall_distance: 0.0,
                    owner: None,
                    guest: None,
                    no_gravity: false,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(game.tnts.last().unwrap().id, base + 9);
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&(base + 9, "minecraft:tnt".into(), false))
                );
                assert_eq!(joins.lock().unwrap().len(), 9);
            });
        });
    }

    #[test]
    fn a_passenger_without_a_position_stands_where_the_stack_is_summoned() {
        let mut rider = Compound::new();
        rider.insert("id".into(), Tag::String("minecraft:pig".into()));
        let mut root = Compound::new();
        root.insert("id".into(), Tag::String("minecraft:pig".into()));
        root.insert("Passengers".into(), Tag::List(vec![Tag::Compound(rider)]));
        let mut tag = Tag::Compound(root);
        place_summoned(&mut tag, [1.5, 70.0, -2.5]);
        let pos = Tag::List(
            [1.5, 70.0, -2.5]
                .map(|v: f64| Tag::Double(v.to_bits()))
                .to_vec(),
        );
        let Tag::Compound(root) = &tag else { panic!() };
        assert_eq!(root.get(&"Pos".into()), Some(&pos));
        let Some(Tag::List(riders)) = root.get(&"Passengers".into()) else {
            panic!("no passengers");
        };
        let Tag::Compound(rider) = &riders[0] else {
            panic!()
        };
        assert_eq!(rider.get(&"Pos".into()), Some(&pos));
    }

    #[test]
    fn an_orb_root_has_an_id_only_while_it_is_in_the_level() {
        // Spawned: the orb holds the first id.
        assert_eq!(root_id(None, 36, |id| id == 36), Some(36));
        // A mod kept it out: id 36 was handed out, but no entity holds it.
        assert_eq!(root_id(None, 36, |_| false), None);
        // A command target answers for itself.
        assert_eq!(root_id(Some(Target::Mob(40)), 36, |_| false), Some(40));
        assert_eq!(root_id(Some(Target::Player), 36, |_| true), None);
    }

    #[test]
    fn summon_positions_are_summons() {
        assert!(position_ok([0.5, 64.0, 0.5]));
        assert!(!position_ok([3.0e7, 64.0, 0.5]));
        assert!(!position_ok([0.5, f64::NAN, 0.5]));
    }
}
