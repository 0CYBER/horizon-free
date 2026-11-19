//! 1.21.5 to 1.21.4 (protocol 770 to 769): the mirror of `translate`'s `v1_21_4` (1.21.4 to 1.21.5), for a
//! [`super::ClientSession`] whose client is 1.21.4 (roadmap `older-clients`).
//!
//! Clientbound, what 1.21.5 added is taken out again: the explosion's player knockback is a double and two floats,
//! the player chat has no leading global index, the team's name tag visibility and collision rule are names rather
//! than VarInt enums, and the advancements have no trailing show flag.
//! Serverbound, the chat packets have no trailing checksum (the client sends none: the server reads a 0 as "not
//! checked"), and a container click's changed slots and carried item are full stacks, not hashes: they are hashed
//! here with `HashedStack`'s rules (`Hashed::of`), with the client's item ids mapped to the pinned ones first.
//!
//! TODO(step3): the hashes of a stack with a component that names a registry entry are incomplete (`Hashed::of`
//! hashes without registry names), and 1.21.5's item components are not written in 1.21.4's layout (see the
//! TODOs in `translate::v1_21_4`).
use super::text_down as text;
use crate::{
    Result, State,
    adapter::{
        ids::{self, Ids, Way},
        translate,
    },
    buf::{Codec, List, Reader, Writer},
    chunk::{BlockEntityInfo, storage_len},
    ids as packet_ids,
    item::{Hashed, HashedSlot, Slot, Stack},
    packets::play,
};
use horizon_core::nbt::{JavaString, Tag};
use horizon_world::datafix::json::{self, Json};

/// What becomes of a payload the 1.21.4 client sent.
#[derive(Debug, PartialEq)]
pub(super) enum Out {
    /// Not one of the packets this step rewrites.
    Untouched,
    Rewritten(Vec<u8>),
}

/// The clientbound packet `id` (`payload` in 1.21.5's layout) in 1.21.4's layout; `None` when it is laid out alike.
pub(super) fn clientbound(state: State, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    use packet_ids::play::clientbound as cb;
    if state != State::Play {
        return Ok(None);
    }
    Ok(Some(match id {
        cb::EXPLODE => explode_down(payload)?,
        cb::PLAYER_CHAT => {
            // The global index goes; 1.21.4 numbers no messages globally.
            // A packet the text rewrite can't read keeps its text as it is.
            let text = text_packet_down(id, payload).ok().flatten();
            let mut r = Reader::new(text.as_deref().unwrap_or(payload));
            r.varint()?;
            r.rest().to_vec()
        }
        cb::SYSTEM_CHAT
        | cb::DISGUISED_CHAT
        | cb::TAB_LIST
        | cb::SET_TITLE_TEXT
        | cb::SET_SUBTITLE_TEXT
        | cb::SET_ACTION_BAR_TEXT
        | cb::DISCONNECT
        | cb::OPEN_SCREEN
        | cb::PLAYER_COMBAT_KILL
        | cb::SET_OBJECTIVE => match text_packet_down(id, payload)? {
            Some(p) => p,
            None => return Ok(None),
        },
        cb::SET_PLAYER_TEAM => team_down(payload)?,
        // The show flag is the last field; 1.21.4 has none.
        cb::UPDATE_ADVANCEMENTS => payload[..payload.len().saturating_sub(1)].to_vec(),
        cb::LEVEL_CHUNK_WITH_LIGHT => chunk_down(payload)?,
        cb::BLOCK_ENTITY_DATA => block_entity_data_down(payload)?,
        cb::CHUNKS_BIOMES => chunks_biomes_down(payload)?,
        _ => return Ok(None),
    }))
}

/// A packet that carries text components, with their click and hover events in 1.21.4's form: `None` for any
/// other packet. Not converted yet: boss bars, entity data (custom names, text displays), books, resource pack
/// prompts, server links and the configuration state's disconnect.
fn text_packet_down(id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    use play::Clientbound as P;
    let mut p = P::decode(id, payload)?;
    let down = text::component_down;
    match &mut p {
        P::SystemChat(c) => down(&mut c.content),
        P::DisguisedChat(c) => {
            down(&mut c.message);
            down(&mut c.chat_type.name);
            c.chat_type.target.as_mut().map(down);
        }
        P::PlayerChat(c) => {
            c.unsigned_content.as_mut().map(down);
            down(&mut c.chat_type.name);
            c.chat_type.target.as_mut().map(down);
        }
        P::TabList(t) => {
            down(&mut t.header);
            down(&mut t.footer);
        }
        P::SetTitleText(t) | P::SetSubtitleText(t) | P::SetActionBarText(t) => down(&mut t.text),
        P::Disconnect(d) => down(&mut d.reason),
        P::OpenScreen(o) => down(&mut o.title),
        P::PlayerCombatKill(k) => down(&mut k.message),
        P::SetObjective(o) => {
            if let Some((name, ..)) = &mut o.display {
                down(name);
            }
        }
        _ => return Ok(None),
    }
    let encoded = p.encode();
    let mut r = Reader::new(&encoded);
    r.varint()?;
    Ok(Some(r.rest().to_vec()))
}

/// The serverbound packet `id` the 1.21.4 client sent (`payload`, under the pinned id) in 1.21.5's layout. `ids` is
/// the 1.21.4 table, which the stacks of a container click are read by.
pub(super) fn serverbound(
    state: State,
    id: i32,
    payload: &[u8],
    ids: Option<&'static Ids>,
) -> Result<Out> {
    use packet_ids::play::serverbound as sb;
    if state != State::Play {
        return Ok(Out::Untouched);
    }
    Ok(match id {
        sb::CHAT | sb::CHAT_COMMAND_SIGNED => {
            // 0 is the checksum 1.21.5 reads as "not checked".
            let mut p = payload.to_vec();
            p.push(0);
            Out::Rewritten(p)
        }
        sb::CONTAINER_CLICK => match ids {
            Some(ids) => Out::Rewritten(container_click(ids, payload)?),
            None => Out::Untouched,
        },
        _ => Out::Untouched,
    })
}

/// Entity data in 1.21.4's indexes, from 1.21.5's (`translate::client_layout_v1_21_4::remap_entity_data`, back). `kind` is the
/// entity's type in the pinned ids, `None` when unknown, which leaves the data as it is.
pub(super) fn entity_data(kind: Option<i32>, data: &mut play::SetEntityData) {
    if let Some(kind) = kind {
        translate::client_layout_v1_21_4::remap_entity_data(kind, data, false);
    }
}

/// A paletted container as 1.21.5 sends it, back to 1.21.4's: its data array's length (a VarInt) comes before
/// the longs, computed from the bits per entry (`storage_len`, with a block palette stored in at least 4 bits).
fn paletted_down(r: &mut Reader, w: &mut Writer, blocks: bool) -> Result<()> {
    let bits = r.u8()?;
    w.u8(bits);
    let mut storage = bits;
    if bits == 0 {
        w.varint(r.varint()?);
    } else if translate::client_layout_v1_21_4::indirect(bits, blocks) {
        let n = r.varint()?;
        w.varint(n);
        for _ in 0..n {
            w.varint(r.varint()?);
        }
        if blocks {
            storage = bits.max(4);
        }
    }
    let entries = if blocks { 4096 } else { 64 };
    let longs = storage_len(storage, entries);
    w.varint(longs as i32);
    w.bytes(r.bytes(longs * 8)?);
    Ok(())
}

/// The sections of a chunk column (or only their biomes) in 1.21.5's layout, back to 1.21.4's.
fn sections_down(data: &[u8], biomes_only: bool) -> Result<Vec<u8>> {
    let mut r = Reader::new(data);
    let mut w = Writer::new();
    while r.remaining() > 0 {
        if !biomes_only {
            w.i16(r.i16()?);
            paletted_down(&mut r, &mut w, true)?;
        }
        paletted_down(&mut r, &mut w, false)?;
    }
    Ok(w.data)
}

/// The heightmaps as 1.21.4 sends them: an NBT compound with a long array per name. A type 1.21.4 has no name for
/// is dropped.
fn heightmaps_down(r: &mut Reader, w: &mut Writer) -> Result<()> {
    let n = r.count(16)?;
    let mut map = std::collections::BTreeMap::new();
    for _ in 0..n {
        let kind = r.varint()?;
        let longs = r.long_array()?;
        if let Some(&(_, name)) = translate::client_layout_v1_21_4::HEIGHTMAPS
            .iter()
            .find(|(k, _)| *k == kind)
        {
            map.insert(
                horizon_core::nbt::JavaString::from(name),
                horizon_core::nbt::Tag::LongArray(longs),
            );
        }
    }
    w.nbt(&horizon_core::nbt::Tag::Compound(map));
    Ok(())
}

/// Level chunk with light, 1.21.5's layout to 1.21.4's (the mirror of `translate::client_layout_v1_21_4::chunk_up`).
fn chunk_down(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.i32(r.i32()?);
    w.i32(r.i32()?);
    heightmaps_down(&mut r, &mut w)?;
    let data = r.byte_array(r.remaining())?;
    w.byte_array(&sections_down(&data, false)?);
    // The column's block entities (signs' text, custom names), then the light data, which is laid out alike.
    let mut block_entities = <List<BlockEntityInfo>>::read(&mut r)?;
    for b in &mut block_entities {
        block_entity_text_down(&mut b.tag);
    }
    <List<BlockEntityInfo>>::write(&block_entities, &mut w);
    w.bytes(r.rest());
    Ok(w.data)
}

/// block_entity_data: the block entity's update tag, its text as 1.21.4 reads it.
fn block_entity_data_down(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.i64(r.i64()?);
    w.varint(r.varint()?);
    let mut tag = r.nbt()?;
    block_entity_text_down(&mut tag);
    w.nbt(&tag);
    w.bytes(r.rest());
    Ok(w.data)
}

/// Until 1.21.4 a block entity's text components are JSON strings in its NBT (`SignText` and `CustomName` read
/// `ComponentSerialization.FLAT_CODEC`); 1.21.5 made them NBT components, a bare string for plain text. A sign's
/// four `messages` and `filtered_messages` per side, and `CustomName`, become JSON text (the lines of a sign
/// that mixes plain and styled text arrive wrapped as `{"": line}`, which goes first).
///
/// Their click and hover events take 1.21.4's spelling first (`text::component_down`).
fn block_entity_text_down(tag: &mut Tag) {
    let Tag::Compound(root) = tag else {
        return;
    };
    for side in ["front_text", "back_text"] {
        let Some(Tag::Compound(text)) = root.get_mut(&JavaString::from(side)) else {
            continue;
        };
        for list in ["messages", "filtered_messages"] {
            if let Some(Tag::List(lines)) = text.get_mut(&JavaString::from(list)) {
                lines.iter_mut().for_each(|line| *line = json_text(line));
            }
        }
    }
    if let Some(name) = root.get_mut(&JavaString::from("CustomName")) {
        *name = json_text(name);
    }
}

/// A text component in NBT form as a string of its JSON.
fn json_text(tag: &Tag) -> Tag {
    let mut tag = tag.clone();
    text::component_down(&mut tag);
    Tag::String(JavaString(json::to_stable_string(&text_json(&tag))))
}

fn text_json(tag: &Tag) -> Json {
    match tag {
        Tag::String(s) => Json::Str(s.0.clone()),
        Tag::List(items) => Json::Arr(items.iter().map(text_json).collect()),
        Tag::Compound(c) => {
            // A list of mixed elements wraps each of them: `{"": element}`.
            if let (1, Some(wrapped)) = (c.len(), c.get(&JavaString::from(""))) {
                return text_json(wrapped);
            }
            Json::Obj(
                c.iter()
                    .map(|(key, value)| {
                        let flag = [
                            "bold",
                            "italic",
                            "underlined",
                            "strikethrough",
                            "obfuscated",
                        ]
                        .contains(&key.to_string_lossy().as_str());
                        let value = match value {
                            Tag::Byte(b) if flag => Json::Bool(*b != 0),
                            value => text_json(value),
                        };
                        (key.0.clone(), value)
                    })
                    .collect(),
            )
        }
        other => json::from_nbt(other),
    }
}

/// Chunk biomes, 1.21.5's layout to 1.21.4's (the mirror of `translate::client_layout_v1_21_4::chunks_biomes_up`).
fn chunks_biomes_down(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.i32(r.i32()?);
    w.i32(r.i32()?);
    let data = r.byte_array(r.remaining())?;
    w.byte_array(&sections_down(&data, true)?);
    w.bytes(r.rest());
    Ok(w.data)
}

/// The explosion's player knockback: a double and two floats in 1.21.4, three doubles in 1.21.5.
fn explode_down(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    for _ in 0..3 {
        w.f64(r.f64()?);
    }
    let knockback = r.bool()?;
    w.bool(knockback);
    if knockback {
        w.f64(r.f64()?);
        w.f32(r.f64()? as f32);
        w.f32(r.f64()? as f32);
    }
    w.bytes(r.rest());
    Ok(w.data)
}

fn text_nbt(mut tag: Tag) -> Tag {
    text::component_down(&mut tag);
    tag
}

/// An update to a team in 1.21.4's layout: the name tag visibility and collision rule are sent by name.
fn team_down(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.string(&r.string(32767)?);
    let method = r.u8()?;
    w.u8(method);
    if method == 0 || method == 2 {
        w.nbt(&text_nbt(r.nbt()?));
        w.u8(r.u8()?);
        w.string(translate::client_layout_v1_21_4::name_of(
            &translate::client_layout_v1_21_4::VISIBILITY,
            r.varint()?,
        )?);
        w.string(translate::client_layout_v1_21_4::name_of(
            &translate::client_layout_v1_21_4::COLLISION,
            r.varint()?,
        )?);
        w.varint(r.varint()?);
        w.nbt(&text_nbt(r.nbt()?));
        w.nbt(&text_nbt(r.nbt()?));
    }
    if method == 0 || method == 3 || method == 4 {
        let n = r.count(r.remaining())?;
        w.varint(n as i32);
        for _ in 0..n {
            w.string(&r.string(32767)?);
        }
    }
    w.bytes(r.rest());
    Ok(w.data)
}

/// A container click with its stacks as 1.21.4 sends them (full slots, item ids of 1.21.4) in 1.21.5's hashed layout.
fn container_click(ids: &'static Ids, payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    w.varint(r.varint()?);
    w.varint(r.varint()?);
    w.i16(r.i16()?);
    w.i8(r.i8()?);
    w.varint(r.varint()?);
    let n = r.count(r.remaining())?;
    w.varint(n as i32);
    for _ in 0..n {
        w.i16(r.i16()?);
        let stack = slot(ids, &mut r)?;
        hashed_slot(ids, stack, &mut w);
    }
    let carried = slot(ids, &mut r)?;
    hashed_slot(ids, carried, &mut w);
    w.bytes(r.rest());
    Ok(w.data)
}

/// A full slot as 1.21.4 wrote it, its item and component ids mapped to the pinned ones.
fn slot(ids: &'static Ids, r: &mut Reader<'_>) -> Result<Option<Stack>> {
    let _up = ids::renumbering(ids, Way::Up);
    Slot::read(r)
}

/// A hashed slot in the client's own ids, as a 1.21.5 client sends it: the session maps the whole
/// click to the pinned ids afterwards, as for every newer client.
fn hashed_slot(ids: &'static Ids, stack: Option<Stack>, w: &mut Writer) {
    let _down = ids::renumbering(ids, Way::Down);
    HashedSlot::write(&hashed(stack), w);
}

fn hashed(stack: Option<Stack>) -> Option<Hashed> {
    stack.as_ref().map(Hashed::of)
}

#[cfg(test)]
mod tests {
    //! Each change is written here and read back with `translate`'s 1.21.4 reader: the round trip.
    use super::*;
    use crate::adapter::{ServerVersion, session::Session, translate};
    use crate::entity_data::{Value, serializer};

    /// An empty compound: a text component (network NBT, no root name) with no text.
    const EMPTY_TEXT: [u8; 2] = [0x0a, 0x00];

    fn read_back_clientbound(id: i32, down: &[u8]) -> Vec<u8> {
        let session = Session::new(ServerVersion::V1_21_4);
        translate::client_layout_v1_21_4::clientbound(&session, State::Play, id, down)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn an_explosion_knockback_is_three_doubles_and_back() {
        let mut w = Writer::new();
        w.f64(1.5);
        w.f64(-2.0);
        w.f64(3.25);
        w.bool(true);
        w.f64(0.5);
        w.f64(1.0);
        w.f64(-0.25);
        w.varint(7);
        w.bytes(&[9, 9]);
        let pinned = w.data;
        let id = packet_ids::play::clientbound::EXPLODE;
        let older = clientbound(State::Play, id, &pinned).unwrap().unwrap();
        assert_ne!(older, pinned);
        assert_eq!(read_back_clientbound(id, &older), pinned);
    }

    #[test]
    fn an_explosion_without_knockback_is_the_same_in_both() {
        let mut w = Writer::new();
        w.f64(1.0);
        w.f64(2.0);
        w.f64(3.0);
        w.bool(false);
        w.varint(0);
        let pinned = w.data;
        let id = packet_ids::play::clientbound::EXPLODE;
        let older = clientbound(State::Play, id, &pinned).unwrap().unwrap();
        assert_eq!(older, pinned);
        assert_eq!(read_back_clientbound(id, &older), pinned);
    }

    #[test]
    fn a_player_chat_has_no_global_index_in_1_21_4() {
        let older: [u8; 3] = [1, 2, 3];
        let id = packet_ids::play::clientbound::PLAYER_CHAT;
        let up = read_back_clientbound(id, &older);
        assert_eq!(up, vec![0u8, 1, 2, 3]);
        let pinned: [u8; 4] = [5, 1, 2, 3];
        let down = clientbound(State::Play, id, &pinned).unwrap().unwrap();
        assert_eq!(down, vec![1u8, 2, 3]);
    }

    #[test]
    fn a_team_names_its_visibility_and_collision_and_reads_back() {
        let id = packet_ids::play::clientbound::SET_PLAYER_TEAM;
        let mut w = Writer::new();
        w.string("red");
        w.u8(0); // create
        w.bytes(&EMPTY_TEXT);
        w.u8(1);
        w.string("hideForOwnTeam");
        w.string("pushOwnTeam");
        w.varint(4);
        w.bytes(&EMPTY_TEXT);
        w.bytes(&EMPTY_TEXT);
        w.varint(1);
        w.string("Steve");
        let older = w.data;
        let up = read_back_clientbound(id, &older);
        let mut r = Reader::new(&up);
        assert_eq!(r.string(32767).unwrap(), "red");
        assert_eq!(r.u8().unwrap(), 0);
        r.nbt().unwrap();
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.varint().unwrap(), 3); // hideForOwnTeam
        assert_eq!(r.varint().unwrap(), 3); // pushOwnTeam
        assert_eq!(clientbound(State::Play, id, &up).unwrap().unwrap(), older);
    }

    #[test]
    fn a_team_removal_is_only_its_name_and_method() {
        let id = packet_ids::play::clientbound::SET_PLAYER_TEAM;
        let mut w = Writer::new();
        w.string("red");
        w.u8(1);
        let older = w.data;
        let up = read_back_clientbound(id, &older);
        assert_eq!(clientbound(State::Play, id, &up).unwrap().unwrap(), older);
    }

    #[test]
    fn an_advancement_update_loses_its_show_flag() {
        let id = packet_ids::play::clientbound::UPDATE_ADVANCEMENTS;
        let mut w = Writer::new();
        w.bool(false); // reset
        w.varint(0);
        w.varint(0);
        let older = w.data;
        let up = read_back_clientbound(id, &older);
        assert_eq!(up.last(), Some(&0));
        assert_eq!(clientbound(State::Play, id, &up).unwrap().unwrap(), older);
    }

    #[test]
    fn a_chat_message_has_no_checksum_in_1_21_4() {
        let id = packet_ids::play::serverbound::CHAT;
        let mut w = Writer::new();
        w.string("hi");
        w.i64(1);
        w.i64(2);
        w.bool(false);
        w.varint(0);
        w.bytes(&[0, 0, 0]);
        let older = w.data;
        let Out::Rewritten(pinned) = serverbound(State::Play, id, &older, None).unwrap() else {
            panic!("a chat is rewritten");
        };
        assert_eq!(pinned.last(), Some(&0));
        assert_eq!(
            translate::client_layout_v1_21_4::without_checksum(&pinned),
            older
        );
    }

    /// A level chunk in 1.21.5's layout: two heightmaps, then a single-valued section, an indirect one (4 bits
    /// over a block palette) with direct biomes (6 bits).
    fn chunk_1_21_5() -> Vec<u8> {
        let mut sections = Writer::new();
        sections.i16(5);
        sections.u8(0);
        sections.varint(1);
        sections.u8(0);
        sections.varint(2);
        sections.i16(4096);
        sections.u8(4);
        sections.varint(2);
        sections.varint(7);
        sections.varint(9);
        for _ in 0..storage_len(4, 4096) {
            sections.i64(0x1234);
        }
        sections.u8(6);
        for _ in 0..storage_len(6, 64) {
            sections.i64(3);
        }
        let mut w = Writer::new();
        w.i32(3);
        w.i32(-7);
        w.varint(2);
        w.varint(1);
        w.long_array(&[1, 2]);
        w.varint(4);
        w.long_array(&[3]);
        w.byte_array(&sections.data);
        // No block entities, then the light data (opaque here).
        w.varint(0);
        w.bytes(&[9, 9]);
        w.data
    }

    #[test]
    fn a_level_chunk_round_trips_through_1_21_4s_layout() {
        let id = packet_ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT;
        let newer = chunk_1_21_5();
        let older = clientbound(State::Play, id, &newer).unwrap().unwrap();
        assert_ne!(older, newer);
        assert_eq!(
            translate::client_layout_v1_21_4::chunk_up(&older).unwrap(),
            newer
        );
    }

    fn string(s: &str) -> Tag {
        Tag::String(JavaString::from(s))
    }

    /// A sign's update tag in 1.21.5's form (NBT components) and the same sign as 1.21.4's `SignText` reads it:
    /// every line a JSON string. The expected strings are written out by hand.
    fn signs() -> (Tag, Tag) {
        let side = |lines: Vec<Tag>| {
            Tag::compound([
                ("messages", Tag::List(lines)),
                ("color", string("black")),
                ("has_glowing_text", Tag::Byte(0)),
            ])
        };
        let new = Tag::compound([
            (
                "front_text",
                side(vec![
                    string(""),
                    string("Hello World"),
                    string("say \"hi\""),
                    string(""),
                ]),
            ),
            (
                "back_text",
                side(vec![
                    // A list mixing plain and styled lines wraps each element.
                    Tag::compound([("", string("Plain"))]),
                    Tag::compound([
                        ("text", string("Hi")),
                        ("bold", Tag::Byte(1)),
                        ("italic", Tag::Byte(0)),
                        ("color", string("red")),
                    ]),
                    Tag::compound([("", string(""))]),
                    Tag::compound([("", string("\u{e9}"))]),
                ]),
            ),
            ("CustomName", string("Chest")),
            ("is_waxed", Tag::Byte(1)),
        ]);
        let old = Tag::compound([
            (
                "front_text",
                side(vec![
                    string("\"\""),
                    string("\"Hello World\""),
                    string("\"say \\\"hi\\\"\""),
                    string("\"\""),
                ]),
            ),
            (
                "back_text",
                side(vec![
                    string("\"Plain\""),
                    string("{\"bold\":true,\"color\":\"red\",\"italic\":false,\"text\":\"Hi\"}"),
                    string("\"\""),
                    string("\"\u{e9}\""),
                ]),
            ),
            ("CustomName", string("\"Chest\"")),
            ("is_waxed", Tag::Byte(1)),
        ]);
        (new, old)
    }

    #[test]
    fn a_signs_lines_are_json_strings_in_block_entity_data() {
        let (new, old) = signs();
        let mut w = Writer::new();
        w.i64(crate::buf::pack_block_pos([4, 70, -9]));
        w.varint(7);
        w.nbt(&new);
        let id = packet_ids::play::clientbound::BLOCK_ENTITY_DATA;
        let down = clientbound(State::Play, id, &w.data).unwrap().unwrap();
        let mut r = Reader::new(&down);
        assert_eq!(r.i64().unwrap(), crate::buf::pack_block_pos([4, 70, -9]));
        assert_eq!(r.varint().unwrap(), 7);
        assert_eq!(r.nbt().unwrap(), old);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn a_signs_lines_are_json_strings_in_a_level_chunk() {
        let (new, old) = signs();
        let tail = |tag: &Tag| {
            let mut w = Writer::new();
            w.varint(1);
            w.i8(0x25);
            w.i16(70);
            w.varint(7);
            w.nbt(tag);
            // The light data (opaque here).
            w.bytes(&[9, 9]);
            w.data
        };
        let full = chunk_1_21_5();
        let head = &full[..full.len() - 3];
        let id = packet_ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT;
        let down = clientbound(State::Play, id, &[head, &tail(&new)].concat())
            .unwrap()
            .unwrap();
        // The 1.21.4 column has the same entries up to the block entities as the one without any.
        let plain = clientbound(State::Play, id, &full).unwrap().unwrap();
        let sections = plain.len() - 3;
        assert_eq!(&down[..sections], &plain[..sections]);
        assert_eq!(&down[sections..], &tail(&old)[..]);
    }

    fn click_new() -> Tag {
        Tag::compound([
            ("text", string("link")),
            (
                "click_event",
                Tag::compound([
                    ("action", string("open_url")),
                    ("url", string("https://example.com")),
                ]),
            ),
            (
                "hover_event",
                Tag::compound([("action", string("show_text")), ("value", string("tip"))]),
            ),
        ])
    }

    fn click_old() -> Tag {
        Tag::compound([
            ("text", string("link")),
            (
                "clickEvent",
                Tag::compound([
                    ("action", string("open_url")),
                    ("value", string("https://example.com")),
                ]),
            ),
            (
                "hoverEvent",
                Tag::compound([("action", string("show_text")), ("contents", string("tip"))]),
            ),
        ])
    }

    #[test]
    fn a_system_chat_keeps_the_old_event_spelling() {
        let id = packet_ids::play::clientbound::SYSTEM_CHAT;
        let mut w = Writer::new();
        w.nbt(&click_new());
        w.bool(false);
        let down = clientbound(State::Play, id, &w.data).unwrap().unwrap();
        let mut r = Reader::new(&down);
        assert_eq!(r.nbt().unwrap(), click_old());
        assert!(!r.bool().unwrap());
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn a_tab_list_and_a_disguised_chat_keep_the_old_event_spelling() {
        let tab = packet_ids::play::clientbound::TAB_LIST;
        let mut w = Writer::new();
        w.nbt(&click_new());
        w.nbt(&string("plain"));
        let down = clientbound(State::Play, tab, &w.data).unwrap().unwrap();
        let mut r = Reader::new(&down);
        assert_eq!(r.nbt().unwrap(), click_old());
        assert_eq!(r.nbt().unwrap(), string("plain"));
        let chat = play::Clientbound::DisguisedChat(play::DisguisedChat {
            message: click_new(),
            chat_type: play::ChatTypeBound {
                chat_type: 1,
                name: click_new(),
                target: Some(click_new()),
            },
        });
        let encoded = chat.encode();
        let mut r = Reader::new(&encoded);
        let id = r.varint().unwrap();
        let down = clientbound(State::Play, id, r.rest()).unwrap().unwrap();
        match play::Clientbound::decode(id, &down).unwrap() {
            play::Clientbound::DisguisedChat(c) => {
                assert_eq!(c.message, click_old());
                assert_eq!(c.chat_type.name, click_old());
                assert_eq!(c.chat_type.target, Some(click_old()));
            }
            _ => panic!("disguised chat"),
        }
    }

    #[test]
    fn a_player_chat_loses_its_global_index_and_keeps_the_old_event_spelling() {
        let chat = |global_index, unsigned: Tag| {
            play::Clientbound::PlayerChat(play::PlayerChat {
                global_index,
                sender: 7,
                index: 3,
                signature: None,
                content: "hi".into(),
                timestamp: 100,
                salt: 9,
                last_seen: Vec::new(),
                unsigned_content: Some(unsigned),
                filter: (0, Vec::new()),
                chat_type: play::ChatTypeBound {
                    chat_type: 1,
                    name: click_new(),
                    target: None,
                },
            })
            .encode()
        };
        let encoded = chat(5, click_new());
        let mut r = Reader::new(&encoded);
        let id = r.varint().unwrap();
        let down = clientbound(State::Play, id, r.rest()).unwrap().unwrap();
        // 1.21.4's packet is 1.21.5's without the index: put a 0 index back to read it.
        let mut w = Writer::new();
        w.varint(0);
        w.bytes(&down);
        let Ok(play::Clientbound::PlayerChat(c)) = play::Clientbound::decode(id, &w.data) else {
            panic!("player chat");
        };
        assert_eq!((c.sender, c.index, c.content.as_str()), (7, 3, "hi"));
        assert_eq!(c.unsigned_content, Some(click_old()));
        assert_eq!(c.chat_type.name, click_old());
    }

    #[test]
    fn a_signs_styled_line_has_the_old_event_spelling() {
        let mut tag = Tag::compound([(
            "front_text",
            Tag::compound([("messages", Tag::List(vec![click_new()]))]),
        )]);
        block_entity_text_down(&mut tag);
        let line = tag.get("front_text").unwrap().get("messages").unwrap();
        assert_eq!(
            line,
            &Tag::List(vec![string(
                "{\"clickEvent\":{\"action\":\"open_url\",\"value\":\"https://example.com\"},\"hoverEvent\":{\"action\":\"show_text\",\"contents\":\"tip\"},\"text\":\"link\"}"
            )])
        );
    }

    #[test]
    fn block_entity_text_that_is_not_a_sign_is_left_alone() {
        let mut tag = Tag::compound([
            ("Items", Tag::List(Vec::new())),
            ("id", string("minecraft:chest")),
        ]);
        let before = tag.clone();
        block_entity_text_down(&mut tag);
        assert_eq!(tag, before);
        let mut end = Tag::End;
        block_entity_text_down(&mut end);
        assert_eq!(end, Tag::End);
    }

    #[test]
    fn a_1_21_4_heightmap_is_named_and_its_type_is_dropped_when_unknown() {
        let mut w = Writer::new();
        w.i32(0);
        w.i32(0);
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            horizon_core::nbt::JavaString::from("WORLD_SURFACE"),
            horizon_core::nbt::Tag::LongArray(vec![5]),
        );
        map.insert(
            horizon_core::nbt::JavaString::from("WORLD_SURFACE_WG"),
            horizon_core::nbt::Tag::LongArray(vec![6]),
        );
        w.nbt(&horizon_core::nbt::Tag::Compound(map));
        w.byte_array(&[]);
        let up = translate::client_layout_v1_21_4::chunk_up(&w.data).unwrap();
        let mut r = Reader::new(&up);
        assert_eq!((r.i32().unwrap(), r.i32().unwrap()), (0, 0));
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.varint().unwrap(), 1); // WORLD_SURFACE
        assert_eq!(r.long_array().unwrap(), vec![5]);
    }

    #[test]
    fn chunk_biomes_round_trip_through_1_21_4s_layout() {
        let id = packet_ids::play::clientbound::CHUNKS_BIOMES;
        let mut biomes = Writer::new();
        biomes.u8(0);
        biomes.varint(2);
        biomes.u8(6);
        for _ in 0..storage_len(6, 64) {
            biomes.i64(3);
        }
        let mut w = Writer::new();
        w.i32(4);
        w.i32(-1);
        w.byte_array(&biomes.data);
        let newer = w.data;
        let older = clientbound(State::Play, id, &newer).unwrap().unwrap();
        assert_ne!(older, newer);
        assert_eq!(
            translate::client_layout_v1_21_4::chunks_biomes_up(&older).unwrap(),
            newer
        );
    }

    /// Entity data of `kind` taken through translate's way up, then this step's way back.
    fn up_then_down(
        kind: i32,
        older: Vec<(u8, i32, Value)>,
    ) -> (Vec<(u8, i32, Value)>, Vec<(u8, i32, Value)>) {
        let mut d = play::SetEntityData {
            id: 1,
            values: older,
        };
        translate::client_layout_v1_21_4::remap_entity_data(kind, &mut d, true);
        let newer = d.values.clone();
        entity_data(Some(kind), &mut d);
        (newer, d.values)
    }

    #[test]
    fn a_dolphin_loses_its_treasure_position_and_keeps_the_rest() {
        let older = vec![
            (17, serializer::BLOCK_POS, Value::Pos([1, 2, 3])),
            (18, serializer::BOOLEAN, Value::Bool(true)),
            (19, serializer::INT, Value::Int(5)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::DOLPHIN, older);
        assert_eq!(
            newer,
            vec![
                (17, serializer::BOOLEAN, Value::Bool(true)),
                (18, serializer::INT, Value::Int(5)),
            ]
        );
        assert_eq!(
            back,
            vec![
                (18, serializer::BOOLEAN, Value::Bool(true)),
                (19, serializer::INT, Value::Int(5)),
            ]
        );
    }

    #[test]
    fn a_turtle_keeps_its_eggs_and_loses_its_positions() {
        let older = vec![
            (17, serializer::BLOCK_POS, Value::Pos([1, 2, 3])),
            (18, serializer::BOOLEAN, Value::Bool(true)),
            (19, serializer::BOOLEAN, Value::Bool(false)),
            (20, serializer::BLOCK_POS, Value::Pos([4, 5, 6])),
            (21, serializer::BOOLEAN, Value::Bool(true)),
            (22, serializer::BOOLEAN, Value::Bool(true)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::TURTLE, older);
        assert_eq!(
            newer,
            vec![
                (17, serializer::BOOLEAN, Value::Bool(true)),
                (18, serializer::BOOLEAN, Value::Bool(false)),
            ]
        );
        assert_eq!(
            back,
            vec![
                (18, serializer::BOOLEAN, Value::Bool(true)),
                (19, serializer::BOOLEAN, Value::Bool(false)),
            ]
        );
    }

    #[test]
    fn a_strider_loses_its_saddle() {
        let older = vec![
            (17, serializer::INT, Value::Int(3)),
            (18, serializer::BOOLEAN, Value::Bool(true)),
            (19, serializer::BOOLEAN, Value::Bool(true)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::STRIDER, older);
        let kept = vec![
            (17, serializer::INT, Value::Int(3)),
            (18, serializer::BOOLEAN, Value::Bool(true)),
        ];
        assert_eq!(newer, kept);
        assert_eq!(back, kept);
    }

    #[test]
    fn a_pig_loses_its_saddle_and_gains_a_variant_it_is_not_sent() {
        let older = vec![
            (17, serializer::BOOLEAN, Value::Bool(true)),
            (18, serializer::INT, Value::Int(7)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::PIG, older);
        assert_eq!(newer, vec![(17, serializer::INT, Value::Int(7))]);
        assert_eq!(back, vec![(18, serializer::INT, Value::Int(7))]);
    }

    #[test]
    fn a_mooshroom_s_type_is_a_string_then_a_varint() {
        let older = vec![(17, serializer::STRING, Value::String("brown".into()))];
        let (newer, back) =
            up_then_down(translate::client_layout_v1_21_4::MOOSHROOM, older.clone());
        assert_eq!(newer, vec![(17, serializer::INT, Value::Int(1))]);
        assert_eq!(back, older);
    }

    #[test]
    fn a_minecart_s_block_moves_to_11_and_12_and_its_shaking_goes() {
        let older = vec![
            (8, serializer::INT, Value::Int(5)),
            (9, serializer::INT, Value::Int(2)),
            (10, serializer::BOOLEAN, Value::Bool(true)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::MINECARTS[0], older);
        assert_eq!(
            newer,
            vec![
                (11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(5)),
                (12, serializer::INT, Value::Int(2)),
            ]
        );
        assert_eq!(
            back,
            vec![
                (8, serializer::INT, Value::Int(5)),
                (10, serializer::BOOLEAN, Value::Bool(true)),
                (9, serializer::INT, Value::Int(2)),
            ]
        );
    }

    #[test]
    fn a_hidden_minecart_block_is_not_sent() {
        let older = vec![
            (8, serializer::INT, Value::Int(5)),
            (9, serializer::INT, Value::Int(2)),
            (10, serializer::BOOLEAN, Value::Bool(false)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::MINECARTS[1], older);
        assert_eq!(newer, vec![(12, serializer::INT, Value::Int(2))]);
        assert_eq!(back, vec![(9, serializer::INT, Value::Int(2))]);
    }

    #[test]
    fn an_experience_orb_s_value_is_dropped_going_down() {
        let mut d = play::SetEntityData {
            id: 1,
            values: vec![(8, serializer::INT, Value::Int(4))],
        };
        entity_data(
            Some(translate::client_layout_v1_21_4::EXPERIENCE_ORB),
            &mut d,
        );
        assert!(d.values.is_empty());
    }

    #[test]
    fn a_piglin_gains_an_immunity_first_and_loses_it_again() {
        let older = vec![
            (16, serializer::BOOLEAN, Value::Bool(true)),
            (17, serializer::BOOLEAN, Value::Bool(false)),
            (18, serializer::BOOLEAN, Value::Bool(true)),
        ];
        let (newer, back) = up_then_down(translate::client_layout_v1_21_4::PIGLIN, older.clone());
        assert_eq!(
            newer,
            vec![
                (17, serializer::BOOLEAN, Value::Bool(true)),
                (18, serializer::BOOLEAN, Value::Bool(false)),
                (19, serializer::BOOLEAN, Value::Bool(true)),
            ]
        );
        assert_eq!(back, older);
    }

    #[test]
    fn the_variants_1_21_5_added_are_dropped_going_down() {
        let mut chicken = play::SetEntityData {
            id: 1,
            values: vec![(17, serializer::CHICKEN_VARIANT, Value::Id(3))],
        };
        entity_data(
            Some(translate::client_layout_v1_21_4::CHICKEN),
            &mut chicken,
        );
        assert!(chicken.values.is_empty());
        let mut wolf = play::SetEntityData {
            id: 1,
            values: vec![
                (22, serializer::WOLF_VARIANT, Value::Id(1)),
                (23, serializer::WOLF_SOUND_VARIANT, Value::Id(2)),
            ],
        };
        entity_data(Some(translate::client_layout_v1_21_4::WOLF), &mut wolf);
        assert_eq!(
            wolf.values,
            vec![(22, serializer::WOLF_VARIANT, Value::Id(1))]
        );
    }

    #[test]
    fn a_kind_of_no_change_passes_and_an_unknown_one_is_left_alone() {
        let values = vec![
            (0, serializer::BYTE, Value::Byte(1)),
            (9, serializer::FLOAT, Value::Float(2.0)),
        ];
        let mut d = play::SetEntityData {
            id: 1,
            values: values.clone(),
        };
        entity_data(Some(1), &mut d);
        assert_eq!(d.values, values);
        let mut unknown = play::SetEntityData {
            id: 1,
            values: values.clone(),
        };
        entity_data(None, &mut unknown);
        assert_eq!(unknown.values, values);
    }

    #[test]
    fn a_container_click_of_empty_slots_is_hashed_and_back() {
        let id = packet_ids::play::serverbound::CONTAINER_CLICK;
        let ids = ServerVersion::V1_21_4.ids().unwrap();
        let mut w = Writer::new();
        w.varint(1); // window
        w.varint(2); // state
        w.i16(36);
        w.i8(0);
        w.varint(0); // mode
        w.varint(1); // one changed slot
        w.i16(36);
        w.varint(0); // empty
        w.varint(0); // carried: empty
        let older = w.data;
        let Out::Rewritten(pinned) = serverbound(State::Play, id, &older, Some(ids)).unwrap()
        else {
            panic!("a container click is rewritten");
        };
        let back = translate::client_layout_v1_21_4::container_click(&pinned).unwrap();
        assert_eq!(back, older);
    }
}
