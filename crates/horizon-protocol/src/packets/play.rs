//! Play state packets: world, entities, player state, inventory and chat.
use crate::{
    Error, Result,
    buf::{
        Bool, Byte, ChunkPos, Codec, Double, Float, Ident, Int, List, Long, LpVec3, Nbt, Opt,
        OptVarInt, Pos, Reader, SectionPos, Short, Str, StrN, UByte, Uuid, VarInt, VarLong, Vec3d,
        Writer,
    },
    chunk::{ChunkData, LightData},
    entity_data::{EntityData, Value},
    item::{Hashed, HashedSlot, Slot, Stack},
    packet, packet_enum,
    packets::{
        Empty,
        configuration::{
            ClientInformation, CustomPayload, Disconnect, KeepAlive, Ping, ResourcePackPush,
            ResourcePackResponse, UpdateTags,
        },
        login::Property,
    },
};
use horizon_world::nbt::Tag;

// ---- shared structures -------------------------------------------------

packet!(
    /// GlobalPos: a dimension and a block position.
    GlobalPos { dimension: String => Ident, pos: [i32; 3] => Pos }
);
packet!(
    /// CommonPlayerSpawnInfo.
    SpawnInfo {
        dimension_type: i32 => VarInt,
        dimension: String => Ident,
        seed: i64 => Long,
        game_type: i32 => VarInt,
        previous_game_type: Option<i32> => OptVarInt,
        debug: bool => Bool,
        flat: bool => Bool,
        last_death: Option<GlobalPos> => Opt<GlobalPos>,
        portal_cooldown: i32 => VarInt,
        sea_level: i32 => VarInt,
    }
);

/// Relative flags (Relative.pack): X, Y, Z, Y_ROT, X_ROT, DELTA_X/Y/Z, ROTATE_DELTA.
pub mod relative {
    pub const X: i32 = 1;
    pub const Y: i32 = 2;
    pub const Z: i32 = 4;
    pub const Y_ROT: i32 = 8;
    pub const X_ROT: i32 = 16;
    pub const DELTA_X: i32 = 32;
    pub const DELTA_Y: i32 = 64;
    pub const DELTA_Z: i32 = 128;
    pub const ROTATE_DELTA: i32 = 256;
}

// ---- clientbound -------------------------------------------------------

packet!(AddEntity {
    id: i32 => VarInt,
    uuid: u128 => Uuid,
    kind: i32 => VarInt,
    position: [f64; 3] => Vec3d,
    velocity: [f64; 3] => LpVec3,
    pitch: i8 => Byte,
    yaw: i8 => Byte,
    head_yaw: i8 => Byte,
    data: i32 => VarInt,
});
packet!(Animate { id: i32 => VarInt, action: u8 => UByte });
packet!(BlockChangedAck { sequence: i32 => VarInt });
packet!(BlockDestruction { id: i32 => VarInt, pos: [i32; 3] => Pos, progress: u8 => UByte });
packet!(BlockEntityData { pos: [i32; 3] => Pos, kind: i32 => VarInt, tag: Tag => Nbt });
packet!(BlockUpdate { pos: [i32; 3] => Pos, state: i32 => VarInt });
packet!(ChangeDifficulty { difficulty: i32 => VarInt, locked: bool => Bool });
packet!(ChunkBatchFinished { size: i32 => VarInt });
packet!(ContainerClose { container: i32 => VarInt });
packet!(ContainerSetContent {
    container: i32 => VarInt,
    state: i32 => VarInt,
    items: Vec<Option<Stack>> => List<Slot>,
    carried: Option<Stack> => Slot,
});
packet!(ContainerSetData { container: i32 => VarInt, id: i16 => Short, value: i16 => Short });
packet!(ContainerSetSlot { container: i32 => VarInt, state: i32 => VarInt, slot: i16 => Short, item: Option<Stack> => Slot });
/// A holder reference: registry id + 1, or 0 for an inline value (unsupported).
pub struct HolderId;
impl Codec<i32> for HolderId {
    fn read(r: &mut Reader) -> Result<i32> {
        match r.varint()? {
            0 => Err(Error::new("inline registry values are not supported")),
            id => Ok(id - 1),
        }
    }
    fn write(v: &i32, w: &mut Writer) {
        w.varint(v + 1)
    }
}
packet!(
    /// ChatType.Bound.
    ChatTypeBound { chat_type: i32 => HolderId, name: Tag => Nbt, target: Option<Tag> => Opt<Nbt> }
);
packet!(DisguisedChat { message: Tag => Nbt, chat_type: ChatTypeBound => ChatTypeBound });
packet!(EntityEvent { id: i32 => Int, event: i8 => Byte });
/// PositionPath: Linear(end position) or Stepped(list of (position, tick offset)).
#[derive(Clone, Debug, PartialEq)]
pub enum PositionPath {
    Linear([f64; 3]),
    Stepped(Vec<([f64; 3], i32)>),
}
impl PositionPath {
    pub fn end(&self) -> [f64; 3] {
        match self {
            Self::Linear(p) => *p,
            Self::Stepped(steps) => steps.last().map_or([0.0; 3], |s| s.0),
        }
    }
}
impl Codec<PositionPath> for PositionPath {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(match r.varint()? {
            1 => {
                let n = r.count(r.remaining() / 25)?;
                Self::Stepped(
                    (0..n)
                        .map(|_| Ok((Vec3d::read(r)?, r.varint()?)))
                        .collect::<Result<_>>()?,
                )
            }
            _ => Self::Linear(Vec3d::read(r)?),
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        match v {
            Self::Linear(p) => {
                w.varint(0);
                Vec3d::write(p, w);
            }
            Self::Stepped(steps) => {
                w.varint(1);
                w.varint(steps.len() as i32);
                for (p, t) in steps {
                    Vec3d::write(p, w);
                    w.varint(*t);
                }
            }
        }
    }
}
packet!(EntityPositionSync { id: i32 => VarInt, path: PositionPath => PositionPath, yaw: f32 => Float, pitch: f32 => Float, on_ground: bool => Bool });
packet!(ForgetLevelChunk { pos: [i32; 2] => ChunkPos });
packet!(GameEvent { event: u8 => UByte, param: f32 => Float });
packet!(HurtAnimation { id: i32 => VarInt, yaw: f32 => Float });
packet!(LevelChunkWithLight { x: i32 => Int, z: i32 => Int, chunk: ChunkData => ChunkData, light: LightData => LightData });
packet!(LevelEvent { kind: i32 => Int, pos: [i32; 3] => Pos, data: i32 => Int, global: bool => Bool });
packet!(LightUpdate { x: i32 => VarInt, z: i32 => VarInt, light: LightData => LightData });
packet!(Login {
    player_id: i32 => Int,
    hardcore: bool => Bool,
    levels: Vec<String> => List<Ident>,
    max_players: i32 => VarInt,
    chunk_radius: i32 => VarInt,
    simulation_distance: i32 => VarInt,
    reduced_debug_info: bool => Bool,
    show_death_screen: bool => Bool,
    limited_crafting: bool => Bool,
    spawn: SpawnInfo => SpawnInfo,
    online_mode: bool => Bool,
    enforces_secure_chat: bool => Bool,
});
/// VecDelta: per-axis deltas in 1/4096 blocks, linear or in timed steps.
#[derive(Clone, Debug, PartialEq)]
pub enum VecDelta {
    Linear([i16; 3]),
    Stepped(Vec<([i16; 3], i32)>),
}
impl VecDelta {
    /// VecDeltaCodec.decode against a base position: zero deltas keep the
    /// base coordinate exactly, others add to its 1/4096 quantization.
    pub fn apply(base: [f64; 3], delta: [i16; 3]) -> [f64; 3] {
        let mut out = base;
        for i in 0..3 {
            if delta[i] != 0 {
                out[i] = ((base[i] * 4096.0).round() as i64 + i64::from(delta[i])) as f64 / 4096.0;
            }
        }
        out
    }
    /// The final position after all steps.
    pub fn end(&self, base: [f64; 3]) -> [f64; 3] {
        match self {
            Self::Linear(d) => Self::apply(base, *d),
            Self::Stepped(steps) => steps.iter().fold(base, |p, (d, _)| Self::apply(p, *d)),
        }
    }
    fn read(r: &mut Reader, steps: usize) -> Result<Self> {
        if steps == 0 {
            return Ok(Self::Linear([r.i16()?, r.i16()?, r.i16()?]));
        }
        if steps > r.remaining() / 7 {
            return Err(Error::new("VecDelta with too many steps"));
        }
        Ok(Self::Stepped(
            (0..steps)
                .map(|_| {
                    let ticks = r.varint()?;
                    Ok(([r.i16()?, r.i16()?, r.i16()?], ticks))
                })
                .collect::<Result<_>>()?,
        ))
    }
    fn steps(&self) -> usize {
        match self {
            Self::Linear(_) => 0,
            Self::Stepped(s) => s.len(),
        }
    }
    fn write(&self, w: &mut Writer) {
        match self {
            Self::Linear(d) => d.iter().for_each(|v| w.i16(*v)),
            Self::Stepped(steps) => {
                for (d, ticks) in steps {
                    w.varint(*ticks);
                    d.iter().for_each(|v| w.i16(*v));
                }
            }
        }
    }
}
/// ClientboundMoveEntityPacket.Pos / PosRot / Rot.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveEntity {
    pub id: i32,
    pub delta: Option<VecDelta>,
    pub rotation: Option<(i8, i8)>,
    pub on_ground: bool,
}
macro_rules! move_entity {
    ($name:ident, pos: $pos:expr, rot: $rot:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub MoveEntity);
        impl Codec<$name> for $name {
            fn read(r: &mut Reader) -> Result<$name> {
                Self::read_inner(r).map($name)
            }
            fn write(v: &$name, w: &mut Writer) {
                Self::write_inner(&v.0, w)
            }
        }
        impl $name {
            fn read_inner(r: &mut Reader) -> Result<MoveEntity> {
                let id = r.varint()?;
                if !$pos {
                    let on_ground = r.bool()?;
                    let rotation = Some((r.i8()?, r.i8()?));
                    return Ok(MoveEntity {
                        id,
                        delta: None,
                        rotation,
                        on_ground,
                    });
                }
                let properties = r.varint()?;
                let delta = Some(VecDelta::read(r, (properties as u32 >> 1) as usize)?);
                let rotation = if $rot { Some((r.i8()?, r.i8()?)) } else { None };
                Ok(MoveEntity {
                    id,
                    delta,
                    rotation,
                    on_ground: properties & 1 != 0,
                })
            }
            fn write_inner(v: &MoveEntity, w: &mut Writer) {
                w.varint(v.id);
                if !$pos {
                    w.bool(v.on_ground);
                } else {
                    let delta = v.delta.clone().unwrap_or(VecDelta::Linear([0; 3]));
                    w.varint(i32::from(v.on_ground) | (delta.steps() as i32) << 1);
                    delta.write(w);
                }
                if let Some((y, x)) = v.rotation.filter(|_| $rot) {
                    w.i8(y);
                    w.i8(x);
                }
            }
        }
    };
}
move_entity!(MoveEntityPos, pos: true, rot: false);
move_entity!(MoveEntityPosRot, pos: true, rot: true);
move_entity!(MoveEntityRot, pos: false, rot: true);
packet!(OpenScreen { container: i32 => VarInt, menu: i32 => VarInt, title: Tag => Nbt });
packet!(PlayerAbilities { flags: i8 => Byte, flying_speed: f32 => Float, walking_speed: f32 => Float });
/// MessageSignature.Packed: an index into the last-seen cache or a full signature.
pub struct PackedSignature;
impl Codec<(i32, Option<Vec<u8>>)> for PackedSignature {
    fn read(r: &mut Reader) -> Result<(i32, Option<Vec<u8>>)> {
        let id = r.varint()? - 1;
        Ok(if id == -1 {
            (-1, Some(r.bytes(256)?.to_vec()))
        } else {
            (id, None)
        })
    }
    fn write(v: &(i32, Option<Vec<u8>>), w: &mut Writer) {
        w.varint(v.0 + 1);
        if let Some(sig) = &v.1 {
            w.bytes(sig);
        }
    }
}
/// MessageSignature: 256 raw bytes.
pub struct Signature;
impl Codec<Vec<u8>> for Signature {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        Ok(r.bytes(256)?.to_vec())
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        w.bytes(v)
    }
}
/// FilterMask: 0 pass-through, 1 fully filtered, 2 partially (with a BitSet).
pub struct FilterMask;
impl Codec<(i32, Vec<i64>)> for FilterMask {
    fn read(r: &mut Reader) -> Result<(i32, Vec<i64>)> {
        let kind = r.varint()?;
        Ok((
            kind,
            if kind == 2 {
                r.long_array()?
            } else {
                Vec::new()
            },
        ))
    }
    fn write(v: &(i32, Vec<i64>), w: &mut Writer) {
        w.varint(v.0);
        if v.0 == 2 {
            w.long_array(&v.1);
        }
    }
}
packet!(PlayerChat {
    global_index: i32 => VarInt,
    sender: u128 => Uuid,
    index: i32 => VarInt,
    signature: Option<Vec<u8>> => Opt<Signature>,
    content: String => StrN<256>,
    timestamp: i64 => Long,
    salt: i64 => Long,
    last_seen: Vec<(i32, Option<Vec<u8>>)> => List<PackedSignature>,
    unsigned_content: Option<Tag> => Opt<Nbt>,
    filter: (i32, Vec<i64>) => FilterMask,
    chat_type: ChatTypeBound => ChatTypeBound,
});
packet!(PlayerCombatKill { id: i32 => VarInt, message: Tag => Nbt });
packet!(PlayerInfoRemove { uuids: Vec<u128> => List<Uuid> });
/// One ClientboundPlayerInfoUpdatePacket entry; fields are set for the
/// actions in the packet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerInfo {
    pub uuid: u128,
    pub name: Option<String>,
    pub properties: Vec<Property>,
    /// RemoteChatSession.Data: session id, key expiry, key, key signature.
    pub chat_session: Option<(u128, i64, Vec<u8>, Vec<u8>)>,
    pub game_mode: Option<i32>,
    pub listed: Option<bool>,
    pub latency: Option<i32>,
    pub display_name: Option<Option<Tag>>,
    pub list_order: Option<i32>,
    pub show_hat: Option<bool>,
}
/// Actions bits: ADD_PLAYER, INITIALIZE_CHAT, UPDATE_GAME_MODE, UPDATE_LISTED,
/// UPDATE_LATENCY, UPDATE_DISPLAY_NAME, UPDATE_LIST_ORDER, UPDATE_HAT.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfoUpdate {
    pub actions: u8,
    pub entries: Vec<PlayerInfo>,
}
impl Codec<PlayerInfoUpdate> for PlayerInfoUpdate {
    fn read(r: &mut Reader) -> Result<Self> {
        let actions = r.u8()?;
        let n = r.count(r.remaining())?;
        let mut entries = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            let mut e = PlayerInfo {
                uuid: r.uuid()?,
                ..PlayerInfo::default()
            };
            if actions & 1 != 0 {
                e.name = Some(r.string(16)?);
                let count = r.count(16)?;
                e.properties = (0..count)
                    .map(|_| Property::read(r))
                    .collect::<Result<_>>()?;
            }
            if actions & 2 != 0 && r.bool()? {
                e.chat_session =
                    Some((r.uuid()?, r.i64()?, r.byte_array(512)?, r.byte_array(4096)?));
            }
            if actions & 4 != 0 {
                e.game_mode = Some(r.varint()?);
            }
            if actions & 8 != 0 {
                e.listed = Some(r.bool()?);
            }
            if actions & 16 != 0 {
                e.latency = Some(r.varint()?);
            }
            if actions & 32 != 0 {
                e.display_name = Some(if r.bool()? { Some(r.nbt()?) } else { None });
            }
            if actions & 64 != 0 {
                e.list_order = Some(r.varint()?);
            }
            if actions & 128 != 0 {
                e.show_hat = Some(r.bool()?);
            }
            entries.push(e);
        }
        Ok(Self { actions, entries })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.u8(v.actions);
        w.varint(v.entries.len() as i32);
        for e in &v.entries {
            w.uuid(e.uuid);
            if v.actions & 1 != 0 {
                w.string(e.name.as_deref().unwrap_or(""));
                w.varint(e.properties.len() as i32);
                e.properties.iter().for_each(|p| Property::write(p, w));
            }
            if v.actions & 2 != 0 {
                w.bool(e.chat_session.is_some());
                if let Some((id, expires, key, sig)) = &e.chat_session {
                    w.uuid(*id);
                    w.i64(*expires);
                    w.byte_array(key);
                    w.byte_array(sig);
                }
            }
            if v.actions & 4 != 0 {
                w.varint(e.game_mode.unwrap_or(0));
            }
            if v.actions & 8 != 0 {
                w.bool(e.listed.unwrap_or(false));
            }
            if v.actions & 16 != 0 {
                w.varint(e.latency.unwrap_or(0));
            }
            if v.actions & 32 != 0 {
                let name = e.display_name.clone().flatten();
                w.bool(name.is_some());
                if let Some(t) = &name {
                    w.nbt(t);
                }
            }
            if v.actions & 64 != 0 {
                w.varint(e.list_order.unwrap_or(0));
            }
            if v.actions & 128 != 0 {
                w.bool(e.show_hat.unwrap_or(false));
            }
        }
    }
}
packet!(PlayerPosition {
    teleport: i32 => VarInt,
    position: [f64; 3] => Vec3d,
    delta: [f64; 3] => Vec3d,
    yaw: f32 => Float,
    pitch: f32 => Float,
    relatives: i32 => Int,
});
packet!(PlayerRotation { yaw: f32 => Float, relative_yaw: bool => Bool, pitch: f32 => Float, relative_pitch: bool => Bool });
packet!(RemoveEntities { ids: Vec<i32> => List<VarInt> });
packet!(Respawn { spawn: SpawnInfo => SpawnInfo, keep: i8 => Byte });
packet!(RotateHead { id: i32 => VarInt, head_yaw: i8 => Byte });
packet!(
    /// Each change is `state << 12 | x << 8 | z << 4 | y` within the section.
    SectionBlocksUpdate { section: [i32; 3] => SectionPos, changes: Vec<i64> => List<VarLong> }
);
packet!(Text { text: Tag => Nbt });
packet!(SetChunkCacheCenter { x: i32 => VarInt, z: i32 => VarInt });
packet!(Distance { distance: i32 => VarInt });
packet!(SetCursorItem { item: Option<Stack> => Slot });
packet!(SetDefaultSpawnPosition { pos: GlobalPos => GlobalPos, yaw: f32 => Float, pitch: f32 => Float });
packet!(SetEntityData { id: i32 => VarInt, values: Vec<(u8, i32, Value)> => EntityData });
packet!(SetEntityMotion { id: i32 => VarInt, velocity: [f64; 3] => LpVec3 });
/// ClientboundSetEquipmentPacket: (EquipmentSlot ordinal, stack) pairs.
pub struct Equipment;
impl Codec<Vec<(u8, Option<Stack>)>> for Equipment {
    fn read(r: &mut Reader) -> Result<Vec<(u8, Option<Stack>)>> {
        let mut out = Vec::new();
        loop {
            let slot = r.u8()?;
            out.push((slot & 127, Slot::read(r)?));
            if slot & 128 == 0 {
                return Ok(out);
            }
        }
    }
    fn write(v: &Vec<(u8, Option<Stack>)>, w: &mut Writer) {
        for (i, (slot, stack)) in v.iter().enumerate() {
            w.u8(if i + 1 < v.len() { slot | 128 } else { *slot });
            Slot::write(stack, w);
        }
    }
}
packet!(SetEquipment { id: i32 => VarInt, slots: Vec<(u8, Option<Stack>)> => Equipment });
packet!(SetExperience { progress: f32 => Float, level: i32 => VarInt, total: i32 => VarInt });
packet!(SetHealth { health: f32 => Float, food: i32 => VarInt, saturation: f32 => Float });
packet!(SetHeldSlot { slot: i32 => VarInt });
packet!(SetPlayerInventory { slot: i32 => VarInt, item: Option<Stack> => Slot });
packet!(
    /// ClockNetworkState for one world clock (registry id).
    ClockState { clock: i32 => VarInt, total_ticks: i64 => VarLong, partial_tick: f32 => Float, rate: f32 => Float }
);
packet!(SetTime { game_time: i64 => Long, clocks: Vec<ClockState> => List<ClockState> });
packet!(TitlesAnimation { fade_in: i32 => Int, stay: i32 => Int, fade_out: i32 => Int });
/// SoundEvent holder: a registry id, or an inline location with optional range.
pub struct SoundHolder;
impl Codec<(i32, Option<(String, Option<f32>)>)> for SoundHolder {
    fn read(r: &mut Reader) -> Result<(i32, Option<(String, Option<f32>)>)> {
        let id = r.varint()?;
        if id != 0 {
            return Ok((id - 1, None));
        }
        let location = r.string(32767)?;
        let range = if r.bool()? { Some(r.f32()?) } else { None };
        Ok((-1, Some((location, range))))
    }
    fn write(v: &(i32, Option<(String, Option<f32>)>), w: &mut Writer) {
        match &v.1 {
            None => w.varint(v.0 + 1),
            Some((location, range)) => {
                w.varint(0);
                w.string(location);
                <Opt<Float> as Codec<Option<f32>>>::write(range, w);
            }
        }
    }
}
packet!(Sound {
    sound: (i32, Option<(String, Option<f32>)>) => SoundHolder,
    source: i32 => VarInt,
    x: i32 => Int,
    y: i32 => Int,
    z: i32 => Int,
    volume: f32 => Float,
    pitch: f32 => Float,
    seed: i64 => Long,
});
packet!(SystemChat { content: Tag => Nbt, overlay: bool => Bool });
packet!(TakeItemEntity { item: i32 => VarInt, player: i32 => VarInt, amount: i32 => VarInt });
packet!(TeleportEntity {
    id: i32 => VarInt,
    position: [f64; 3] => Vec3d,
    delta: [f64; 3] => Vec3d,
    yaw: f32 => Float,
    pitch: f32 => Float,
    relatives: i32 => Int,
    on_ground: bool => Bool,
});

packet_enum!(Clientbound, crate::ids::play::clientbound {
    BundleDelimiter(Empty) = BUNDLE_DELIMITER,
    AddEntity(AddEntity) = ADD_ENTITY,
    Animate(Animate) = ANIMATE,
    BlockChangedAck(BlockChangedAck) = BLOCK_CHANGED_ACK,
    BlockDestruction(BlockDestruction) = BLOCK_DESTRUCTION,
    BlockEntityData(BlockEntityData) = BLOCK_ENTITY_DATA,
    BlockUpdate(BlockUpdate) = BLOCK_UPDATE,
    ChangeDifficulty(ChangeDifficulty) = CHANGE_DIFFICULTY,
    ChunkBatchFinished(ChunkBatchFinished) = CHUNK_BATCH_FINISHED,
    ChunkBatchStart(Empty) = CHUNK_BATCH_START,
    ClearTitles(Bool1) = CLEAR_TITLES,
    ContainerClose(ContainerClose) = CONTAINER_CLOSE,
    ContainerSetContent(ContainerSetContent) = CONTAINER_SET_CONTENT,
    ContainerSetData(ContainerSetData) = CONTAINER_SET_DATA,
    ContainerSetSlot(ContainerSetSlot) = CONTAINER_SET_SLOT,
    CustomPayload(CustomPayload) = CUSTOM_PAYLOAD,
    Disconnect(Disconnect) = DISCONNECT,
    DisguisedChat(DisguisedChat) = DISGUISED_CHAT,
    EntityEvent(EntityEvent) = ENTITY_EVENT,
    EntityPositionSync(EntityPositionSync) = ENTITY_POSITION_SYNC,
    ForgetLevelChunk(ForgetLevelChunk) = FORGET_LEVEL_CHUNK,
    GameEvent(GameEvent) = GAME_EVENT,
    HurtAnimation(HurtAnimation) = HURT_ANIMATION,
    KeepAlive(KeepAlive) = KEEP_ALIVE,
    LevelChunkWithLight(LevelChunkWithLight) = LEVEL_CHUNK_WITH_LIGHT,
    LevelEvent(LevelEvent) = LEVEL_EVENT,
    LightUpdate(LightUpdate) = LIGHT_UPDATE,
    Login(Login) = LOGIN,
    MoveEntityPos(MoveEntityPos) = MOVE_ENTITY_POS,
    MoveEntityPosRot(MoveEntityPosRot) = MOVE_ENTITY_POS_ROT,
    MoveEntityRot(MoveEntityRot) = MOVE_ENTITY_ROT,
    OpenScreen(OpenScreen) = OPEN_SCREEN,
    Ping(Ping) = PING,
    PlayerAbilities(PlayerAbilities) = PLAYER_ABILITIES,
    PlayerChat(PlayerChat) = PLAYER_CHAT,
    PlayerCombatKill(PlayerCombatKill) = PLAYER_COMBAT_KILL,
    PlayerInfoRemove(PlayerInfoRemove) = PLAYER_INFO_REMOVE,
    PlayerInfoUpdate(PlayerInfoUpdate) = PLAYER_INFO_UPDATE,
    PlayerPosition(PlayerPosition) = PLAYER_POSITION,
    PlayerRotation(PlayerRotation) = PLAYER_ROTATION,
    RemoveEntities(RemoveEntities) = REMOVE_ENTITIES,
    ResourcePackPush(ResourcePackPush) = RESOURCE_PACK_PUSH,
    Respawn(Respawn) = RESPAWN,
    RotateHead(RotateHead) = ROTATE_HEAD,
    SectionBlocksUpdate(SectionBlocksUpdate) = SECTION_BLOCKS_UPDATE,
    SetActionBarText(Text) = SET_ACTION_BAR_TEXT,
    SetChunkCacheCenter(SetChunkCacheCenter) = SET_CHUNK_CACHE_CENTER,
    SetChunkCacheRadius(Distance) = SET_CHUNK_CACHE_RADIUS,
    SetCursorItem(SetCursorItem) = SET_CURSOR_ITEM,
    SetDefaultSpawnPosition(SetDefaultSpawnPosition) = SET_DEFAULT_SPAWN_POSITION,
    SetEntityData(SetEntityData) = SET_ENTITY_DATA,
    SetEntityMotion(SetEntityMotion) = SET_ENTITY_MOTION,
    SetEquipment(SetEquipment) = SET_EQUIPMENT,
    SetExperience(SetExperience) = SET_EXPERIENCE,
    SetHealth(SetHealth) = SET_HEALTH,
    SetHeldSlot(SetHeldSlot) = SET_HELD_SLOT,
    SetPlayerInventory(SetPlayerInventory) = SET_PLAYER_INVENTORY,
    SetSimulationDistance(Distance) = SET_SIMULATION_DISTANCE,
    SetSubtitleText(Text) = SET_SUBTITLE_TEXT,
    SetTime(SetTime) = SET_TIME,
    SetTitleText(Text) = SET_TITLE_TEXT,
    SetTitlesAnimation(TitlesAnimation) = SET_TITLES_ANIMATION,
    Sound(Sound) = SOUND,
    StartConfiguration(Empty) = START_CONFIGURATION,
    SystemChat(SystemChat) = SYSTEM_CHAT,
    TakeItemEntity(TakeItemEntity) = TAKE_ITEM_ENTITY,
    TeleportEntity(TeleportEntity) = TELEPORT_ENTITY,
    UpdateTags(UpdateTags) = UPDATE_TAGS,
});
packet!(
    /// ClientboundClearTitlesPacket: whether to also reset the timings.
    Bool1 { value: bool => Bool }
);

// ---- serverbound -------------------------------------------------------

packet!(AcceptTeleportation { id: i32 => VarInt, x: f64 => Double, y: f64 => Double, z: f64 => Double, yaw: f32 => Float, pitch: f32 => Float });
packet!(Attack { id: i32 => VarInt });
packet!(LastSeenUpdate { offset: i32 => VarInt, acknowledged: [u8; 3] => Fixed3, checksum: i8 => Byte });
/// LastSeenMessages.Update's fixed 20-bit BitSet (3 bytes).
pub struct Fixed3;
impl Codec<[u8; 3]> for Fixed3 {
    fn read(r: &mut Reader) -> Result<[u8; 3]> {
        Ok(r.bytes(3)?.try_into().expect("3 bytes"))
    }
    fn write(v: &[u8; 3], w: &mut Writer) {
        w.bytes(v)
    }
}
packet!(Chat {
    message: String => StrN<256>,
    timestamp: i64 => Long,
    salt: i64 => Long,
    signature: Option<Vec<u8>> => Opt<Signature>,
    last_seen: LastSeenUpdate => LastSeenUpdate,
});
packet!(ChatCommand { command: String => Str });
packet!(ChunkBatchReceived { chunks_per_tick: f32 => Float });
packet!(
    /// ServerboundClientCommandPacket.Action: 0 respawn, 1 request stats, 2 request game rules.
    ClientCommand { action: i32 => VarInt }
);
/// Changed slots: (slot, hashed stack) pairs.
pub struct ChangedSlots;
impl Codec<Vec<(i16, Option<Hashed>)>> for ChangedSlots {
    fn read(r: &mut Reader) -> Result<Vec<(i16, Option<Hashed>)>> {
        let n = r.count(128)?;
        (0..n)
            .map(|_| Ok((r.i16()?, HashedSlot::read(r)?)))
            .collect()
    }
    fn write(v: &Vec<(i16, Option<Hashed>)>, w: &mut Writer) {
        w.varint(v.len() as i32);
        for (slot, h) in v {
            w.i16(*slot);
            HashedSlot::write(h, w);
        }
    }
}
packet!(
    /// ContainerInput: 0 pickup, 1 quick move, 2 swap, 3 clone, 4 throw, 5 quick craft, 6 pickup all.
    ContainerClick {
        container: i32 => VarInt,
        state: i32 => VarInt,
        slot: i16 => Short,
        button: i8 => Byte,
        input: i32 => VarInt,
        changed: Vec<(i16, Option<Hashed>)> => ChangedSlots,
        carried: Option<Hashed> => HashedSlot,
    }
);
packet!(Interact { id: i32 => VarInt, hand: i32 => VarInt, location: [f64; 3] => LpVec3, secondary: bool => Bool });
packet!(
    /// ServerboundMovePlayerPacket flags: 1 on ground, 2 horizontal collision.
    MovePlayerPos { x: f64 => Double, y: f64 => Double, z: f64 => Double, flags: u8 => UByte }
);
packet!(MovePlayerPosRot { x: f64 => Double, y: f64 => Double, z: f64 => Double, yaw: f32 => Float, pitch: f32 => Float, flags: u8 => UByte });
packet!(MovePlayerRot { yaw: f32 => Float, pitch: f32 => Float, flags: u8 => UByte });
packet!(MovePlayerStatus { flags: u8 => UByte });
packet!(PickItemFromBlock { pos: [i32; 3] => Pos, include_data: bool => Bool });
packet!(
    /// ServerboundPlayerActionPacket.Action: 0 start destroy, 1 change destroy
    /// direction, 2 abort, 3 stop destroy, 4 drop all, 5 drop item, 6 release
    /// use item, 7 swap offhand, 8 stab. Direction is the 3D data value.
    PlayerAction { action: i32 => VarInt, pos: [i32; 3] => Pos, direction: u8 => UByte, sequence: i32 => VarInt }
);
packet!(
    /// ServerboundPlayerCommandPacket.Action: 0 stop sleeping, 1 start sprinting,
    /// 2 stop sprinting, 3/4 riding jump, 5 open inventory, 6 start fall flying.
    PlayerCommand { id: i32 => VarInt, action: i32 => VarInt, data: i32 => VarInt }
);
packet!(
    /// Input flags: forward 1, backward 2, left 4, right 8, jump 16, shift 32, sprint 64.
    PlayerInput { flags: u8 => UByte }
);
packet!(SetCarriedItem { slot: i16 => Short });
packet!(SetCreativeModeSlot { slot: i16 => Short, item: Option<Stack> => Slot });
packet!(UseItem { hand: i32 => VarInt, sequence: i32 => VarInt, yaw: f32 => Float, pitch: f32 => Float });
packet!(UseItemOn {
    hand: i32 => VarInt,
    pos: [i32; 3] => Pos,
    face: i32 => VarInt,
    cursor: [f32; 3] => Cursor,
    inside: bool => Bool,
    world_border: bool => Bool,
    sequence: i32 => VarInt,
});
/// The click location relative to the block (three floats).
pub struct Cursor;
impl Codec<[f32; 3]> for Cursor {
    fn read(r: &mut Reader) -> Result<[f32; 3]> {
        Ok([r.f32()?, r.f32()?, r.f32()?])
    }
    fn write(v: &[f32; 3], w: &mut Writer) {
        v.iter().for_each(|f| w.f32(*f));
    }
}

packet_enum!(Serverbound, crate::ids::play::serverbound {
    AcceptTeleportation(AcceptTeleportation) = ACCEPT_TELEPORTATION,
    Attack(Attack) = ATTACK,
    Chat(Chat) = CHAT,
    ChatCommand(ChatCommand) = CHAT_COMMAND,
    ChunkBatchReceived(ChunkBatchReceived) = CHUNK_BATCH_RECEIVED,
    ClientCommand(ClientCommand) = CLIENT_COMMAND,
    ClientInformation(ClientInformation) = CLIENT_INFORMATION,
    ClientTickEnd(Empty) = CLIENT_TICK_END,
    ConfigurationAcknowledged(Empty) = CONFIGURATION_ACKNOWLEDGED,
    ContainerClick(ContainerClick) = CONTAINER_CLICK,
    ContainerClose(ContainerClose) = CONTAINER_CLOSE,
    CustomPayload(CustomPayload) = CUSTOM_PAYLOAD,
    Interact(Interact) = INTERACT,
    KeepAlive(KeepAlive) = KEEP_ALIVE,
    MovePlayerPos(MovePlayerPos) = MOVE_PLAYER_POS,
    MovePlayerPosRot(MovePlayerPosRot) = MOVE_PLAYER_POS_ROT,
    MovePlayerRot(MovePlayerRot) = MOVE_PLAYER_ROT,
    MovePlayerStatus(MovePlayerStatus) = MOVE_PLAYER_STATUS_ONLY,
    PickItemFromBlock(PickItemFromBlock) = PICK_ITEM_FROM_BLOCK,
    PlayerAction(PlayerAction) = PLAYER_ACTION,
    PlayerCommand(PlayerCommand) = PLAYER_COMMAND,
    PlayerInput(PlayerInput) = PLAYER_INPUT,
    PlayerLoaded(Empty) = PLAYER_LOADED,
    Pong(Ping) = PONG,
    Punch(Empty) = PUNCH,
    ResourcePack(ResourcePackResponse) = RESOURCE_PACK,
    SetCarriedItem(SetCarriedItem) = SET_CARRIED_ITEM,
    SetCreativeModeSlot(SetCreativeModeSlot) = SET_CREATIVE_MODE_SLOT,
    UseItem(UseItem) = USE_ITEM,
    UseItemOn(UseItemOn) = USE_ITEM_ON,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk::BlockEntityInfo, packets::round_trip, text::literal};

    fn spawn() -> SpawnInfo {
        SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: -123,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: Some(GlobalPos {
                dimension: "minecraft:the_nether".into(),
                pos: [1, 2, 3],
            }),
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    #[test]
    fn clientbound_round_trips() {
        let packets = vec![
            Clientbound::BundleDelimiter(Empty),
            Clientbound::AddEntity(AddEntity {
                id: 5,
                uuid: 9,
                kind: 100,
                position: [1.5, 64.0, -3.25],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 64,
                head_yaw: 64,
                data: 0,
            }),
            Clientbound::BlockUpdate(BlockUpdate {
                pos: [-5, -60, 7],
                state: 1,
            }),
            Clientbound::ContainerSetContent(ContainerSetContent {
                container: 0,
                state: 3,
                items: vec![None, Some(Stack::new(1, 64))],
                carried: None,
            }),
            Clientbound::ContainerSetSlot(ContainerSetSlot {
                container: 0,
                state: 4,
                slot: 36,
                item: Some(Stack::new(2, 1)),
            }),
            Clientbound::Disconnect(Disconnect {
                reason: literal("kicked"),
            }),
            Clientbound::EntityPositionSync(EntityPositionSync {
                id: 1,
                path: PositionPath::Stepped(vec![([1.0, 2.0, 3.0], 0), ([1.5, 2.0, 3.0], 2)]),
                yaw: 1.0,
                pitch: 2.0,
                on_ground: true,
            }),
            Clientbound::ForgetLevelChunk(ForgetLevelChunk { pos: [-1, 2] }),
            Clientbound::GameEvent(GameEvent {
                event: 13,
                param: 0.0,
            }),
            Clientbound::LevelChunkWithLight(LevelChunkWithLight {
                x: 3,
                z: -4,
                chunk: ChunkData {
                    heightmaps: vec![(4, vec![0; 37])],
                    buffer: vec![0, 0, 0, 0, 0, 0, 0, 0],
                    block_entities: vec![BlockEntityInfo {
                        packed_xz: 1,
                        y: 70,
                        kind: 3,
                        tag: Tag::End,
                    }],
                },
                light: LightData {
                    sky_mask: vec![1],
                    block_mask: vec![],
                    empty_sky_mask: vec![],
                    empty_block_mask: vec![2],
                    sky: vec![vec![0xFF; 2048]],
                    block: vec![],
                },
            }),
            Clientbound::Login(Login {
                player_id: 42,
                hardcore: false,
                levels: vec!["minecraft:overworld".into(), "minecraft:the_nether".into()],
                max_players: 20,
                chunk_radius: 10,
                simulation_distance: 10,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: spawn(),
                online_mode: false,
                enforces_secure_chat: false,
            }),
            Clientbound::MoveEntityPos(MoveEntityPos(MoveEntity {
                id: 3,
                delta: Some(VecDelta::Linear([4096, 0, -4096])),
                rotation: None,
                on_ground: true,
            })),
            Clientbound::MoveEntityPosRot(MoveEntityPosRot(MoveEntity {
                id: 3,
                delta: Some(VecDelta::Stepped(vec![([1, 2, 3], 1), ([0, 0, 1], 2)])),
                rotation: Some((10, -10)),
                on_ground: false,
            })),
            Clientbound::MoveEntityRot(MoveEntityRot(MoveEntity {
                id: 3,
                delta: None,
                rotation: Some((1, 2)),
                on_ground: true,
            })),
            Clientbound::OpenScreen(OpenScreen {
                container: 1,
                menu: 2,
                title: literal("Chest"),
            }),
            Clientbound::PlayerChat(PlayerChat {
                global_index: 0,
                sender: 5,
                index: 0,
                signature: None,
                content: "hello".into(),
                timestamp: 1,
                salt: 2,
                last_seen: vec![(3, None), (-1, Some(vec![7; 256]))],
                unsigned_content: None,
                filter: (0, vec![]),
                chat_type: ChatTypeBound {
                    chat_type: 0,
                    name: literal("Steve"),
                    target: None,
                },
            }),
            Clientbound::PlayerInfoUpdate(PlayerInfoUpdate {
                actions: 0b0001_1101,
                entries: vec![PlayerInfo {
                    uuid: 1,
                    name: Some("Steve".into()),
                    game_mode: Some(1),
                    listed: Some(true),
                    latency: Some(12),
                    ..PlayerInfo::default()
                }],
            }),
            Clientbound::PlayerPosition(PlayerPosition {
                teleport: 1,
                position: [0.5, 70.0, 0.5],
                delta: [0.0; 3],
                yaw: 90.0,
                pitch: 0.0,
                relatives: relative::DELTA_X,
            }),
            Clientbound::Respawn(Respawn {
                spawn: spawn(),
                keep: 3,
            }),
            Clientbound::SectionBlocksUpdate(SectionBlocksUpdate {
                section: [1, -4, 2],
                changes: vec![(9 << 12) | 0x123],
            }),
            Clientbound::SetEntityData(SetEntityData {
                id: 2,
                values: vec![(8, 7, Value::Item(Some(Stack::new(5, 3))))],
            }),
            Clientbound::SetEntityMotion(SetEntityMotion {
                id: 2,
                velocity: [0.0; 3],
            }),
            Clientbound::SetEquipment(SetEquipment {
                id: 2,
                slots: vec![(0, None), (5, Some(Stack::new(1, 1)))],
            }),
            Clientbound::SetHealth(SetHealth {
                health: 17.5,
                food: 18,
                saturation: 3.0,
            }),
            Clientbound::SetTime(SetTime {
                game_time: 100,
                clocks: vec![ClockState {
                    clock: 0,
                    total_ticks: 6000,
                    partial_tick: 0.0,
                    rate: 1.0,
                }],
            }),
            Clientbound::Sound(Sound {
                sound: (12, None),
                source: 4,
                x: 8,
                y: 9,
                z: 10,
                volume: 1.0,
                pitch: 1.0,
                seed: 3,
            }),
            Clientbound::Sound(Sound {
                sound: (-1, Some(("minecraft:x".into(), Some(16.0)))),
                source: 0,
                x: 0,
                y: 0,
                z: 0,
                volume: 1.0,
                pitch: 1.0,
                seed: 0,
            }),
            Clientbound::SystemChat(SystemChat {
                content: literal("hi"),
                overlay: false,
            }),
            Clientbound::TeleportEntity(TeleportEntity {
                id: 1,
                position: [1.0; 3],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: 0,
                on_ground: true,
            }),
        ];
        for p in packets {
            round_trip(p, Clientbound::encode, Clientbound::decode);
        }
    }

    #[test]
    fn serverbound_round_trips() {
        let packets = vec![
            Serverbound::AcceptTeleportation(AcceptTeleportation {
                id: 1,
                x: 0.5,
                y: 64.0,
                z: 0.5,
                yaw: 0.0,
                pitch: 0.0,
            }),
            Serverbound::Chat(Chat {
                message: "hello".into(),
                timestamp: 1,
                salt: 0,
                signature: None,
                last_seen: LastSeenUpdate {
                    offset: 0,
                    acknowledged: [0; 3],
                    checksum: 0,
                },
            }),
            Serverbound::ChatCommand(ChatCommand {
                command: "time set day".into(),
            }),
            Serverbound::ChunkBatchReceived(ChunkBatchReceived {
                chunks_per_tick: 9.5,
            }),
            Serverbound::ClientCommand(ClientCommand { action: 0 }),
            Serverbound::ClientTickEnd(Empty),
            Serverbound::ContainerClick(ContainerClick {
                container: 0,
                state: 5,
                slot: 36,
                button: 0,
                input: 0,
                changed: vec![(36, None)],
                carried: Some(Hashed::of(&Stack::new(1, 64))),
            }),
            Serverbound::MovePlayerPosRot(MovePlayerPosRot {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                yaw: 4.0,
                pitch: 5.0,
                flags: 1,
            }),
            Serverbound::MovePlayerStatus(MovePlayerStatus { flags: 3 }),
            Serverbound::PlayerAction(PlayerAction {
                action: 0,
                pos: [1, 2, 3],
                direction: 1,
                sequence: 7,
            }),
            Serverbound::PlayerInput(PlayerInput { flags: 0b0101_0001 }),
            Serverbound::PlayerLoaded(Empty),
            Serverbound::Punch(Empty),
            Serverbound::SetCarriedItem(SetCarriedItem { slot: 4 }),
            Serverbound::UseItemOn(UseItemOn {
                hand: 0,
                pos: [1, 64, 1],
                face: 1,
                cursor: [0.5, 1.0, 0.5],
                inside: false,
                world_border: false,
                sequence: 8,
            }),
            Serverbound::Interact(Interact {
                id: 9,
                hand: 0,
                location: [0.25, 1.0, 0.0],
                secondary: false,
            }),
        ];
        for p in packets {
            if let Serverbound::Interact(_) = &p {
                // LpVec3 is lossy; check bytes instead.
                let body = p.encode();
                let mut r = Reader::new(&body);
                let id = r.varint().unwrap();
                assert!(matches!(
                    Serverbound::decode(id, r.rest()).unwrap(),
                    Serverbound::Interact(Interact { id: 9, .. })
                ));
                continue;
            }
            round_trip(p, Serverbound::encode, Serverbound::decode);
        }
    }

    #[test]
    fn move_entity_deltas_decode_like_vec_delta_codec() {
        let base = [10.0, 64.0, -3.5];
        assert_eq!(
            VecDelta::Linear([4096, 0, -2048]).end(base),
            [11.0, 64.0, -4.0]
        );
        // Zero deltas keep unquantized base coordinates exactly.
        assert_eq!(
            VecDelta::Linear([0, 0, 0]).end([0.123_456_789, 1.0, 2.0]),
            [0.123_456_789, 1.0, 2.0]
        );
        assert_eq!(
            VecDelta::Stepped(vec![([4096, 0, 0], 1), ([4096, 0, 0], 2)]).end(base),
            [12.0, 64.0, -3.5]
        );
    }

    #[test]
    fn unknown_packets_keep_their_payload() {
        let p = Clientbound::decode(crate::ids::play::clientbound::BOSS_EVENT, &[1, 2, 3]).unwrap();
        assert_eq!(
            p,
            Clientbound::Unknown(crate::ids::play::clientbound::BOSS_EVENT, vec![1, 2, 3])
        );
        assert_eq!(p.encode()[1..], [1, 2, 3]);
    }
}
