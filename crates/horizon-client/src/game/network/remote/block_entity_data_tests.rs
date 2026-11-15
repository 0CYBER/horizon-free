//! End-to-end client receive notices with independent tag values.
use super::*;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use mod_api::{BlockEntityPart, BlockPart, Hooks, Registrar};
use std::sync::{Arc, Mutex};

type Seen = Arc<
    Mutex<
        Vec<(
            mod_api::BlockEntityDataSource,
            [i32; 3],
            horizon_core::nbt::Compound,
            horizon_core::nbt::Compound,
        )>,
    >,
>;
struct Observer(Seen);
#[mod_api::horizon_part]
impl BlockPart for Observer {}
#[mod_api::horizon_part]
impl BlockEntityPart for Observer {
    fn block_entity_data_received(
        &self,
        host: &dyn mod_api::ClientHost,
        event: &mut mod_api::BlockEntityDataReceivedEvent<'_>,
    ) {
        self.0.lock().unwrap().push((
            event.source,
            event.entity.position(host).unwrap(),
            event.tag.clone(),
            event.entity.data(host).unwrap(),
        ));
    }
}
struct TestMod(Seen);
#[mod_api::__subscriptions]
impl Hooks for TestMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("counter").blocks(&[]).build();
        r.block_entity_part(&kind, Observer(self.0.clone()))
            .unwrap();
        r.block_entity_type("silent").blocks(&[]).build();
    }
}
#[test]
#[ignore = "daily"]
fn block_entity_data_received_after_merge() {
    if run_fresh_cases(
        "game::remote::block_entity_data_tests::block_entity_data_received_after_merge",
        "HORIZON_RECEIVED_CHILD",
        &["receive"],
    ) {
        return;
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "notice",
        Box::new(TestMod(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        horizon_client::block_entity_types::install(
            horizon_client::block_entity_types::BlockEntityTypes::new(
                [
                    ("minecraft:stone".into(), "notice:counter".into()),
                    ("minecraft:dirt".into(), "notice:silent".into()),
                ]
                .into(),
                vec!["notice:counter".into(), "notice:silent".into()],
            ),
        )
        .unwrap();
        with_client_game(|game| {
            let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Notice").unwrap();
            game.remote.as_mut().unwrap().logged_in = true;
            Arc::make_mut(&mut game.remote.as_mut().unwrap().builtin)
                .registries
                .insert(
                    "minecraft:block_entity_type".into(),
                    horizon_protocol::registry::Ids::from_names(vec![
                        "notice:counter".into(),
                        "notice:silent".into(),
                        "minecraft:chest".into(),
                    ]),
                );

            use mod_api::BlockEntityDataSource::{Chunk, UpdatePacket};
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let mut column = horizon_client::chunk::Chunk::empty(0, 0);
            column.set(1, 64, 1, stone.0);
            column.set(3, 64, 3, stone.0);
            let tag = |n| Tag::Compound([("count".into(), Tag::Int(n))].into());
            let mut packet = horizon_protocol::packets::play::LevelChunkWithLight {
                x: 0,
                z: 0,
                chunk: horizon_protocol::chunk::ChunkData {
                    heightmaps: Vec::new(),
                    buffer: Vec::new(),
                    block_entities: vec![horizon_protocol::chunk::BlockEntityInfo {
                        packed_xz: 0x11,
                        y: 64,
                        kind: 0,
                        tag: Tag::Compound(Default::default()),
                    }],
                },
                light: horizon_protocol::chunk::LightData {
                    sky_mask: Vec::new(),
                    block_mask: Vec::new(),
                    empty_sky_mask: Vec::new(),
                    empty_block_mask: Vec::new(),
                    sky: Vec::new(),
                    block: Vec::new(),
                },
            };
            let format = game.remote.as_ref().unwrap().chunk_format.clone();
            game.remote_chunk(&packet, Some((&format, Ok(column.clone()))));
            assert_eq!(seen.lock().unwrap().len(), 0, "chunk empty");
            packet.chunk.block_entities[0].tag = tag(3);
            game.remote_chunk(&packet, Some((&format, Ok(column.clone()))));
            assert_eq!(seen.lock().unwrap().len(), 1, "chunk content");
            assert_eq!(seen.lock().unwrap()[0].0, Chunk);
            seen.lock().unwrap().clear();
            game.remote_block_entity_data(
                [1, 64, 1],
                0,
                UpdatePacket,
                &Tag::Compound(Default::default()),
            );
            assert_eq!(seen.lock().unwrap().len(), 1, "packet empty");
            assert_eq!(seen.lock().unwrap()[0].0, UpdatePacket);
            assert!(seen.lock().unwrap()[0].2.is_empty());
            assert_eq!(
                seen.lock().unwrap()[0].3.get(&"count".into()),
                Some(&Tag::Int(3))
            );
            seen.lock().unwrap().clear();
            game.remote_block_entity_data([1, 64, 1], 0, UpdatePacket, &Tag::End);
            assert_eq!(seen.lock().unwrap().len(), 0, "packet absent");
            // Existing entity on an invalid holder must not receive or apply data.
            let dirt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dirt")
                .unwrap();
            game.world.set(1, 64, 1, dirt).unwrap();
            game.remote_block_entity_data([1, 64, 1], 0, UpdatePacket, &tag(9));
            assert_eq!(seen.lock().unwrap().len(), 0, "invalid holder");
            // Packet order is deliberately different from position order.
            packet.chunk.block_entities = vec![
                horizon_protocol::chunk::BlockEntityInfo {
                    packed_xz: 0x33,
                    y: 64,
                    kind: 0,
                    tag: tag(7),
                },
                horizon_protocol::chunk::BlockEntityInfo {
                    packed_xz: 0x11,
                    y: 64,
                    kind: 0,
                    tag: Tag::Compound(
                        [
                            ("count".into(), Tag::Int(3)),
                            ("name".into(), Tag::String("a".into())),
                        ]
                        .into(),
                    ),
                },
            ];
            game.remote_chunk(&packet, Some((&format, Ok(column))));
            {
                let records = seen.lock().unwrap();
                assert_eq!(records.len(), 2);
                assert_eq!(
                    [(records[0].0, records[0].1), (records[1].0, records[1].1)],
                    [(Chunk, [3, 64, 3]), (Chunk, [1, 64, 1])]
                );
            }
            seen.lock().unwrap().clear();
            game.remote_block_entity_data([1, 64, 1], 0, UpdatePacket, &tag(5));
            game.remote_block_entity_data([1, 64, 1], 0, UpdatePacket, &tag(5));
            let records = seen.lock().unwrap();
            assert_eq!(records.len(), 2, "identical packets each notify");
            for (source, position, arrived, merged) in records.iter() {
                assert_eq!(*source, UpdatePacket);
                assert_eq!(*position, [1, 64, 1]);
                assert_eq!(*arrived, [("count".into(), Tag::Int(5))].into());
                assert_eq!(merged.get(&"count".into()), Some(&Tag::Int(5)));
                assert_eq!(merged.get(&"name".into()), Some(&Tag::String("a".into())));
            }
            drop(records);
            seen.lock().unwrap().clear();
            game.world.set(2, 64, 2, dirt).unwrap();
            game.sync_block_entity((2, 64, 2), None, dirt);
            game.remote_block_entity_data([2, 64, 2], 1, UpdatePacket, &tag(7));
            game.remote_block_entity_data([4, 64, 4], 0, UpdatePacket, &tag(9));
            game.remote_block_entity_data([1, 64, 1], 1, UpdatePacket, &tag(11));
            let vanilla = Tag::Compound(
                [
                    ("id".into(), Tag::String("minecraft:chest".into())),
                    ("x".into(), Tag::Int(4)),
                    ("y".into(), Tag::Int(64)),
                    ("z".into(), Tag::Int(4)),
                ]
                .into(),
            );
            let (_, chest) = horizon_client::save::decode_block_entity(&vanilla).unwrap();
            game.world.block_entities.insert((4, 64, 4), chest);
            game.remote_block_entity_data([4, 64, 4], 2, UpdatePacket, &tag(13));
            assert_eq!(
                seen.lock().unwrap().len(),
                0,
                "silent, absent, mismatched, and vanilla entities"
            );
        });
    });
}
