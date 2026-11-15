//! The game layer's door of the move gate (arismoko/horizon#2414): who is moving, and the one
//! `Game::entity_move` the pushers and family ticks arm a move through. PR 1a
//! carries no hook — every decision is `Run` and the tick cost is one branch;
//! PR 1b fires the `entity_move` hook inside the client `move_gate::arm` this
//! door calls, with a revision recheck before the kind's physics runs.
use super::Game;
use super::entity::{Entity, EntityHandle, with_entity};
use horizon_core::geometry::Vec3;
use horizon_world::entity_move::MoverType;
use horizon_world::move_gate::MoveGate;
use std::borrow::Cow;

/// Which body a move is for: the local player window, a LAN guest, or a listed
/// entity. The piston and shulker-box pushes and the family ticks name their
/// mover with it.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(in crate::game) enum MoveWho {
    Player,
    Guest(u32),
    Entity(EntityHandle),
}

impl Game<'_> {
    /// Arm one physical `Entity.move` invocation (arismoko/horizon#2414) for `who` with `cause`
    /// and hand back its gate: the caller then runs the kind's physics with
    /// the movement the gate decided (`delta`), and skips it when the gate
    /// says so. A pusher arms before its own restrictions — the piston limit
    /// included — so a cancelled move spends none of them.
    pub(in crate::game) fn entity_move(
        &mut self,
        who: MoveWho,
        cause: MoverType,
        delta: Vec3,
    ) -> MoveGate {
        let entity = self.move_who_id(who);
        // The type name is built only if a listener asks for it (arismoko/horizon#2414: a
        // move nobody listens to costs one branch and no string).
        horizon_client::move_gate::armed_with(
            &self.loop_context,
            entity,
            || self.move_who_kind(who),
            cause,
            delta,
        )
    }

    /// The entity id a move is for; the player window's own id for it.
    fn move_who_id(&self, who: MoveWho) -> u32 {
        match who {
            MoveWho::Player => self.player.as_ref().map_or(0, |p| p.entity_id as u32),
            MoveWho::Guest(conn) => self.lan.as_ref().map_or(0, |host| {
                host.hub.guest(conn).map_or(0, |g| g.entity_id as u32)
            }),
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => entity.id()),
        }
    }

    /// The kind name (`minecraft:zombie`) a move is for, built on demand.
    fn move_who_kind(&self, who: MoveWho) -> Cow<'static, str> {
        match who {
            MoveWho::Player | MoveWho::Guest(_) => Cow::Borrowed("minecraft:player"),
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => entity.native_type()),
        }
    }
}
