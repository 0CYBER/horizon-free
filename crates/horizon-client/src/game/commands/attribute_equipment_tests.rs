//! `/attribute` on a mob: a lowered `max_health` lowers `Health` in the mob's
//! next tick (`LivingEntity.refreshDirtyAttributes` -> `onAttributeUpdated`),
//! and reads see the modifiers of what the mob holds or wears, one per id
//! (`LivingEntity.collectEquipmentChanges`).
use super::vehicle_damage_command_tests::with_game;
use crate::game::Game;

/// The chat the command sent, and whether it was accepted.
fn say(game: &mut Game<'_>, command: &str) -> (bool, String) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter().map(horizon_client::text::Text::plain).collect(),
    )
}

fn health(game: &Game<'_>, uuid: [i32; 4]) -> Option<f32> {
    game.mobs
        .iter()
        .find(|m| game.persist.uuid_of(m.id) == Some(uuid))
        .map(|m| m.health)
}

const PIG: &str = "00000af1-0000-0000-0000-000000000001";
const ZOMBIE: &str = "00000af1-0000-0000-0000-000000000002";

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_lowered_maximum_lowers_health_and_gear_counts_for_attributes() {
    with_game(|game| {
        assert!(game.run_command(
            "summon minecraft:pig 2.5 200 2.5 {UUID:[I;2801,0,0,1],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b}"
        ));
        assert!(game.run_command(&format!("attribute {PIG} minecraft:max_health base set 1")));
        // The command only marks the instance dirty; the mob's tick clamps.
        assert_eq!(health(game, [2801, 0, 0, 1]), Some(10.0));
        game.tick_server(false).unwrap();
        assert_eq!(health(game, [2801, 0, 0, 1]), Some(1.0));
        let (_, said) = say(game, &format!("data get entity {PIG} Health"));
        assert!(said.ends_with(": 1.0f"), "data get Health {said:?}");

        assert!(game.run_command(
            "summon minecraft:zombie 8.5 200 2.5 {UUID:[I;2801,0,0,2],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,CanPickUpLoot:0b}"
        ));
        let equip =
            |slot: &str, item: &str| format!("item replace entity {ZOMBIE} {slot} with {item}");
        assert!(game.run_command(&equip("weapon.mainhand", "minecraft:iron_sword")));
        // 3 + the sword's 5.
        let (ok, said) = say(
            game,
            &format!("attribute {ZOMBIE} minecraft:attack_damage get"),
        );
        assert!(ok && said.ends_with("is 8.0"), "attack_damage get {said:?}");
        let (ok, said) = say(
            game,
            &format!(
                "attribute {ZOMBIE} minecraft:attack_damage modifier value get minecraft:base_attack_damage"
            ),
        );
        assert!(
            ok && said.ends_with("is 5.0"),
            "modifier value get {said:?}"
        );
        // The off hand's modifier of the same id replaces the sword's.
        assert!(game.run_command(&equip(
            "weapon.offhand",
            "minecraft:stick[attribute_modifiers=[{type:\"attack_damage\",id:\"minecraft:base_attack_damage\",amount:2,operation:\"add_value\",slot:\"any\"}]]"
        )));
        let (ok, said) = say(
            game,
            &format!("attribute {ZOMBIE} minecraft:attack_damage get"),
        );
        assert!(ok && said.ends_with("is 5.0"), "replaced id {said:?}");
        // Worn armour counts in the armour attribute: 2 + the chestplate's 6.
        assert!(game.run_command(&equip("armor.chest", "minecraft:iron_chestplate")));
        let (ok, said) = say(game, &format!("attribute {ZOMBIE} minecraft:armor get"));
        assert!(ok && said.ends_with("is 8.0"), "armor get {said:?}");
    });
}
