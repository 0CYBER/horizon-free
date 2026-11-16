//! Entities ticking for native mods (`native_entity_tick_pre`, `native_entity_tick_post`:
//! NeoForge's `EntityTickEvent.Pre` and `.Post`). A family of entities ticks in its own loop,
//! and the loops of the mobs and the items hold their lists out of `Game`, so a family hands
//! its ticking entities to a `TickRound` before the loop and hears the Posts after it; the
//! minecarts and boats, whose loops can call the hooks, ask for each one in turn.
//!
//! Each entity's position writes are filed for `entity_entering_section`
//! (`entity_sections.rs`) while its tick is still running: a round keeps its entities while
//! either hook is listened to.
use super::Game;
use horizon_client::mobs::MobType;
use std::sync::Arc;

/// The type of an entity in a round, as the hooks name it.
pub(super) enum TickKind {
    Fixed(&'static str),
    Mob(Arc<MobType>),
}

impl TickKind {
    fn id(&self) -> &str {
        match self {
            Self::Fixed(id) => id,
            Self::Mob(kind) => &kind.id,
        }
    }
}

/// The entities of one family in one tick.
#[derive(Default)]
pub(in crate::game) struct TickRound {
    /// Whether this game is a server the tick hooks hear: otherwise no Pre or Post is told.
    listening: bool,
    /// Whether `entity_entering_section` is listened to on this server: each entity's position
    /// write is filed and told while its tick is still running.
    sections: bool,
    /// The entities the Pre let through, in order, for the Post.
    ticking: Vec<(u32, TickKind)>,
    /// The entities a Pre canceled.
    canceled: Vec<u32>,
    /// The items whose part ended their tick (`on_entity_item_update`): they are not
    /// canceled, so they still hear their Post.
    halted: Vec<u32>,
}

impl TickRound {
    /// Whether a mod canceled the tick of entity `id`.
    pub(super) fn canceled(&self, id: u32) -> bool {
        !self.canceled.is_empty() && self.canceled.contains(&id)
    }
    /// Whether a part ended the tick of item `id` (`on_entity_item_update`).
    pub(super) fn halted(&self, id: u32) -> bool {
        !self.halted.is_empty() && self.halted.contains(&id)
    }
    /// A part ended the tick of item `id`.
    pub(super) fn halt(&mut self, id: u32) {
        self.halted.push(id);
    }
    /// Whether `entity_entering_section` is listened to: each entity's position updates are filed.
    pub(super) fn sections(&self) -> bool {
        self.sections
    }
    /// Whether the round keeps the entities that tick: a mod hears their ticks or their moves.
    pub(super) fn listening(&self) -> bool {
        self.listening || self.sections
    }
    /// The entity `id`, told its Pre, went before it ticked (an item merged into an earlier
    /// one): it hears no Post.
    pub(super) fn not_ticked(&mut self, id: u32) {
        if self.listening() {
            self.ticking.retain(|(other, _)| *other != id);
        }
    }
}

impl Game<'_> {
    /// A round for a family about to tick: on a server only (the server thread or a dedicated
    /// server, never a client), and only while a mod listens
    /// (`native_entity_tick_listened`). Its list is kept between ticks.
    pub(super) fn native_tick_round(&mut self) -> TickRound {
        let server = self.serving.is_some() || self.options.dedicated.is_some();
        let mut listened = false;
        if server {
            self.native_entity_tick_listened(&mut listened);
        }
        let listening = server && listened;
        let sections = self.is_logical_server() && self.entity_section_scope().is_some();
        TickRound {
            listening,
            sections,
            ticking: if listening || sections {
                std::mem::take(&mut self.creatures.tick_buffer)
            } else {
                Vec::new()
            },
            canceled: Vec::new(),
            halted: Vec::new(),
        }
    }

    /// `native_entity_tick_pre` for the entity `id` of `kind` about to tick in a family that
    /// hears its Posts together: whether it ticks.
    pub(super) fn native_tick_pre_batch(
        &mut self,
        round: &mut TickRound,
        id: u32,
        kind: TickKind,
    ) -> bool {
        if !round.listening() {
            return true;
        }
        let mut canceled = false;
        if round.listening {
            self.native_entity_tick_pre(self.dim.id(), id, kind.id(), &mut canceled);
        }
        if canceled {
            round.canceled.push(id);
        } else {
            round.ticking.push((id, kind));
        }
        !canceled
    }

    /// `native_entity_tick_post` for each entity the round let through after its position writes
    /// have already filed and told their crossings.
    pub(super) fn native_tick_post_batch(&mut self, mut round: TickRound) {
        if !round.listening() {
            return;
        }
        let dimension = self.dim.id();
        for (id, kind) in &round.ticking {
            if round.listening {
                self.native_entity_tick_post(dimension, *id, kind.id());
            }
        }
        round.ticking.clear();
        self.creatures.tick_buffer = round.ticking;
    }

    /// `native_entity_tick_pre` for one entity about to tick: whether it does.
    pub(super) fn native_tick_pre_one(
        &mut self,
        round: &mut TickRound,
        id: u32,
        kind: &str,
    ) -> bool {
        if !round.listening {
            return true;
        }
        let mut canceled = false;
        self.native_entity_tick_pre(self.dim.id(), id, kind, &mut canceled);
        if canceled {
            round.canceled.push(id);
        }
        !canceled
    }

    /// `native_entity_tick_post` for one entity that ticked, after its move was filed and told.
    pub(super) fn native_tick_post_one(&mut self, round: &TickRound, id: u32, kind: &str) {
        if round.listening {
            self.native_entity_tick_post(self.dim.id(), id, kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round() -> TickRound {
        TickRound {
            listening: true,
            ..TickRound::default()
        }
    }

    /// A canceled entity is skipped, and one that went before its tick (an item merged into an
    /// earlier one) hears no Post; a round nobody listens to keeps nothing.
    #[test]
    fn a_round_keeps_the_entities_that_tick() {
        let mut r = round();
        r.ticking.push((1, TickKind::Fixed("minecraft:item")));
        r.ticking.push((2, TickKind::Fixed("minecraft:item")));
        r.canceled.push(3);
        assert!(r.canceled(3));
        assert!(!r.canceled(1));
        r.not_ticked(2);
        let left: Vec<_> = r
            .ticking
            .iter()
            .map(|(id, kind)| (*id, kind.id()))
            .collect();
        assert_eq!(left, [(1, "minecraft:item")]);
        let mut quiet = TickRound::default();
        assert!(!quiet.listening());
        quiet.not_ticked(1);
        assert!(quiet.ticking.is_empty() && !quiet.canceled(1));
    }
}
