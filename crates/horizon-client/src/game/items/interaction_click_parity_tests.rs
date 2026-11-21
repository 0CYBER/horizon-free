//! The Rust side of tools/test_interaction_click_parity.py: with
//! `HORIZON_INTERACTION_CLICK_TRACE` pointing at a file, one use click of a guest's
//! interact packet lands on a server `minecraft:interaction` once per `response`
//! value, and the entity's saved fields go out in the format of
//! `VanillaInteractionClickOracle`.
use super::*;

/// The player a guest's interact packet names, as `UUIDUtil` saves it (`PlayerAction`).
const PLAYER: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
/// The id of the box the click names.
const BOX: u32 = 900;

/// A server `Interaction` holding the settings the guest's packet reached.
fn box_entity(width: f32, height: f32, response: bool) -> commands::OtherEntity {
    commands::OtherEntity {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id: BOX,
        kind: "minecraft:interaction".into(),
        position: horizon_core::geometry::Vec3::new(4.5, 70.0, 4.5),
        yaw: 0.0,
        pitch: 0.0,
        item_transformation: None,
        interaction: Some(crate::game::interactions::InteractionState {
            width,
            height,
            response,
            attack: None,
            interaction: None,
        }),
        knot_check: None,
        eye: None,
    }
}

/// `Interaction.addAdditionalSaveData`'s fields, read back from the tag the game would
/// write: the box, `response`, and the `interaction` record the click made.
fn saved(fields: &horizon_core::nbt::Tag, key: &str) -> String {
    use horizon_core::nbt::Tag;
    match fields.get(key) {
        Some(Tag::Float(bits)) => format!("{bits:08x}"),
        other => panic!("{key} saved as {other:?}"),
    }
}

fn case(name: &str, result: &str, state: &crate::game::interactions::InteractionState) -> String {
    use horizon_core::nbt::Tag;
    let mut written = horizon_core::nbt::Compound::new();
    state.write(&mut written);
    let fields = Tag::Compound(written);
    let pressed = match fields.get("interaction") {
        Some(Tag::Compound(record)) => Tag::Compound(record.clone()),
        other => panic!("the click saved no interaction record: {other:?}"),
    };
    let player = match pressed.get("player") {
        Some(Tag::IntArray(player)) => player.clone(),
        other => panic!("player saved as {other:?}"),
    };
    let timestamp = match pressed.get("timestamp") {
        Some(Tag::Long(timestamp)) => *timestamp,
        other => panic!("timestamp saved as {other:?}"),
    };
    let response = match fields.get("response") {
        Some(Tag::Byte(response)) => i32::from(*response),
        other => panic!("response saved as {other:?}"),
    };
    format!(
        "{{\"case\":\"{name}\",\"result\":\"{result}\",\"width\":\"{}\",\"height\":\"{}\",\
         \"response\":{response},\"interaction\":{{\"player\":[{},{},{},{}],\"timestamp\":{timestamp}}}}}",
        saved(&fields, "width"),
        saved(&fields, "height"),
        player[0],
        player[1],
        player[2],
        player[3],
    )
}

/// The two clicks of the harness: a box that does not respond and one that does, with
/// their own widths, heights and game times. The click is set where a guest's interact
/// packet arrives, as `interact_entity` receives it on a server.
#[test]
#[ignore = "parity harness: tools/test_interaction_click_parity.py runs it"]
fn a_click_on_a_server_interaction_consumes_like_vanilla() {
    let Some(trace) = std::env::var_os("HORIZON_INTERACTION_CLICK_TRACE") else {
        return;
    };
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.lan_acting_uuid = Some(PLAYER);
        // A stack the click must not reach: the box ends the click before the item.
        game.inventory[0] = Some(ItemStack::new("minecraft:stone", 5));
        let mut cases = Vec::new();
        for response in [false, true] {
            let (width, height, time) = match response {
                false => (1.5, 2.25, 4321),
                true => (2.5, 3.25, 8642),
            };
            game.world.game_time = time;
            game.others.clear();
            game.others.push(box_entity(width, height, response));
            game.guest_use = Some(GuestUse::Entity(
                Hand::Main,
                InteractTarget {
                    id: BOX as i32,
                    location: [0.0, 1.0, 0.0],
                },
            ));
            let eaten = game.use_item_as_player().expect("the click runs").is_some();
            assert_eq!(
                game.held().map(|stack| stack.count),
                Some(5),
                "the item behind the box was not used"
            );
            let state = game.others[0]
                .interaction
                .expect("the box keeps its settings");
            cases.push(case(
                if response {
                    "response_true"
                } else {
                    "response_false"
                },
                if eaten { "consume" } else { "other" },
                &state,
            ));
        }
        std::fs::write(&trace, format!("[{}]\n", cases.join(","))).expect("the trace is written");
    });
}
