//! horizon-rust/horizon#17877: the main-hand choice as synched entity data.
//!
//! Vanilla's `Avatar.DATA_PLAYER_MAIN_HAND` is entity data index 15, a
//! `HumanoidArm` (serializer 42) whose default is RIGHT = 1 and whose LEFT is
//! 0 (`Avatar.java:38`, `HumanoidArm.java:13-14`). Its dirty entries go to the
//! player itself as well as to every tracker (`ChunkMap.java:1352-1356`), and a
//! change back to the default is a dirty entry too (`SynchedEntityData.java:60-68`,
//! `:74-90`). Pairing sends the non-default values only (`ServerEntity.java:311-312`),
//! so a right-handed player gets nothing until they change from LEFT.
use super::*;

/// The index of a `HumanoidArm` value in `EntityDataSerializers`.
const HUMANOID_ARM: i32 = 42;

/// Every `SetEntityData` entry of index 15 sent to the receiver, as
/// (entity, index, serializer, value).
fn main_hand_frames(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(i32, u8, i32, Value)> {
    let mut frames = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let C::SetEntityData(p) = C::decode(id, r.rest()).unwrap() {
            frames.extend(
                p.values
                    .into_iter()
                    .filter(|(index, _, _)| *index == 15)
                    .map(|(index, serializer, value)| (p.id, index, serializer, value)),
            );
        }
    }
    frames
}

/// The guest's own client: the hand its own model draws the item in.
#[test]
fn the_main_hand_reaches_the_players_own_client_on_a_change() {
    let (mut hub, rx) = hub_with_a_guest();
    // `ClientInformation::new` asks for RIGHT = 1, the default: nothing to send.
    hub.sync_player_state(0);
    assert_eq!(main_hand_frames(&rx), [], "the default hand is not sent");
    // LEFT = 0 is not the default: one frame, to the player's own entity.
    hub.guests.update(0, |g| g.info.main_hand = 0);
    hub.sync_player_state(0);
    assert_eq!(
        main_hand_frames(&rx),
        [(1, 15, HUMANOID_ARM, Value::Id(0))],
        "the left hand goes to the player itself"
    );
    hub.sync_player_state(0);
    assert_eq!(main_hand_frames(&rx), [], "the same hand is not sent again");
    // Back to the right hand: the change itself is a dirty entry.
    hub.guests.update(0, |g| g.info.main_hand = 1);
    hub.sync_player_state(0);
    assert_eq!(
        main_hand_frames(&rx),
        [(1, 15, HUMANOID_ARM, Value::Id(1))],
        "the hand changed back to RIGHT"
    );
    hub.sync_player_state(0);
    assert_eq!(main_hand_frames(&rx), []);
    // HumanoidArm's id mapper reads any other value as LEFT, which is 0
    // (HumanoidArm.java:17): the raw 7 goes out as one 0, and not again,
    // although the raw value never equals the RIGHT the field starts at.
    hub.guests.update(0, |g| g.info.main_hand = 7);
    hub.sync_player_state(0);
    assert_eq!(
        main_hand_frames(&rx),
        [(1, 15, HUMANOID_ARM, Value::Id(0))],
        "an out-of-range hand is LEFT"
    );
    hub.sync_player_state(0);
    assert_eq!(
        main_hand_frames(&rx),
        [],
        "the normalised hand is not repeated"
    );
}

/// PlayerList.respawn makes a new ServerPlayer, whose tracker sends its
/// synched data afresh: a left-handed player's hand goes out again.
#[test]
fn a_left_handed_player_is_sent_its_hand_again_after_a_respawn() {
    let (mut hub, rx) = hub_with_a_guest();
    hub.guests.update(0, |g| g.info.main_hand = 0);
    hub.sync_player_state(0);
    assert_eq!(main_hand_frames(&rx), [(1, 15, HUMANOID_ARM, Value::Id(0))]);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    hub.respawn(0, [0.5, 64.0, 0.5], Some([0.0; 2]), &ctx);
    assert_eq!(main_hand_frames(&rx), [], "the respawn alone sends no hand");
    hub.sync_player_state(0);
    assert_eq!(
        main_hand_frames(&rx),
        [(1, 15, HUMANOID_ARM, Value::Id(0))],
        "the new player's tracker sends the non-default hand again"
    );
    hub.sync_player_state(0);
    assert_eq!(main_hand_frames(&rx), []);
}

/// Other players' model draws the item in the hand the player chose
/// (`ArmedEntityRenderState.mainArm`), which their clients cannot guess.
#[test]
fn other_players_are_told_a_left_hand_but_never_the_default_one() {
    let (hub, _rx) = hub_with_a_guest();
    let state = |main_hand: i32| {
        hub.player_state(
            1,
            1,
            [0.5, 64.0, 0.5],
            (0.0, 0.0, 0.0),
            true,
            (false, false, false),
            BodyFlags::default(),
            0,
            None,
            0,
            main_hand,
            20.0,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    };
    let hand = |entries: Vec<(u8, i32, Value)>| {
        entries
            .into_iter()
            .filter(|(index, _, _)| *index == 15)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        hand(state(0).meta),
        [(15, HUMANOID_ARM, Value::Id(0))],
        "a left-handed player is drawn left-handed by the others"
    );
    assert_eq!(
        hand(state(1).meta),
        [],
        "the default RIGHT is left out, as vanilla's data does"
    );
    assert_eq!(
        hand(state(7).meta),
        [(15, HUMANOID_ARM, Value::Id(0))],
        "an out-of-range hand is LEFT here too (HumanoidArm.java:17)"
    );
    // The state leaves the default out, so the tracker needs the default
    // itself to send a client that holds LEFT the change back to RIGHT
    // (Avatar.java:14: DATA_PLAYER_MAIN_HAND starts at RIGHT).
    assert_eq!(
        hand(state(1).defaults),
        [(15, HUMANOID_ARM, Value::Id(1))],
        "the tracker can put the hand back to its default"
    );
}

/// The tracker itself (entity.rs send_dirty): another player's client is
/// paired with the left hand and is sent the RIGHT it changes back to, which
/// the state no longer carries.
#[test]
fn another_players_client_is_sent_the_hand_going_back_to_right() {
    let (mut hub, _own) = hub_with_a_guest();
    // Bob (entity 1) is left-handed before anybody is paired with him.
    hub.guests.update(0, |g| {
        g.info.main_hand = 0;
        g.sent_chunks.insert((0, 0));
    });
    // Ada (entity 2) watches him.
    let (tx, watching) = std::sync::mpsc::channel();
    let mut ada = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Ada".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [0.5, 64.0, 0.5],
        8,
    );
    ada.placed = true;
    ada.dimension = "minecraft:overworld".into();
    ada.sent_chunks.insert((0, 0));
    hub.guests.push(ada);
    // The test hub syncs no entity type registry.
    hub.entity_types.insert("minecraft:player".into(), 1);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 3;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let bob = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
        main_hand_frames(rx)
            .into_iter()
            .filter(|frame| frame.0 == 1)
            .collect::<Vec<_>>()
    };
    hub.track_entities(&mut ctx);
    assert_eq!(
        bob(&watching),
        [(1, 15, HUMANOID_ARM, Value::Id(0))],
        "the pairing sends the left hand he had before it"
    );
    hub.guests.update(0, |g| g.info.main_hand = 1);
    hub.track_entities(&mut ctx);
    assert_eq!(
        bob(&watching),
        [(1, 15, HUMANOID_ARM, Value::Id(1))],
        "the change back to RIGHT reaches the other player's client"
    );
    hub.track_entities(&mut ctx);
    assert_eq!(bob(&watching), [], "and it is not sent again");
}
