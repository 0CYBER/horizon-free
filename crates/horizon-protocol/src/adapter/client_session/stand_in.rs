//! Mannequins (1.21.9) for a client through 1.21.8, which has no such entity: each is shown as a player
//! with the mannequin's skin, under the mannequin's entity id and UUID (so clicking it still reaches it).
//! A player entity needs a player info entry first: one is added (ADD_PLAYER only: not listed, no game mode)
//! with an empty name (no name tag) and the mannequin profile's properties (its skin), and removed with the
//! entity. The add waits for the mannequin's first entity data, which carries its profile; any other packet
//! first sends the waiting ones with no skin. Of the mannequin's entity data the player keeps what both have
//! (Entity, LivingEntity, Avatar: up to the skin layers, index 16); the profile, immovable and description go.
use std::collections::HashMap;

use crate::{
    entity_data::{Value, serializer},
    packets::{
        login::Property,
        play::{self, PlayerInfo},
    },
};

pub(super) use crate::adapter::pinned_ids::entity_type::{MANNEQUIN, PLAYER};

/// Avatar.DATA_PLAYER_MODE_CUSTOMISATION: the skin layers shown.
const SKIN_LAYERS: u8 = 16;

/// A connection's mannequins: their UUIDs, and the add of one not shown yet.
#[derive(Debug, Default)]
pub(super) struct Mannequins(pub(super) HashMap<i32, (u128, Option<play::AddEntity>)>);

/// The packets (pinned) that show the mannequin `add` as a player, with its first entity data.
pub(super) fn spawn(
    add: &play::AddEntity,
    data: Option<&play::SetEntityData>,
) -> Vec<play::Clientbound> {
    let properties = data
        .and_then(|d| {
            d.values.iter().find_map(|(_, _, v)| match v {
                Value::Profile(p) => Some(p.properties.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();
    let info = play::PlayerInfoUpdate {
        actions: 0x01,
        entries: vec![PlayerInfo {
            uuid: add.uuid,
            name: Some(String::new()),
            properties: properties.into_iter().collect::<Vec<Property>>(),
            ..PlayerInfo::default()
        }],
    };
    let mut player = add.clone();
    player.kind = PLAYER;
    let mut values = data.map(|d| kept(&d.values)).unwrap_or_default();
    // A player's skin layers default to none; a mannequin's to all: say so unless the data does.
    if !values.iter().any(|(i, _, _)| *i == SKIN_LAYERS) {
        values.push((SKIN_LAYERS, serializer::BYTE, Value::Byte(0x7F)));
    }
    vec![
        play::Clientbound::PlayerInfoUpdate(info),
        play::Clientbound::AddEntity(player),
        play::Clientbound::SetEntityData(play::SetEntityData { id: add.id, values }),
    ]
}

/// The entity data a player has of a mannequin's.
pub(super) fn kept(values: &[(u8, i32, Value)]) -> Vec<(u8, i32, Value)> {
    values
        .iter()
        .filter(|(i, _, _)| *i <= SKIN_LAYERS)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_data::Profile;

    #[test]
    fn a_mannequin_is_a_player_with_its_skin_and_no_mannequin_data() {
        let add = play::AddEntity {
            id: 7,
            uuid: 99,
            kind: MANNEQUIN,
            position: [1.0, 2.0, 3.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        };
        let skin = Property {
            name: "textures".into(),
            value: "abc".into(),
            signature: Some("sig".into()),
        };
        let data = play::SetEntityData {
            id: 7,
            values: vec![
                (0, serializer::BYTE, Value::Byte(0)),
                (15, serializer::HUMANOID_ARM, Value::Id(1)),
                (
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(Profile {
                        name: Some("Someone".into()),
                        uuid: Some(5),
                        properties: vec![skin.clone()],
                        ..Default::default()
                    }),
                ),
                (18, serializer::BOOLEAN, Value::Bool(true)),
            ],
        };
        let out = spawn(&add, Some(&data));
        let play::Clientbound::PlayerInfoUpdate(info) = &out[0] else {
            panic!()
        };
        assert_eq!(info.actions, 0x01);
        assert_eq!(info.entries[0].uuid, 99);
        assert_eq!(info.entries[0].name.as_deref(), Some(""));
        assert_eq!(info.entries[0].properties, vec![skin]);
        let play::Clientbound::AddEntity(player) = &out[1] else {
            panic!()
        };
        assert_eq!((player.id, player.uuid, player.kind), (7, 99, PLAYER));
        let play::Clientbound::SetEntityData(d) = &out[2] else {
            panic!()
        };
        let indexes: Vec<u8> = d.values.iter().map(|(i, _, _)| *i).collect();
        assert_eq!(indexes, [0, 15, 16]);
    }
}
