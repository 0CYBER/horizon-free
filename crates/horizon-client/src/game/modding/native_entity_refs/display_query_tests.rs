use super::tests::{with_client_game, with_game};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};
use horizon_protocol::{
    connection::Pipe,
    entity_data::{
        Value,
        serializer::{FLOAT, INT},
    },
    packets::play::{
        AddEntity, Clientbound as C, EntityPositionSync, PositionPath, RemoveEntities,
        SetEntityData,
    },
};
use mod_api::{CrosshairTarget, DisplayKind, Host, LevelRef};
use std::sync::Arc;

fn query_ids(host: &dyn Host, level: LevelRef, min: [f64; 3], max: [f64; 3]) -> Vec<u32> {
    level
        .entities_in(host, min, max)
        .unwrap()
        .into_iter()
        .map(|entity| entity.id())
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_client_queries_tracked_display_entities_without_picking_them() {
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "DisplayQueryTest").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        let registries = remote.builtin.clone();
        let kinds = registries.get("minecraft:entity_type").unwrap();
        game.title = None;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 151.0, 0.5)).unwrap());
        game.yaw = 0.0;
        game.pitch = 0.0;
        game.entity_reach = 6.0;

        let add = |id, kind: &str, position| {
            C::AddEntity(AddEntity {
                id,
                uuid: id as u128,
                kind: kinds.id(kind).unwrap(),
                position,
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
        };
        let displays = [
            (22, DisplayKind::Text, [1.5, 152.0, 4.0]),
            (21, DisplayKind::Block, [0.5, 152.0, 4.0]),
            (23, DisplayKind::Item, [10.0, 152.0, 4.0]),
        ];
        for (id, kind, position) in displays {
            game.handle_packet(add(id, kind.entity_type(), position))
                .unwrap();
        }

        let level = game.level("minecraft:overworld").unwrap();
        let min = [0.0, 151.5, 0.0];
        let max = [4.0, 153.1, 5.0];
        assert_eq!(query_ids(game, level, min, max), [21, 22, 7]);
        assert_eq!(
            query_ids(game, level, [9.5, 151.5, 3.5], [10.5, 153.1, 4.5]),
            [23]
        );
        for (id, kind, position) in displays {
            let entity = game.entity(id as u32).unwrap();
            assert_eq!(entity.position(game), Some(position));
            assert_eq!(entity.kind(game).unwrap().id(game), kind.entity_type());
            assert_eq!(entity.bounding_box(game), Some([position; 2]));
        }

        game.handle_packet(C::SetEntityData(SetEntityData {
            id: 21,
            values: vec![
                (10, INT, Value::Int(2)),
                (20, FLOAT, Value::Float(2.0)),
                (21, FLOAT, Value::Float(2.0)),
            ],
        }))
        .unwrap();
        let block = game.entity(21).unwrap();
        assert_eq!(block.bounding_box(game), Some([[0.5, 152.0, 4.0]; 2]));
        assert_eq!(query_ids(game, level, min, max), [21, 22, 7]);
        // The block display's culling bounds overlap these boxes, but its point
        // is only on the face of the first and outside the second.
        for x in [0.5, 0.75] {
            assert!(query_ids(game, level, [x, 151.5, 3.5], [1.0, 153.1, 4.5]).is_empty());
        }

        game.handle_packet(add(24, "minecraft:pig", [0.5, 152.0, 5.5]))
            .unwrap();
        assert!(matches!(
            game.crosshair_target(),
            Some(CrosshairTarget::Entity { entity, .. }) if entity.id() == 24
        ));

        game.handle_packet(C::EntityPositionSync(EntityPositionSync {
            id: 21,
            path: PositionPath::Linear([10.5, 152.0, 4.0]),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        }))
        .unwrap();
        let assert_block_position = |host: &dyn Host, x: f64| {
            let min = [x - 0.25, 151.5, 3.5];
            let max = [x + 0.25, 153.1, 4.5];
            assert_eq!(query_ids(host, level, min, max), [21]);
            assert_eq!(block.position(host), Some([x, 152.0, 4.0]));
            assert_eq!(block.bounding_box(host), Some([[x, 152.0, 4.0]; 2]));
        };
        assert_block_position(game, 0.5);
        assert!(query_ids(game, level, [10.25, 151.5, 3.5], [10.75, 153.1, 4.5]).is_empty());
        game.tick_text_displays();
        assert_block_position(game, 5.5);
        game.tick_text_displays();
        assert_block_position(game, 10.5);

        game.handle_packet(C::RemoveEntities(RemoveEntities { ids: vec![21] }))
            .unwrap();
        assert!(query_ids(game, level, [10.25, 151.5, 3.5], [10.75, 153.1, 4.5]).is_empty());
        assert_eq!(query_ids(game, level, min, max), [22, 7]);
        assert_eq!(block.position(game), None);
        assert_eq!(block.bounding_box(game), None);
    });
}

#[test]
#[ignore = "daily"]
fn a_server_queries_display_entity_points_until_discarded() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let dimension = game.dim.id();
        let level = game.level(dimension).unwrap();
        let position = [0.5, 152.0, 4.0];
        let nbt = Compound::from([
            ("width".into(), Tag::Float(2.0_f32.to_bits())),
            ("height".into(), Tag::Float(2.0_f32.to_bits())),
        ]);
        let id = game
            .summon(dimension, DisplayKind::Block.entity_type(), position, nbt)
            .unwrap();
        let marker = game
            .summon(dimension, "minecraft:marker", position, Compound::new())
            .unwrap();
        assert!(game.entity(marker).is_some());

        let min = [0.0, 151.5, 3.5];
        let max = [1.0, 153.1, 4.5];
        assert_eq!(query_ids(game, level, min, max), [id]);
        let display = game.entity(id).unwrap();
        assert_eq!(display.bounding_box(game), Some([position; 2]));
        for x in [0.5, 0.75] {
            assert!(query_ids(game, level, [x, min[1], min[2]], max).is_empty());
        }
        assert!(display.discard(game));
        assert!(query_ids(game, level, min, max).is_empty());
        assert_eq!(display.bounding_box(game), None);
    });
}
