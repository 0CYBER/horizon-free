use super::*;
use horizon_core::nbt::{Compound, parse_snbt};

fn snbt(s: &str) -> Tag {
    parse_snbt(s).unwrap().0
}

#[test]
fn current_data_is_left_alone() {
    let mut tag = snbt("{DataVersion: 5023, Status: \"minecraft:full\"}");
    let before = tag.clone();
    update_chunk(&mut tag, "minecraft:overworld", Some("minecraft:noise"));
    assert_eq!(tag, before);
}

#[test]
fn item_stacks_of_1_20_take_components() {
    let mut player = snbt(
        "{DataVersion: 3465, Inventory: [{Slot: 0b, id: \"minecraft:diamond_sword\", Count: 1b, \
         tag: {Damage: 5, display: {Name: '{\"text\":\"Blade\"}'}}}]}",
    );
    update_player(&mut player);
    let stack = &dynamic::stream(player.get("Inventory"))[0];
    assert_eq!(
        dynamic::as_str(stack.get("id")).as_deref(),
        Some("minecraft:diamond_sword")
    );
    assert_eq!(stack.get("count"), Some(&Tag::Int(1)));
    let components = stack.get("components").expect("components");
    assert_eq!(components.get("minecraft:damage"), Some(&Tag::Int(5)));
    assert_eq!(
        components.get("minecraft:custom_name"),
        Some(&Tag::compound([("text", dynamic::string("Blade"))]))
    );
}

#[test]
fn level_data_of_1_20_moves_to_the_new_fields() {
    let mut level = snbt(
        "{DataVersion: 3465, Difficulty: 3b, DifficultyLocked: 1b, hardcore: 0b, SpawnX: 1, SpawnY: 64, SpawnZ: -3, \
         SpawnAngle: 90.0f, GameRules: {doFireTick: \"false\", keepInventory: \"true\", randomTickSpeed: \"5\"}}",
    );
    update(Ty::Level, &mut level, 3465, CURRENT);
    let settings = level
        .get("difficulty_settings")
        .expect("difficulty settings");
    assert_eq!(
        dynamic::as_str(settings.get("difficulty")).as_deref(),
        Some("hard")
    );
    assert_eq!(settings.get("locked"), Some(&Tag::Byte(1)));
    let rules = level.get("game_rules").expect("game rules");
    assert_eq!(rules.get("minecraft:keep_inventory"), Some(&Tag::Byte(1)));
    assert_eq!(rules.get("minecraft:random_tick_speed"), Some(&Tag::Int(5)));
    assert_eq!(
        rules.get("minecraft:fire_spread_radius_around_player"),
        Some(&Tag::Int(0))
    );
    assert_eq!(
        level.get("spawn").and_then(|s| s.get("pos")),
        Some(&Tag::IntArray(vec![1, 64, -3]))
    );
}

#[test]
fn block_states_take_their_new_field_names() {
    let mut chunk = snbt(
        "{DataVersion: 4903, Status: \"minecraft:full\", sections: [{Y: 0b, block_states: \
         {palette: [{Name: \"minecraft:chain\", Properties: {axis: \"y\"}}]}}]}",
    );
    update_chunk(&mut chunk, "minecraft:the_nether", Some("minecraft:noise"));
    let section = &dynamic::stream(chunk.get("sections"))[0];
    let state = &dynamic::stream(section.get("block_states").and_then(|b| b.get("palette")))[0];
    assert_eq!(
        dynamic::as_str(state.get("id")).as_deref(),
        Some("minecraft:chain")
    );
    assert!(state.get("properties").is_some());
    assert_eq!(chunk.get("DataVersion"), Some(&Tag::Int(CURRENT)));
}

#[test]
fn block_states_from_the_flattening_on_are_not_renamed() {
    if !tables::available() {
        eprintln!("skipping: no prepared datafix tables");
        return;
    }
    let props = [("facing".to_owned(), "north".to_owned())];
    assert_eq!(
        native_upgrade_block_state("minecraft:oak_stairs", &props, FLATTENING),
        ("minecraft:oak_stairs".to_owned(), props.to_vec())
    );
    assert_eq!(
        native_upgrade_block_state("minecraft:no_such_block", &[], 1400),
        ("minecraft:no_such_block".to_owned(), Vec::new())
    );
}

#[test]
fn block_states_from_before_the_flattening_take_their_new_names() {
    if !tables::available() {
        eprintln!("skipping: no prepared datafix tables");
        return;
    }
    let granite = [("variant".to_owned(), "granite".to_owned())];
    assert_eq!(
        native_upgrade_block_state("minecraft:stone", &granite, 1400),
        ("minecraft:granite".to_owned(), Vec::new())
    );
    // The same state written after the flattening is not upgraded.
    assert_eq!(
        native_upgrade_block_state("minecraft:stone", &granite, FLATTENING),
        ("minecraft:stone".to_owned(), granite.to_vec())
    );
}

/// `add_padding` must not divide by zero (`64 / bits`) or shift by 64+
/// (`1i64 << bits`) for `bits == 0` or `bits >= 64`.
#[test]
fn add_padding_rejects_zero_and_large_bits() {
    let empty: Vec<i64> = Vec::new();
    assert_eq!(super::fixes_mid::add_padding(10, 0, &[1, 2, 3]), empty);
    assert_eq!(super::fixes_mid::add_padding(10, 64, &[1, 2, 3]), empty);
    assert_eq!(super::fixes_mid::add_padding(10, 100, &[1, 2, 3]), empty);
    // Normal case still pads.
    let padded = super::fixes_mid::add_padding(10, 4, &[1, 2, 3]);
    assert!(!padded.is_empty());
}

const SIGN: &str = "{id: \"minecraft:sign\", x: 1, y: 64, z: 2, Text1: '{\"text\":\"north\"}', Text2: '\"\"', \
                    Text3: '\"\"', Text4: '\"\"', Color: \"black\", GlowingText: 0b}";
const VILLAGER: &str = "{id: \"minecraft:villager\", Pos: [0.5d, 64.0d, 0.5d], Profession: 1, Career: 2, \
                        CareerLevel: 1, Health: 20.0f, Passengers: [{id: \"minecraft:villager\", \
                        Pos: [0.5d, 66.0d, 0.5d], Profession: 2, Career: 1, CareerLevel: 1, Health: 20.0f}]}";

fn compound(tag: Tag) -> Compound {
    match tag {
        Tag::Compound(c) => c,
        other => panic!("expected a compound, got {other:?}"),
    }
}

fn text(tag: Option<&Tag>) -> Option<String> {
    dynamic::as_str(tag)
}

#[test]
fn a_1_20_sign_takes_its_front_text_as_the_chunk_loader_gives_it() {
    // DataVersion 3400 (1.20): the four lines move into `front_text.messages` (3439, the sign text fix).
    let old = 3400;
    let sign = snbt(SIGN);
    let alone = Tag::Compound(upgrade_block_entity(old, compound(sign.clone())).unwrap());
    let front = alone.get("front_text").expect("front_text");
    let messages = dynamic::stream(front.get("messages"));
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0], snbt("{text: \"north\"}"));
    assert_eq!(text(front.get("color")).as_deref(), Some("black"));
    assert!(alone.get("Text1").is_none());

    let mut chunk = Tag::compound([
        ("DataVersion", Tag::Int(old)),
        ("Status", dynamic::string("minecraft:full")),
        ("block_entities", Tag::List(vec![sign])),
    ]);
    update_chunk(&mut chunk, "minecraft:overworld", None);
    assert_eq!(dynamic::stream(chunk.get("block_entities"))[0], alone);
}

#[test]
fn a_1_14_villager_and_its_passenger_take_their_professions() {
    // DataVersion 1900: Profession 1 with Career 2 is a cartographer, Profession 2 with Career 1 a cleric.
    let old = 1900;
    let villager = snbt(VILLAGER);
    let alone = Tag::Compound(upgrade_entity(old, compound(villager.clone())).unwrap());
    let profession = |v: &Tag| text(v.get("VillagerData").and_then(|d| d.get("profession")));
    assert_eq!(
        profession(&alone).as_deref(),
        Some("minecraft:cartographer")
    );
    assert!(alone.get("Profession").is_none());
    let rider = &dynamic::stream(alone.get("Passengers"))[0];
    assert_eq!(profession(rider).as_deref(), Some("minecraft:cleric"));

    let mut record = Tag::compound([
        ("DataVersion", Tag::Int(old)),
        ("Entities", Tag::List(vec![villager])),
    ]);
    update_region_tag(Ty::EntityChunk, &mut record);
    assert_eq!(dynamic::stream(record.get("Entities"))[0], alone);
}

#[test]
fn versions_below_zero_or_past_the_games_are_refused_by_name() {
    let sign = compound(snbt(SIGN));
    assert_eq!(
        upgrade_block_entity(-1, sign.clone()),
        Err(UpgradeError::NegativeVersion(-1))
    );
    assert_eq!(
        upgrade_entity(CURRENT + 1, sign.clone()),
        Err(UpgradeError::FutureVersion(CURRENT + 1))
    );
    // The game's own version is no upgrade: the data comes back as it is.
    assert_eq!(
        upgrade_block_entity(CURRENT, sign.clone()),
        Ok(sign.clone())
    );
    let villager = compound(snbt(VILLAGER));
    assert_eq!(upgrade_entity(CURRENT, villager.clone()), Ok(villager));
    // Zero, the oldest version there is, is a valid source, not a refusal: a version-0 chunk knew no
    // `minecraft:sign` (its signs were `Sign`), so this one is kept as such a chunk keeps it.
    assert_eq!(upgrade_block_entity(0, sign.clone()), Ok(sign));
}

#[test]
fn unknown_and_unreadable_data_comes_back_unchanged() {
    // A block entity type a 3400 chunk doesn't know is kept as the chunk keeps it.
    let unknown = compound(snbt(
        "{id: \"minecraft:no_such_block_entity\", Text1: '\"x\"', x: 0, y: 0, z: 0}",
    ));
    assert_eq!(upgrade_block_entity(3400, unknown.clone()), Ok(unknown));
    // An entity with no id is no type any fix reads.
    let nameless = compound(snbt("{Profession: 1, Career: 2, Health: 20.0f}"));
    assert_eq!(upgrade_entity(1900, nameless.clone()), Ok(nameless));
}

#[test]
fn too_deep_data_is_refused_before_any_fix() {
    // Passengers 512 levels deep: the innermost compound sits at depth 1024, past the cap.
    let mut deep = compound(snbt("{id: \"minecraft:pig\"}"));
    for _ in 0..MAX_DEPTH {
        let mut outer = compound(snbt("{id: \"minecraft:pig\"}"));
        outer.insert("Passengers".into(), Tag::List(vec![Tag::Compound(deep)]));
        deep = outer;
    }
    assert_eq!(upgrade_entity(1900, deep), Err(UpgradeError::TooDeep));
    // Just within the cap: 255 riders put the innermost compound at depth 510.
    let mut within = compound(snbt("{id: \"minecraft:pig\"}"));
    for _ in 0..255 {
        let mut outer = compound(snbt("{id: \"minecraft:pig\"}"));
        outer.insert("Passengers".into(), Tag::List(vec![Tag::Compound(within)]));
        within = outer;
    }
    assert!(upgrade_entity(CURRENT, within).is_ok());
}

/// A pre-1.9 entity riding a chain of `riders` mounts, each mount nested in its rider's `Riding`.
fn riding_chain(riders: usize) -> Compound {
    let mut chain = compound(snbt("{id: \"Pig\"}"));
    for _ in 0..riders {
        let mut rider = compound(snbt("{id: \"Pig\"}"));
        rider.insert("Riding".into(), Tag::Compound(chain));
        chain = rider;
    }
    chain
}

#[test]
#[ignore = "daily"]
fn a_riding_chain_the_fix_makes_too_deep_is_refused_after_it() {
    // Data from before 1.13 is upgraded with the data fixers' tables of the prepared resources.
    set_resources(std::path::Path::new(
        &std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
    ));
    // DataVersion 100, before `EntityRidingToPassengersFix` (135): each mount is one level below its rider, and
    // after the fix each rider is two below its mount (a `Passengers` list, then the compound).
    let old = 100;
    // A small positive control verifies that the Riding fix actually runs.
    let upgraded = Tag::Compound(upgrade_entity(old, riding_chain(2)).unwrap());
    assert!(upgraded.get("Riding").is_none());
    assert_eq!(dynamic::stream(upgraded.get("Passengers")).len(), 1);
    // One rider with a deep opaque payload is enough: the input's deepest
    // compound is at 510, and moving the rider below Passengers adds two levels.
    // Avoid hundreds of riders: legacy fixes preserve additional nested data,
    // and repeatedly walking and cloning that chain is needlessly expensive.
    let mut rider = riding_chain(1);
    let mut payload = Tag::Compound(Compound::new());
    for _ in 0..509 {
        payload = Tag::compound([("x", payload)]);
    }
    rider.insert("payload".into(), payload);
    assert!(upgrade_entity(CURRENT, rider.clone()).is_ok());
    assert_eq!(
        upgrade_entity(old, rider),
        Err(UpgradeError::UpgradedTooDeep)
    );
}

/// A tree whose deepest list or compound, as encoded, is at depth `levels - 1` (`levels` levels counting the root),
/// reached through mixed lists: `x` holds one compound or none, then lists of a byte and the next list, each
/// next list two levels down (its wrapper compound, then itself), the last one a byte and an int, wrapped too.
fn mixed_lists(levels: usize) -> Compound {
    let (lists, compounds) = ((levels - 3) / 2, (levels - 3) % 2);
    let mut value = Tag::List(vec![Tag::Byte(1), Tag::Int(2)]);
    for _ in 0..lists {
        value = Tag::List(vec![Tag::Byte(0), value]);
    }
    for _ in 0..compounds {
        value = Tag::compound([("x", value)]);
    }
    let mut root = Compound::new();
    root.insert("x".into(), value);
    root
}

#[test]
fn mixed_list_wrappers_count_as_levels_as_the_writer_encodes_them() {
    use horizon_core::nbt::{JavaString, Limits, NamedTag, write_named};
    let written = |root: &Compound| {
        let named = NamedTag {
            name: JavaString::default(),
            tag: Tag::Compound(root.clone()),
        };
        write_named(&named, Limits::saved()).is_ok()
    };
    // 512 levels: the deepest wrapper at 511, which the writer takes and so does the upgrade.
    let at_cap = mixed_lists(MAX_DEPTH);
    assert!(written(&at_cap));
    assert_eq!(upgrade_entity(CURRENT, at_cap.clone()), Ok(at_cap));
    // 513 levels: the deepest wrapper at 512, which the writer refuses, and the upgrade refuses by name.
    let past = mixed_lists(MAX_DEPTH + 1);
    assert!(!written(&past));
    assert_eq!(upgrade_entity(CURRENT, past), Err(UpgradeError::TooDeep));
    // Counted without wrappers, a root with `x: [1b, 2]` is 2 levels deep; encoded, it is 3.
    assert_eq!(mixed_lists(3).get(&"x".into()), Some(&snbt("[1b, 2]")));
}

#[test]
fn large_data_is_not_refused() {
    // Past the 1 GiB this upgrade used to refuse, at the game's own version: zeroed and never read, so the pages
    // are not touched.
    let mut large = Compound::new();
    large.insert("data".into(), Tag::ByteArray(vec![0; (1 << 30) + 1]));
    let upgraded = upgrade_entity(CURRENT, large).unwrap();
    assert!(
        matches!(upgraded.get(&"data".into()), Some(Tag::ByteArray(v)) if v.len() == (1 << 30) + 1)
    );
    // An upgrade that runs: a villager with a wide, shallow list of 200,000 entries comes back fixed, its list kept.
    let mut villager = compound(snbt(VILLAGER));
    villager.insert("Tags".into(), Tag::List(vec![Tag::Int(7); 200_000]));
    let upgraded = Tag::Compound(upgrade_entity(1900, villager).unwrap());
    assert!(upgraded.get("VillagerData").is_some());
    assert_eq!(dynamic::stream(upgraded.get("Tags")).len(), 200_000);
}
