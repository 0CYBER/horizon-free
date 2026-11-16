//! Lookup and save order of the entities a chunk loads with, across storage
//! families. `tools/test_chunk_entity_order_parity.py` compares the printed
//! sequences with the real `PersistentEntitySectionManager`.
use super::*;
use crate::game::native_entity_refs::tests::with_game;

fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(
        values
            .into_iter()
            .map(|v| Tag::Double(v.to_bits()))
            .collect(),
    )
}

/// A saved record named by the last int of its UUID.
fn saved(kind: &str, n: i32, at: [f64; 3], extra: Vec<(&str, Tag)>) -> Tag {
    let mut fields = vec![
        ("id", Tag::String(kind.into())),
        ("UUID", Tag::IntArray(vec![2703, 0, 0, n])),
        ("Pos", doubles(at)),
    ];
    fields.extend(extra);
    Tag::compound(fields)
}

fn name(tag: &Tag) -> i32 {
    entity_nbt::uuid(tag).unwrap()[3]
}

/// Every lookup member's name, in lookup order.
fn lookup_order(game: &mut Game<'_>) -> Vec<i32> {
    let level = game.level(game.dim.id()).unwrap();
    let members = game.level_all_entities(level).unwrap();
    members
        .into_iter()
        .filter_map(|e| game.persist.uuids.get(&e.id()).copied())
        .filter(|uuid| uuid[0] == 2703)
        .map(|uuid| uuid[3])
        .collect()
}

fn saved_order(chunks: &[(ChunkKey, Vec<Tag>)]) -> Vec<Vec<i32>> {
    chunks
        .iter()
        .map(|(key, tags)| {
            let mut line = vec![key.0, key.1];
            line.extend(tags.iter().map(name));
            println!("saved-order: {line:?}");
            line
        })
        .collect()
}

fn snapshot(game: &mut Game<'_>) -> Vec<i32> {
    let order = lookup_order(game);
    println!("load-order: {order:?}");
    order
}

#[test]
#[ignore = "daily"]
fn chunk_loaded_families_keep_vanilla_lookup_and_save_order() {
    with_game(|game| {
        let a = (0, 0);
        let b = (1, 0);
        for key in [a, b] {
            game.world
                .chunks
                .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        }
        let item = Tag::compound([
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(1)),
        ]);
        let sand = Tag::compound([("Name", Tag::String("minecraft:sand".into()))]);
        // Chunk (0, 0) as vanilla stored it: one root per family, in sections
        // 4, 0 and -2 (the item and the armor stand), a pig riding the boat.
        let first = vec![
            saved("minecraft:tnt", 1, [2.5, 70.0, 2.5], vec![]),
            saved("minecraft:pig", 2, [3.5, 70.0, 3.5], vec![]),
            saved("minecraft:arrow", 3, [4.5, 70.0, 4.5], vec![]),
            saved(
                "minecraft:oak_boat",
                4,
                [5.5, 70.0, 5.5],
                vec![(
                    "Passengers",
                    Tag::List(vec![saved("minecraft:pig", 5, [5.5, 70.0, 5.5], vec![])]),
                )],
            ),
            saved("minecraft:item", 6, [6.5, 10.0, 6.5], vec![("Item", item)]),
            saved(
                "minecraft:falling_block",
                7,
                [7.5, 70.0, 7.5],
                vec![("BlockState", sand)],
            ),
            saved("minecraft:armor_stand", 8, [8.5, -20.0, 8.5], vec![]),
        ];
        let second = vec![
            saved("minecraft:minecart", 9, [18.5, 70.0, 2.5], vec![]),
            saved("minecraft:zombie", 10, [19.5, 70.0, 2.5], vec![]),
            saved("minecraft:end_crystal", 13, [22.5, 70.0, 2.5], vec![]),
            saved(
                "minecraft:experience_orb",
                11,
                [20.5, 70.0, 2.5],
                vec![("Value", Tag::Short(3)), ("Count", Tag::Int(1))],
            ),
            saved("minecraft:snowball", 12, [21.5, 70.0, 2.5], vec![]),
        ];
        game.world.queue_entity_nbt(a, first);
        game.world.queue_entity_nbt(b, second);
        game.spawn_loaded_entities();
        let mut lookup = vec![snapshot(game)];
        // The chunk unloads with what it stores, then loads that again.
        let unloaded = game.entity_chunks(&HashSet::from([a]), true);
        let unload = saved_order(&unloaded);
        lookup.push(snapshot(game));
        for (key, tags) in unloaded {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        lookup.push(snapshot(game));
        // The pig leaves through a portal and comes back.
        let pig = game.mob_by_uuid([2703, 0, 0, 2]).unwrap();
        let (tag, state) = game.take_vehicle_tree_transfer(pig).unwrap();
        game.remove_vehicle_tree(pig);
        lookup.push(snapshot(game));
        game.restore_vehicle_tree_state(tag, state).unwrap();
        lookup.push(snapshot(game));
        let mut chunks = game.entity_chunks(&HashSet::from([a, b]), false);
        chunks.sort_by_key(|(key, _)| *key);
        let save = saved_order(&chunks);
        // VanillaChunkEntityOrderOracle's sequences for the same records.
        assert_eq!(
            lookup,
            vec![
                vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 11, 12],
                vec![9, 10, 13, 11, 12],
                vec![9, 10, 13, 11, 12, 6, 1, 2, 3, 4, 5, 7, 8],
                vec![9, 10, 13, 11, 12, 6, 1, 3, 4, 5, 7, 8],
                vec![9, 10, 13, 11, 12, 6, 1, 3, 4, 5, 7, 8, 2],
            ]
        );
        assert_eq!(unload, vec![vec![0, 0, 6, 1, 2, 3, 4, 7, 8]]);
        assert_eq!(
            save,
            vec![
                vec![0, 0, 6, 1, 3, 4, 7, 2, 8],
                vec![1, 0, 9, 10, 13, 11, 12]
            ]
        );
    });
}
