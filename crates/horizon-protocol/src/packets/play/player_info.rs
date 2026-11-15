//! The player list packets: `ClientboundPlayerInfoRemovePacket` and
//! `ClientboundPlayerInfoUpdatePacket`.
use crate::{
    Result,
    buf::{Codec, List, Reader, Uuid, Writer},
    packet,
    packets::login::Property,
};
use horizon_world::nbt::Tag;

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
