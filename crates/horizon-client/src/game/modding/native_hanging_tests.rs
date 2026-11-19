//! `entity_set_hanging_direction` and `entity_set_item_frame_rotation` on a server: a frame and a
//! painting turn and their bounds follow, a painting refuses up and down, a frame's item turns
//! with vanilla's wrap, and another kind is refused.
use super::native_entity_refs::tests::with_game;
use horizon_client::entities::{ItemFrame, Painting};
use horizon_core::geometry::Direction;

const FRAME: u32 = 900;
const PAINTING: u32 = 901;

/// A server's game with a south-facing item frame and painting hanging at 4, 70, 4 and 4, 70, 6.
fn with_hanging(test: impl FnOnce(&mut super::Game<'_>)) {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        with_hanging_in(game);
        test(game);
    });
}

fn with_hanging_in(game: &mut super::Game<'_>) {
    game.item_frames.push(ItemFrame {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id: FRAME,
        pos: [4, 70, 4],
        facing: 3,
        glow: false,
        item: None,
        rotation: 0,
        drop_chance: 1.0,
        fixed: false,
        invisible: false,
        custom_name: None,
        random: horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(0)),
        check_interval: 0,
    });
    game.paintings.push(Painting {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id: PAINTING,
        pos: [4, 70, 6],
        facing: 0,
        variant: "minecraft:kebab".into(),
        width: 1,
        height: 1,
        check_interval: 0,
    });
    game.next_entity = game.next_entity.max(PAINTING);
}

/// Which axes of a box are thin (a hanging entity's depth).
fn thin((lo, hi): ([f64; 3], [f64; 3])) -> [bool; 3] {
    std::array::from_fn(|axis| hi[axis] - lo[axis] < 0.1)
}

#[test]
#[ignore = "daily"]
fn a_frame_turns_to_every_direction_and_its_bounds_follow() {
    with_hanging(|game| {
        let frame = game.entity(FRAME).expect("the frame's handle");
        assert_eq!(thin(game.item_frames[0].bounds()), [false, false, true]);
        assert!(game.entity_set_hanging_direction(frame, Direction::West));
        assert_eq!(game.item_frames[0].facing, 4);
        assert_eq!(thin(game.item_frames[0].bounds()), [true, false, false]);
        assert!(game.entity_set_hanging_direction(frame, Direction::Up));
        assert_eq!(game.item_frames[0].facing, 1);
        assert_eq!(thin(game.item_frames[0].bounds()), [false, true, false]);
        assert!(game.entity_set_hanging_direction(frame, Direction::Down));
        assert_eq!(game.item_frames[0].facing, 0);
        assert!(game.entity_set_hanging_direction(frame, Direction::South));
        assert_eq!(game.item_frames[0].facing, 3);
    });
}

#[test]
#[ignore = "daily"]
fn a_painting_turns_only_to_a_horizontal_direction() {
    with_hanging(|game| {
        let painting = game.entity(PAINTING).expect("the painting's handle");
        // Its 2D data value: south 0, west 1, north 2, east 3.
        assert!(game.entity_set_hanging_direction(painting, Direction::East));
        assert_eq!(game.paintings[0].facing, 3);
        assert_eq!(thin(game.paintings[0].bounds()), [true, false, false]);
        assert!(game.entity_set_hanging_direction(painting, Direction::North));
        assert_eq!(game.paintings[0].facing, 2);
        assert!(!game.entity_set_hanging_direction(painting, Direction::Up));
        assert!(!game.entity_set_hanging_direction(painting, Direction::Down));
        assert_eq!(
            game.paintings[0].facing, 2,
            "a refused direction changes nothing"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_frames_item_turns_with_vanillas_wrap_and_other_kinds_are_refused() {
    with_hanging(|game| {
        let frame = game.entity(FRAME).expect("the frame's handle");
        let painting = game.entity(PAINTING).expect("the painting's handle");
        assert!(game.entity_set_item_frame_rotation(frame, 5));
        assert_eq!(game.item_frames[0].rotation, 5);
        // ItemFrame.setRotation keeps `rotation % 8`.
        assert!(game.entity_set_item_frame_rotation(frame, 13));
        assert_eq!(game.item_frames[0].rotation, 5);
        assert!(game.entity_set_item_frame_rotation(frame, 8));
        assert_eq!(game.item_frames[0].rotation, 0);
        // A painting has no item to turn.
        assert!(!game.entity_set_item_frame_rotation(painting, 2));
    });
}

/// The facing (`HangingEntity.DATA_DIRECTION`, entry 8) the tracker holds for entity `id`: its
/// value, or the default it sends once the value is left out (`ServerEntity.sendDirtyEntityData`).
fn tracked_facing(game: &mut super::Game<'_>, id: u32) -> Option<i32> {
    use horizon_protocol::entity_data::Value;
    let mut states = horizon_client::lan::entity::States::default();
    game.lan_object_states(&mut states, false, 0);
    let state = states
        .as_slice()
        .iter()
        .find(|e| e.id == id as i32)
        .expect("the hanging entity is tracked");
    let entry = state
        .meta
        .iter()
        .chain(&state.defaults)
        .find(|m| m.0 == 8)?;
    match entry.2 {
        Value::Id(facing) => Some(facing),
        _ => None,
    }
}

#[test]
#[ignore = "daily"]
fn a_hanging_entity_turned_back_to_south_is_sent_south() {
    super::native_guest_turn_tests::with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        with_hanging_in(game);
        let frame = game.entity(FRAME).expect("the frame's handle");
        let painting = game.entity(PAINTING).expect("the painting's handle");
        for (entity, id, away) in [(frame, FRAME, 4), (painting, PAINTING, 4)] {
            assert!(game.entity_set_hanging_direction(entity, Direction::West));
            assert_eq!(tracked_facing(game, id), Some(away));
            // South is the default, left out of the entity's data: the clients still hold west
            // unless the default is sent.
            assert!(game.entity_set_hanging_direction(entity, Direction::South));
            assert_eq!(tracked_facing(game, id), Some(3));
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_client_turns_its_painting_with_the_servers_direction() {
    use horizon_protocol::entity_data::{Value, serializer};
    super::native_entity_refs::tests::with_client_game(|game| {
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Paintings").unwrap();
        with_hanging_in(game);
        let direction = |d| [(8, serializer::DIRECTION, Value::Id(d))];
        // West (3D value 4): the painting keeps its 2D value, 1.
        game.remote_object_data(PAINTING as i32, &direction(4));
        assert_eq!(game.paintings[0].facing, 1);
        // Up is no direction a painting takes: it stays.
        game.remote_object_data(PAINTING as i32, &direction(1));
        assert_eq!(game.paintings[0].facing, 1);
        // Back to south, the default the server sends: 0.
        game.remote_object_data(PAINTING as i32, &direction(3));
        assert_eq!(game.paintings[0].facing, 0);
    });
}

/// A listener that records the dimension `hanging_break` names, and keeps the frame.
struct BreakWatcher(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for BreakWatcher {
    fn hanging_break(
        &self,
        _host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::HangingBreakEvent<'_>,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().push(event.dimension.to_owned());
        mod_api::Flow::Cancel
    }
}

/// `hanging_break` names the level the frame hangs in: a frame in the Nether reports
/// `minecraft:the_nether`, not the overworld.
#[test]
#[ignore = "daily"]
fn hanging_break_names_the_nether_it_happens_in() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "break-watcher",
        Box::new(BreakWatcher(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_hanging(|game| {
            game.dim = super::Dim::Nether;
            assert!(!game.break_frame_by(0, None), "the listener kept the frame");
            assert_eq!(*seen.lock().unwrap(), vec!["minecraft:the_nether"]);
        });
    });
}
