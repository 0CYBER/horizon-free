//! `ServerPlayer.resetLastActionTime`: which packets of a guest count as the player being active,
//! for the idle kick (`player-idle-timeout`).
use super::*;

/// Whether `packet` resets the guest's idle timer, as the handlers of
/// `ServerGamePacketListenerImpl` do: 17 of them call `resetLastActionTime` (chat, a command, a
/// sign, the hotbar slot, a punch, a recipe, a container click or button, a client command, and
/// the seven that first need the client to have loaded), and the movement handlers do through
/// `handlePlayerKnownMovement` once the move is past 1.0E-5 squared. Nothing else does.
///
/// `loaded` is [`Guest::client_loaded`]; `moved_far` says whether a position is past that
/// threshold from where the guest is; `vehicle_good` is the position of the vehicle the guest
/// rides as the game last accepted it (a vehicle's move that is no farther from it is the client
/// repeating itself while it stands still).
///
/// Written with no `_` arm on purpose: a packet added to the protocol does not compile until
/// someone decides what it is.
pub(super) fn counts_as_activity(
    packet: &S,
    loaded: bool,
    vehicle_good: Option<[f64; 3]>,
    moved_far: impl FnOnce([f64; 3]) -> bool,
) -> bool {
    match packet {
        // Handlers that call resetLastActionTime with nothing in the way.
        S::Chat(_)
        | S::ChatCommand(_)
        | S::ChatCommandSigned(_)
        | S::SignUpdate(_)
        | S::Punch(_)
        | S::PlaceRecipe(_)
        | S::ContainerClick(_)
        | S::ContainerButtonClick(_)
        | S::ClientCommand(_) => true,
        // handleSetCarriedItem: only for a slot of the hotbar.
        S::SetCarriedItem(p) => (0..=8).contains(&p.slot),
        // hasClientLoaded: these handlers return before the reset until the client has loaded
        // (and while the player is dead); handlePlayerInput is the same.
        S::Attack(_)
        | S::Interact(_)
        | S::UseItemOn(_)
        | S::UseItem(_)
        | S::PlayerAction(_)
        | S::PlayerCommand(_)
        | S::SpectatorAction(_)
        | S::PlayerInput(_) => loaded,
        // handlePlayerKnownMovement: the move must be past 1.0E-5 squared.
        S::MovePlayerPos(p) => moved_far([p.x, p.y, p.z]),
        S::MovePlayerPosRot(p) => moved_far([p.x, p.y, p.z]),
        // handleMoveVehicle reaches handlePlayerKnownMovement with the vehicle's move. The
        // client sends this every tick for a vehicle it drives, standing still or not, so a
        // repeat of the position the game last accepted is not activity. With no such position
        // (it is not tracked for every vehicle) the packet counts, as before.
        S::MoveVehicle(p) => vehicle_good.is_none_or(|from| {
            let d: f64 = (0..3).map(|k| (p.position[k] - from[k]).powi(2)).sum();
            d > f64::from(1.0E-5_f32)
        }),
        // Handlers that do not call it: a turn alone (it moves nothing), the accept of a teleport
        // (the move it reaches is 0 for an honest client), the answers to what the server asked,
        // the settings and the other requests of a client.
        S::AcceptTeleportation(_)
        | S::PlayerAbilities(_)
        | S::MovePlayerStatus(_)
        | S::MovePlayerRot(_)
        | S::PlayerLoaded(_)
        | S::ChunkBatchReceived(_)
        | S::ClientInformation(_)
        | S::RecipeBookChangeSettings(_)
        | S::RecipeBookSeenRecipe(_)
        | S::ClientTickEnd(_)
        | S::KeepAlive(_)
        | S::Pong(_)
        | S::PingRequest(_)
        | S::ChangeDifficulty(_)
        | S::LockDifficulty(_)
        | S::SetGameRule(_)
        | S::BlockEntityTagQuery(_)
        | S::EntityTagQuery(_)
        | S::TeleportToEntity(_)
        | S::ChangeGameMode(_)
        | S::ChatAck(_)
        | S::ChatSessionUpdate(_)
        | S::ConfigurationAcknowledged(_)
        | S::ContainerClose(_)
        | S::ContainerSlotStateChanged(_)
        | S::CookieResponse(_)
        | S::CustomPayload(_)
        | S::CustomClickAction(_)
        | S::EditBook(_)
        | S::PaddleBoat(_)
        | S::PickItemFromBlock(_)
        | S::PickItemFromEntity(_)
        | S::RenameItem(_)
        | S::SeenAdvancements(_)
        | S::JigsawGenerate(_)
        | S::ResourcePack(_)
        | S::SelectBundleItem(_)
        | S::SelectTrade(_)
        | S::SetBeacon(_)
        | S::SetCommandBlock(_)
        | S::SetCommandMinecart(_)
        | S::SetCreativeModeSlot(_)
        | S::SetJigsawBlock(_)
        | S::SetTestBlock(_)
        | S::TestInstanceBlockAction(_)
        | S::SetStructureBlock(_) => false,
        // A packet Horizon does not model: counted, as before.
        S::Unknown(..) => true,
    }
}
