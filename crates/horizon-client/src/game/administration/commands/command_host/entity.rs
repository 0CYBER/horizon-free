use super::super::Target;
use crate::game::Game;
use horizon_client::{
    command::Context,
    text::Text,
};
use horizon_gameplay::commands::{
    entity::{EntityHost, EntityRef},
    host::R,
};

/// The game's entity handle for a command: the kind of `Target` and its id.
pub(super) fn entity_ref(t: Target) -> EntityRef {
    let (kind, id) = match t {
        Target::Player => (0, 0),
        Target::Guest(id) => (1, id),
        Target::Mob(id) => (2, id),
        Target::Item(id) => (3, id),
        Target::Orb(id) => (4, id),
        Target::Projectile(id) => (5, id),
        Target::Stand(id) => (6, id),
        Target::Frame(id) => (7, id),
        Target::Painting(id) => (8, id),
        Target::Boat(id) => (9, id),
        Target::Cart(id) => (10, id),
        Target::Other(id) => (11, id),
        Target::Fangs(id) => (12, id),
        Target::Crystal(id) => (13, id),
        Target::Dragon(id) => (14, id),
        Target::DragonPart(id) => (15, id),
        Target::Tnt(id) => (16, id),
        Target::Cushion(id) => (17, id),
        Target::Falling(id) => (18, id),
        Target::Rocket(id) => (19, id),
        Target::Trident(id) => (20, id),
        Target::Spawner(id) => (21, id),
        Target::Cloud(id) => (22, id),
        Target::General(id) => (23, id),
        Target::Hook(id) => (24, id),
        Target::Knot(id) => (25, id),
    };
    EntityRef { kind, id }
}

/// The `Target` an `entity_ref` handle names. Only handles the game made are passed back.
pub(super) fn target_of(entity: EntityRef) -> Target {
    let id = entity.id;
    match entity.kind {
        0 => Target::Player,
        1 => Target::Guest(id),
        2 => Target::Mob(id),
        3 => Target::Item(id),
        4 => Target::Orb(id),
        5 => Target::Projectile(id),
        6 => Target::Stand(id),
        7 => Target::Frame(id),
        8 => Target::Painting(id),
        9 => Target::Boat(id),
        10 => Target::Cart(id),
        11 => Target::Other(id),
        12 => Target::Fangs(id),
        13 => Target::Crystal(id),
        14 => Target::Dragon(id),
        15 => Target::DragonPart(id),
        16 => Target::Tnt(id),
        17 => Target::Cushion(id),
        18 => Target::Falling(id),
        19 => Target::Rocket(id),
        20 => Target::Trident(id),
        21 => Target::Spawner(id),
        22 => Target::Cloud(id),
        23 => Target::General(id),
        24 => Target::Hook(id),
        _ => Target::Knot(id),
    }
}

impl EntityHost for Game<'_> {
    fn select_entities(&mut self, ctx: &Context, name: &str, players: bool) -> R<Vec<EntityRef>> {
        Ok(Game::entities(self, ctx, name, players)?
            .into_iter()
            .map(entity_ref)
            .collect())
    }
    fn entity_name(&self, entity: EntityRef) -> Text {
        self.target_name(target_of(entity))
    }
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        self.target_position(target_of(entity))
    }
    fn position_arg(&self, ctx: &Context, name: &str) -> Option<[f64; 3]> {
        Game::position_arg(self, ctx, name)
    }
    fn source_position(&self) -> [f64; 3] {
        Game::source_position(self)
    }
}
