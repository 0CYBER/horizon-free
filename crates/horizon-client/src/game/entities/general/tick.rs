//! PURPOSE: the general entity rows' authoritative tick (arismoko/horizon#792 stage 4): one
//! pass over the rows in join order, each root's rider subtree inside its
//! tick.
//! WHY: vanilla `ServerLevel.tick` walks `entityTickList.forEach`, and
//! `tickNonPassenger` runs the entity's common tick, its tick, then each
//! rider recursively (`tickPassenger`: a rider is never a root). The tick
//! list is insertion-ordered, and its `ensureActiveIsNotIterated` keeps a
//! join made during the pass out of the pass, so Horizon's join order
//! (`join_seq.rs`) is that order and a row that joins mid-pass first ticks
//! the next pass.
//! REFS: vanilla `ServerLevel.tick`, `tickNonPassenger`, `tickPassenger`,
//! `EntityTickList`; `join_seq.rs`; `ordered_tick.rs`; `tools/rea_vanilla.py`
//! (`add` appends to the active map, `remove` takes out, and the passive
//! swap holds a mid-pass join for the next tick).
//! LOGS: none. READS-BY: `Game::tick_world`. ENDS-WHEN: every kind ticks
//! from the join-order walk.

use super::super::Game;
use super::super::commands::Target;
use super::{GeneralTypeCatalog, general_type_catalog};
use horizon_client::join_seq::JoinSeq;
use mod_api::EntityRef;

impl Game<'_> {
    /// The general rows' entity phase: roots in join order, each root's
    /// bookkeeping, subscribed server behavior and the global tick pair,
    /// then its riders — every passenger gets the same Pre, tick and Post
    /// inside its own ride tick, at any depth of a stack, in the flat order
    /// the maintainers ruled: vehicle Pre, tick, Post, then each passenger
    /// (Pre, tick, Post), then that passenger's own passengers, depth
    /// first. A canceled vehicle Pre skips only the vehicle: no
    /// bookkeeping, no tick, no Post — its passengers still ride. A row
    /// that joined this tick ticks from the next (the tick list's passive
    /// swap), rows outside entity-ticking chunks stay (`ServerLevel.tick`'s
    /// range gate), and a parked level never reaches here at all. A type
    /// slot resolved without subscribers costs its bookkeeping and nothing
    /// else: no per-row type lookup, call, or event allocation.
    pub(in crate::game) fn tick_general(&mut self) -> horizon_client::Result<()> {
        if self.resident_entities.general.rows.is_empty() {
            return Ok(());
        }
        // `begin_ordered_ticks`' cutoff: a row numbered at or above `born`
        // joined during this tick and first ticks the next.
        let born = self.ordered.born;
        let catalog = general_type_catalog();
        // One bit per type slot, resolved once per pass. The rows keep plain
        // slots, so the walk itself never names a type until a part hears it.
        let subscribed = general_tick_subscribed(catalog);
        let epoch = self.native_lifecycle.epoch();
        let entity_ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // The walk stays on the live list, as the ordered run's does: a
        // lifted pass would take every row's residency slot, and a part's
        // writes reach its row through that slot. What joins during the pass
        // lands at or above `born`, ticks from the next pass, and never
        // reaches this loop's rows.
        let total = self.resident_entities.general.rows.len();
        // The rider adjacency, resolved once into the reusable first-child
        // and next-sibling index: a stable world's pass allocates nothing
        // (the index rebuilds in place only when the rows or a mount
        // changed), and every edge is visited exactly once, through its
        // vehicle's chain — no row scans the list.
        self.resident_entities.general.riders.refresh(
            &self.resident_entities.general.rows,
            self.resident_entities.general.mount_writes,
        );
        for index in 0..total {
            let (id, slot, join, mount, at) = {
                let row = &self.resident_entities.general.rows[index];
                (row.id, row.type_slot, row.join, row.mount, row.position)
            };
            // A rider ticks inside its vehicle's tick (`tickNonPassenger`
            // never takes one as a root); a row of this tick's join waits
            // for the next pass.
            if mount.is_some() || join >= born {
                continue;
            }
            // ServerLevel.tick: only entities in entity-ticking chunks tick.
            if !entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                continue;
            }
            // Live before anything: an earlier row's callback can have
            // queued this row's removal — `entity_remove` leaves it in the
            // list until the sweep — or rewritten its mount. The ordered
            // run re-resolves its join before each entity
            // (`run_ordered_ticks`' `locate`), and the mob tick checks
            // `removed`; this walk re-reads the row by its index.
            if !general_row_live(self, index, mount) {
                continue;
            }
            let kind = catalog.key(slot);
            let ticks = !round.listening() || self.native_tick_pre_one(&mut round, id, kind);
            // A row that rides nothing and whose Pre was canceled skips its
            // bookkeeping too (the ruling's only-skip list); a Pre's
            // listeners may also have removed or remounted the row, and a
            // row that no longer ticks hears no Post, as a merged item's
            // does not (the round's own `not_ticked`).
            if ticks {
                if general_row_live(self, index, mount) {
                    // `Entity.commonTick`'s setOldPosAndRot: the row's
                    // previous position is its current one. The rest of the
                    // common tick (tick counting, invulnerable time) has no
                    // row field yet.
                    self.resident_entities.general.rows[index].previous =
                        self.resident_entities.general.rows[index].position;
                    if subscribed.get(slot).copied().unwrap_or(false)
                        && let Some(epoch) = epoch
                    {
                        self.dispatch_general_tick(kind, epoch, id);
                    }
                    if round.listening() {
                        self.native_tick_post_one(&round, id, kind);
                    }
                } else {
                    round.not_ticked(id);
                }
            }
            // The vehicle's canceled Pre does not stop its passengers: every
            // rider still takes its own ride tick.
            self.tick_general_riders(index, born, catalog, &subscribed, epoch, &mut round);
        }
        Ok(())
    }

    /// One vehicle's riders, each with its own ride tick: Pre, bookkeeping,
    /// tick, Post, then that rider's own riders, depth first — the flat
    /// order of the maintainers' ruling, not a nested one. A canceled
    /// rider's Pre skips only its entity tick and its Post: the rider still
    /// follows its vehicle (the bookkeeping runs), and its own passengers
    /// still tick; a rider removed in its own Pre — canceled or not —
    /// neither ticks nor walks its descendants. The parent's callbacks are
    /// re-checked at every edge (`tickPassenger`'s `getVehicle` gate): a
    /// rider the callback removed or dismounted does not tick in this pass.
    /// A rider that joined this tick waits for the next pass with its root
    /// — a tree joins whole.
    fn tick_general_riders(
        &mut self,
        vehicle: usize,
        born: JoinSeq,
        catalog: &GeneralTypeCatalog,
        subscribed: &[bool],
        epoch: Option<mod_api::WorldEpoch>,
        round: &mut crate::game::native_ticks::TickRound,
    ) {
        // The vehicle's rider chain, each edge once: the index was resolved
        // before the walk began, so rows joining mid-pass never appear here.
        let mut child = self.resident_entities.general.riders.first_child[vehicle];
        while let Some(index) = child {
            child = self.resident_entities.general.riders.next_sibling[index];
            let (id, slot, join, mount) = {
                let row = &self.resident_entities.general.rows[index];
                (row.id, row.type_slot, row.join, row.mount)
            };
            if join >= born {
                continue;
            }
            // `tickPassenger`'s own gate: the rider still rides this
            // vehicle, and no callback has queued its removal.
            if mount != Some(self.resident_entities.general.rows[vehicle].id)
                || !general_row_live(self, index, mount)
            {
                continue;
            }
            let kind = catalog.key(slot);
            let ticks = !round.listening() || self.native_tick_pre_one(round, id, kind);
            // Liveness is re-checked after the Pre whether it was canceled
            // or not: a rider removed in its own Pre — canceled or not —
            // neither ticks nor walks its descendants, and a row that no
            // longer ticks hears no Post.
            if !general_row_live(self, index, mount) {
                if ticks {
                    round.not_ticked(id);
                }
                continue;
            }
            // The bookkeeping runs even for a canceled rider: it still
            // follows its vehicle (the ruling).
            self.resident_entities.general.rows[index].previous =
                self.resident_entities.general.rows[index].position;
            if ticks {
                if subscribed.get(slot).copied().unwrap_or(false)
                    && let Some(epoch) = epoch
                {
                    self.dispatch_general_tick(kind, epoch, id);
                }
                if round.listening() {
                    self.native_tick_post_one(round, id, kind);
                }
            }
            self.tick_general_riders(index, born, catalog, subscribed, epoch, round);
        }
    }

    /// One row's subscribed behavior, contained: the dispatch runs in a
    /// transaction, so a native panic unwinding out of it (caught here, as
    /// the mob tick's is) or a wasm trap inside it (a wasm dispatch fails
    /// without unwinding) rolls back every write the row's parts made —
    /// their counters and listener edits — and nothing leaves the row. The
    /// pass's lift stays balanced either way, and the next row ticks as if
    /// nothing happened; a panicking subscriber is reported, not disabled.
    fn dispatch_general_tick(&mut self, kind: &str, epoch: mod_api::WorldEpoch, id: u32) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mod_api::run_transaction(self, |game, _| {
                let before = game.wasm_dispatch_failures.get();
                game.native_general_entity_tick(kind, EntityRef::new(epoch, id));
                (game.wasm_dispatch_failures.get() == before)
                    .then_some(())
                    .ok_or(())
            })
        }));
        match outcome {
            Ok(Ok(())) => {}
            // A wasm trap: the runtime poisoned the caller's group and told
            // the log; the transaction above aborted with this refusal.
            Ok(Err(())) => {
                log::error!(
                    "general entity {kind} #{id}: its tick's writes were rolled back after a wasm dispatch failure"
                );
            }
            // A native panic: the unwind aborted the transaction as it
            // passed, and the thread-local mod attribution named the part.
            Err(payload) => {
                let panic = payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "box".to_owned());
                log::error!(
                    "general entity {kind} #{id}: its tick's writes were rolled back after a native mod panicked: {panic}"
                );
            }
        }
    }
}

/// Whether each type slot has a part subscribed to the tick hook: nothing
/// at all when no part anywhere subscribes (the dispatch's own bit check),
/// so a parts-free world resolves nothing.
fn general_tick_subscribed(catalog: &GeneralTypeCatalog) -> Vec<bool> {
    if !mod_api::__private::has_part_hook(mod_api::HookId::GeneralEntityTick) {
        return Vec::new();
    }
    (0..catalog.len())
        .map(|slot| {
            mod_api::__private::entity_part_hooked(
                mod_api::HookId::GeneralEntityTick,
                "general_entity_tick",
                catalog.key(slot),
            )
        })
        .collect()
}

/// Whether the row at `index` is a live row still riding `mount`: re-read at
/// every step of the walk, because a listener's callback can queue the row's
/// removal — `entity_remove` leaves it in the list until the sweep — or
/// rewrite its mount. The row's identity is its place: nothing in the pass
/// reorders the storage, so an index read now is the row it names.
fn general_row_live(game: &Game<'_>, index: usize, mount: Option<u32>) -> bool {
    game.resident_entities
        .general
        .rows
        .get(index)
        .is_some_and(|row| {
            row.mount == mount && !game.persist.removals.contains(&Target::General(row.id))
        })
}
