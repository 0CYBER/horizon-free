//! What guests do in a hosted world. The host's gameplay code is written for
//! the one local player, so a guest's action runs through it with the guest's
//! state swapped in for the duration of the call (position, inventory, game
//! mode, experience, mining and item use): digging, placing, using items and
//! blocks, attacking and the commands all behave exactly as they do for the
//! host, and every world change they cause reaches all players through the
//! block log, the level events and the entity tracker.
use super::{
    Dim, Game, GuestUse, Hit, InteractTarget, Mining, Screen,
    building_players::{BuildingPlayer, BuildingPlayers, HubBodies, shared},
    lan::LanHost,
    lan_crafting::MenuRemoval,
    level_sounds::LevelSound,
    respawn_anchor::RespawnConfig,
};
use horizon_client::{
    containers::BlockEntity,
    entities::Experience,
    first_person::{Hand, SwingAnimation},
    items::ItemStack,
    lan::{
        guest::SPECTATOR,
        hub::TickCtx,
        menu::{Rules, Slots},
    },
    motion::{MotionEvent, MotionWorld, PlayerMotion},
    pose::Dimensions,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use horizon_protocol::{
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::literal,
};
use horizon_world::entity_move::EntityMoveWorld;
use mod_api::{ActionRefused, GameMode, SoundSource};
use winit::keyboard::KeyCode;

include!(concat!(env!("OUT_DIR"), "/lan_action_modules.rs"));
include!("shared.rs");
