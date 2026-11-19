use super::*;

/// A level whose block parts answer the piston structure resolver's questions.
struct StickyLevel {
    inner: TestLevel,
    /// The states a part makes slime blocks (`adjust_is_slime_block`).
    slime: HashSet<BlockStateId>,
    /// The states a part makes sticky (`adjust_is_sticky_block`).
    sticky: HashSet<BlockStateId>,
    /// The (state, other) pairs a part of `state` refuses to stick to (`adjust_can_stick_to`).
    refused: HashSet<(BlockStateId, BlockStateId)>,
    updates: Vec<BlockPos>,
}
impl crate::level::BlockEntityNeighbors for StickyLevel {}
impl LevelAccess for StickyLevel {
    fn is_client_side(&self) -> bool {
        self.inner.client_side
    }
    fn send_block_updated(&mut self, pos: BlockPos) {
        self.updates.push(pos);
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.inner.block_state(pos)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        self.inner.set_raw(pos, state)
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.inner.is_loaded(pos)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        self.inner.random()
    }
    fn game_time(&self) -> i64 {
        self.inner.game_time()
    }
    fn adjust_is_slime_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        vanilla || self.slime.contains(&state)
    }
    fn adjust_is_sticky_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        vanilla || self.sticky.contains(&state)
    }
    fn adjust_can_stick_to(&self, state: BlockStateId, other: BlockStateId, vanilla: bool) -> bool {
        vanilla && !self.refused.contains(&(state, other))
    }
}

fn state(f: &Fixture, name: &str) -> BlockStateId {
    f.registry
        .default_state(&format!("minecraft:{name}"))
        .unwrap_or_else(|| panic!("unknown block {name}"))
}

/// A piston at the origin facing east is powered from the west, with `pushed` east of it and
/// `above` on top of that, in a level whose parts `parts` sets up: the states at `(1, 2, 0)` and
/// `(2, 2, 0)` once it extended, where `above` was and where it goes if it moved along.
fn push_pair(
    f: &Fixture,
    pushed: BlockStateId,
    above: BlockStateId,
    parts: impl FnOnce(&mut StickyLevel),
) -> (BlockStateId, BlockStateId) {
    let mut level = StickyLevel {
        inner: TestLevel::new(&f.behaviours),
        slime: HashSet::new(),
        sticky: HashSet::new(),
        refused: HashSet::new(),
        updates: Vec::new(),
    };
    parts(&mut level);
    let mut ticks = Ticks::new(0);
    for x in -4..4 {
        for z in -4..4 {
            ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
        }
    }
    let stone = state(f, "stone");
    for x in -3..=3 {
        for z in -3..=3 {
            level.inner.blocks.insert(BlockPos::new(x, 0, z), stone);
        }
    }
    let piston = f
        .registry
        .resolve(&BlockState {
            name: "minecraft:piston".into(),
            properties: [("facing".to_owned(), "east".to_owned())].into(),
        })
        .unwrap();
    let redstone = state(f, "redstone_block");
    for (pos, block) in [
        (BlockPos::new(0, 1, 0), piston),
        (BlockPos::new(1, 1, 0), pushed),
        (BlockPos::new(1, 2, 0), above),
        (BlockPos::new(-1, 1, 0), redstone),
    ] {
        f.behaviours
            .set_block(&mut level, &mut ticks, pos, block, flags::UPDATE_ALL);
    }
    for _ in 0..4 {
        level.inner.time += 1;
        f.behaviours.tick_level_blocks(&mut level, &mut ticks);
        f.behaviours.tick_block_entities(&mut level, &mut ticks);
    }
    let extended = f.registry.state(level.block_state(BlockPos::new(0, 1, 0)));
    assert_eq!(
        extended
            .and_then(|s| s.properties.get("extended"))
            .map(String::as_str),
        Some("true"),
        "the piston extended"
    );
    assert_eq!(level.block_state(BlockPos::new(2, 1, 0)), pushed);
    (
        level.block_state(BlockPos::new(1, 2, 0)),
        level.block_state(BlockPos::new(2, 2, 0)),
    )
}

#[test]
fn vanilla_slime_pulls_the_block_on_top_and_stone_does_not() {
    let Some(f) = fixture() else {
        return;
    };
    let (slime, stone, air) = (
        state(f, "slime_block"),
        state(f, "stone"),
        f.behaviours.air(),
    );
    assert_eq!(push_pair(f, slime, stone, |_| {}), (air, stone));
    let gold = state(f, "gold_block");
    assert_eq!(push_pair(f, gold, stone, |_| {}), (stone, air));
}

#[test]
fn a_sticky_part_block_pulls_its_neighbours_along_with_a_piston() {
    let Some(f) = fixture() else {
        return;
    };
    // Gold is an ordinary block until a part makes it sticky: then the stone on top moves too.
    let (gold, stone, air) = (
        state(f, "gold_block"),
        state(f, "stone"),
        f.behaviours.air(),
    );
    let sticky = |level: &mut StickyLevel| {
        level.sticky.insert(gold);
    };
    assert_eq!(push_pair(f, gold, stone, sticky), (air, stone));
}

#[test]
fn a_sticky_part_block_lets_go_of_what_can_stick_to_says_no_to() {
    let Some(f) = fixture() else {
        return;
    };
    // Honey on a sticky gold block moves with it, as two sticky blocks hold on to each other,
    // unless either side refuses: the resolver asks both.
    let (gold, honey, air) = (
        state(f, "gold_block"),
        state(f, "honey_block"),
        f.behaviours.air(),
    );
    for (refused, expected) in [
        (None, (air, honey)),
        (Some((gold, honey)), (honey, air)),
        (Some((honey, gold)), (honey, air)),
    ] {
        let parts = |level: &mut StickyLevel| {
            level.sticky.insert(gold);
            level.refused.extend(refused);
        };
        assert_eq!(
            push_pair(f, gold, honey, parts),
            expected,
            "refused {refused:?}"
        );
    }
}

#[test]
fn a_slime_part_block_is_held_apart_from_honey_as_slime_is() {
    let Some(f) = fixture() else {
        return;
    };
    // Honey on a sticky gold block moves with it, unless a part also says gold is slime.
    let (gold, honey, air) = (
        state(f, "gold_block"),
        state(f, "honey_block"),
        f.behaviours.air(),
    );
    for (slime, expected) in [(false, (air, honey)), (true, (honey, air))] {
        let parts = |level: &mut StickyLevel| {
            level.sticky.insert(gold);
            if slime {
                level.slime.insert(gold);
            }
        };
        assert_eq!(
            push_pair(f, gold, honey, parts),
            expected,
            "gold is slime: {slime}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn refused_adhesion_next_tick_updates_replace_client_replay_before_landing() {
    let f = fixture().expect("prepared resources");
    let slime = state(f, "slime_block");
    let stone = state(f, "stone");
    let air = f.behaviours.air();
    for (refuse, sticky_stone) in [(false, false), (true, false), (false, true)] {
        let make = || StickyLevel {
            inner: TestLevel::new(&f.behaviours),
            slime: HashSet::new(),
            sticky: HashSet::new(),
            refused: HashSet::new(),
            updates: Vec::new(),
        };
        let mut server = make();
        let mut client = make();
        client.inner.client_side = true;
        if refuse {
            server.refused.insert((slime, stone));
        }
        if sticky_stone {
            server.sticky.insert(stone);
        }
        let pushed = if sticky_stone { stone } else { slime };
        let piston = f
            .registry
            .resolve(&BlockState {
                name: "minecraft:sticky_piston".into(),
                properties: [("facing".to_owned(), "east".to_owned())].into(),
            })
            .unwrap();
        for level in [&mut server, &mut client] {
            for (pos, block) in [
                (BlockPos::new(0, 1, 0), piston),
                (BlockPos::new(1, 1, 0), pushed),
                (BlockPos::new(1, 2, 0), stone),
                (BlockPos::new(-1, 1, 0), state(f, "redstone_block")),
            ] {
                level.inner.blocks.insert(pos, block);
            }
        }
        let mut server_ticks = Ticks::new(0);
        let mut client_ticks = Ticks::new(0);
        // Execute the server event, then replay its event on a client with no part.
        assert!(f.behaviours.client_block_event(
            &mut server,
            &mut server_ticks,
            BlockPos::new(0, 1, 0),
            0,
            Direction::East as i32
        ));
        assert!(f.behaviours.client_block_event(
            &mut client,
            &mut client_ticks,
            BlockPos::new(0, 1, 0),
            0,
            Direction::East as i32
        ));
        assert_eq!(
            client_ticks
                .moving_pistons
                .iter()
                .any(|(p, _)| *p == BlockPos::new(2, 2, 0)),
            !sticky_stone,
        );
        for level in [&mut server, &mut client] {
            level.inner.time += 1;
        }
        f.behaviours
            .tick_block_entities(&mut server, &mut server_ticks);
        f.behaviours
            .client_tick_block_entities(&mut client, &mut client_ticks);
        if refuse {
            assert!(server.updates.contains(&BlockPos::new(1, 2, 0)));
            assert!(server.updates.contains(&BlockPos::new(2, 2, 0)));
        }
        // Next tick's ordinary block packets replace both erroneous replay cells.
        for pos in server.updates.iter().copied() {
            f.behaviours.set_block(
                &mut client,
                &mut client_ticks,
                pos,
                server.block_state(pos),
                flags::UPDATE_CLIENTS,
            );
        }
        server.updates.clear();
        for _ in 0..10 {
            // Continue ordinary next-tick delivery, including the server's landing
            // updates for blocks that the client's vanilla replay never moved.
            for pos in std::mem::take(&mut server.updates) {
                f.behaviours.set_block(
                    &mut client,
                    &mut client_ticks,
                    pos,
                    server.block_state(pos),
                    flags::UPDATE_CLIENTS,
                );
            }
            server.inner.time += 1;
            client.inner.time += 1;
            f.behaviours
                .tick_block_entities(&mut server, &mut server_ticks);
            f.behaviours
                .client_tick_block_entities(&mut client, &mut client_ticks);
        }
        let expected = if refuse { (stone, air) } else { (air, stone) };
        assert_eq!(server.block_state(BlockPos::new(1, 2, 0)), expected.0);
        assert_eq!(server.block_state(BlockPos::new(2, 2, 0)), expected.1);
        for pos in [
            BlockPos::new(0, 1, 0),
            BlockPos::new(1, 1, 0),
            BlockPos::new(2, 1, 0),
            BlockPos::new(1, 2, 0),
            BlockPos::new(2, 2, 0),
        ] {
            assert_eq!(client.block_state(pos), server.block_state(pos), "{pos:?}");
        }
    }
}

#[test]
#[ignore = "daily"]
fn failed_part_retraction_corrects_successful_vanilla_replay() {
    let f = fixture().expect("prepared resources");
    let stone = state(f, "stone");
    let air = f.behaviours.air();
    let make = || StickyLevel {
        inner: TestLevel::new(&f.behaviours),
        slime: HashSet::new(),
        sticky: HashSet::new(),
        refused: HashSet::new(),
        updates: Vec::new(),
    };
    let mut server = make();
    let mut client = make();
    server.sticky.insert(stone);
    client.inner.client_side = true;
    let piston = f
        .registry
        .resolve(&BlockState {
            name: "minecraft:sticky_piston".into(),
            properties: [
                ("facing".to_owned(), "east".to_owned()),
                ("extended".to_owned(), "true".to_owned()),
            ]
            .into(),
        })
        .unwrap();
    let head = f
        .registry
        .resolve(&BlockState {
            name: "minecraft:piston_head".into(),
            properties: [
                ("facing".to_owned(), "east".to_owned()),
                ("type".to_owned(), "sticky".to_owned()),
            ]
            .into(),
        })
        .unwrap();
    // Thirteen stones exceed the server's push limit only when parts make them sticky.
    // Vanilla pulls just the bottom stone west, from (2, 1, 0) to (1, 1, 0).
    let stones = [
        BlockPos::new(2, 1, 0),
        BlockPos::new(2, 2, 0),
        BlockPos::new(2, 3, 0),
        BlockPos::new(2, 4, 0),
        BlockPos::new(2, 5, 0),
        BlockPos::new(2, 6, 0),
        BlockPos::new(2, 7, 0),
        BlockPos::new(2, 8, 0),
        BlockPos::new(2, 9, 0),
        BlockPos::new(2, 10, 0),
        BlockPos::new(2, 11, 0),
        BlockPos::new(2, 12, 0),
        BlockPos::new(2, 13, 0),
    ];
    for level in [&mut server, &mut client] {
        level.inner.blocks.insert(BlockPos::new(0, 1, 0), piston);
        level.inner.blocks.insert(BlockPos::new(1, 1, 0), head);
        for pos in stones {
            level.inner.blocks.insert(pos, stone);
        }
    }
    let mut server_ticks = Ticks::new(0);
    let mut client_ticks = Ticks::new(0);
    assert!(f.behaviours.client_block_event(
        &mut server,
        &mut server_ticks,
        BlockPos::new(0, 1, 0),
        1,
        Direction::East as i32
    ));
    assert!(f.behaviours.client_block_event(
        &mut client,
        &mut client_ticks,
        BlockPos::new(0, 1, 0),
        1,
        Direction::East as i32
    ));
    assert_eq!(server.block_state(BlockPos::new(2, 1, 0)), stone);
    assert_eq!(client.block_state(BlockPos::new(2, 1, 0)), air);
    assert!(
        client_ticks
            .moving_pistons
            .iter()
            .any(|(p, _)| *p == BlockPos::new(1, 1, 0))
    );
    assert!(server.updates.contains(&BlockPos::new(2, 1, 0)));
    assert!(server.updates.contains(&BlockPos::new(1, 1, 0)));
    for _ in 0..10 {
        server.inner.time += 1;
        client.inner.time += 1;
        // Apply the preceding tick's ordinary updates before the replay can land.
        for pos in std::mem::take(&mut server.updates) {
            f.behaviours.set_block(
                &mut client,
                &mut client_ticks,
                pos,
                server.block_state(pos),
                flags::UPDATE_CLIENTS,
            );
        }
        f.behaviours
            .tick_block_entities(&mut server, &mut server_ticks);
        f.behaviours
            .client_tick_block_entities(&mut client, &mut client_ticks);
    }
    assert_eq!(server.block_state(BlockPos::new(1, 1, 0)), air);
    assert_eq!(client.block_state(BlockPos::new(1, 1, 0)), air);
    for pos in stones {
        assert_eq!(server.block_state(pos), stone, "server {pos:?}");
        assert_eq!(client.block_state(pos), stone, "client {pos:?}");
    }
    assert_eq!(
        client.block_state(BlockPos::new(0, 1, 0)),
        server.block_state(BlockPos::new(0, 1, 0))
    );
}
