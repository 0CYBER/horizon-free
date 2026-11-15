//! The server's reach rules for a player and an entity: which one
//! [`PlayerRef::within_entity_reach`](crate::PlayerRef::within_entity_reach) answers.

/// Which of the server's reach rules a player's reach read answers. Each is
/// measured from the player's eyes to the closest point of the entity's
/// bounding box, not inflated, in the caller's copy of the world; with the eyes inside
/// the box the distance is 0. The buffer each rule adds is part of the rule, including
/// when a client queries these server rules for its local player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the server has exactly these reach rules; the engine answers each
#[derive(horizon_mod_api_macros::Wire)]
pub enum EntityReach {
    /// Interacting with the entity, as the server checks a use packet
    /// (`Player.canInteractWithEntity` with a 3.0 buffer): the squared
    /// distance is strictly below (`entity_interaction_range` + 3.0)².
    Interact,
    /// Keeping a container the entity holds open, a chest boat's or a
    /// mount's inventory (`canInteractWithEntity` with a 4.0 buffer): the
    /// squared distance is strictly below (`entity_interaction_range` + 4.0)².
    Container,
    /// Attacking the entity, as the server checks an attack packet
    /// (`Player.isWithinAttackRange` with a 3.0 buffer): the distance is at
    /// least (minimum − margin − 3.0) and at most (maximum + margin + 3.0),
    /// both inclusive. The main hand's `minecraft:attack_range` component
    /// gives the minimum, the maximum and the margin (0.3 unless it says
    /// otherwise) and replaces the attribute; without it the range is 0 to
    /// `entity_interaction_range` with no margin.
    Attack,
}
