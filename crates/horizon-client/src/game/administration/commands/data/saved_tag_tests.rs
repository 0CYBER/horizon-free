use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use std::collections::HashSet;

/// A server-side game that runs commands, its world in a scratch folder.
pub(super) fn with_command_game(name: &str, test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let scratch = std::env::temp_dir().join(format!(
        "horizon-saved-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(10.5, 200.0, 0.5)).unwrap());
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    crate::game::native_entity_refs::tests::bind_local_player(&mut game, 1, 1);
    game.build_registries();
    game.set_allow_commands(true);
    test(&mut game);
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "daily"]
fn tag_commands_and_explicit_empty_loads_save_without_empty_tags() {
    with_command_game("tags", tag_commands_and_explicit_empty);
}
fn tag_commands_and_explicit_empty(game: &mut Game<'_>) {
    let start = game.world.game_time;
    let uuid = "00000148-0000-0000-0000-000000000001";
    assert!(game.run_command("summon pig 4.5 200 4.5 {UUID:[I;328,0,0,1],Health:10f,NoAI:1b,NoGravity:1b,PortalCooldown:17}"));
    let target = game.target_with_uuid([328, 0, 0, 1]).unwrap();
    assert!(matches!(target, Target::Mob(_)));
    assert!(game.tags_of(target).is_empty());
    assert!(
        game.data_of(&DataRef::Entity(target))
            .unwrap()
            .get("Tags")
            .is_none()
    );
    for tagged in [true, false, true, false] {
        assert!(game.run_command(&format!(
            "tag {uuid} {} audit",
            if tagged { "add" } else { "remove" }
        )));
        let expected: Vec<String> = if tagged { vec!["audit".into()] } else { vec![] };
        assert_eq!(game.tags_of(target), expected);
        for _ in 0..2 {
            let saved = game.data_of(&DataRef::Entity(target)).unwrap();
            assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(vec![328, 0, 0, 1])));
            assert_eq!(saved.get("Health"), Some(&Tag::Float(10.0_f32.to_bits())));
            assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("NoGravity"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(17)));
            assert_eq!(
                saved.get("Tags"),
                if tagged {
                    Some(Tag::List(vec![Tag::String("audit".into())]))
                } else {
                    None
                }
                .as_ref()
            );
            assert_eq!(game.tags_of(target), expected);
        }
    }
    assert!(game.run_command(
        "summon pig 8.5 200 4.5 {UUID:[I;328,0,0,2],NoAI:1b,Tags:[],PortalCooldown:23}"
    ));
    let empty = game.target_with_uuid([328, 0, 0, 2]).unwrap();
    assert!(game.tags_of(empty).is_empty());
    let saved = game.data_of(&DataRef::Entity(empty)).unwrap();
    assert!(saved.get("Tags").is_none());
    assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(23)));
    assert!(game.run_command("tag 00000148-0000-0000-0000-000000000002 add keep"));
    assert_eq!(game.tags_of(empty), ["keep"]);
    assert_eq!(
        game.data_of(&DataRef::Entity(empty)).unwrap().get("Tags"),
        Some(&Tag::List(vec![Tag::String("keep".into())]))
    );
    assert_eq!(game.world.game_time, start);
}

/// `/data get` on a cloud that was never saved shows `Entity.saveWithoutId`'s
/// common fields, as 26.3 prints them (#13057).
#[test]
#[ignore = "daily"]
fn a_live_area_effect_cloud_reads_with_the_common_entity_fields() {
    with_command_game("cloud", |game| {
        assert!(game.run_command("summon area_effect_cloud 4.5 200 4.5 {UUID:[I;328,0,0,3]}"));
        let target = game.target_with_uuid([328, 0, 0, 3]).unwrap();
        let read = game.data_of(&DataRef::Entity(target)).unwrap();
        let zero = Tag::Double(0);
        assert_eq!(read.get("Motion"), Some(&Tag::List(vec![zero.clone(); 3])));
        assert_eq!(read.get("fall_distance"), Some(&zero));
        assert_eq!(read.get("Air"), Some(&Tag::Short(300)));
        assert_eq!(read.get("OnGround"), Some(&Tag::Byte(0)));
        assert_eq!(read.get("Invulnerable"), Some(&Tag::Byte(0)));
        assert_eq!(read.get("PortalCooldown"), Some(&Tag::Int(0)));
    });
}

/// What `command` told the player.
fn say(game: &mut Game<'_>, command: &str) -> Vec<Text> {
    game.chat_capture = Some(Vec::new());
    assert!(game.run_command(command), "{command}");
    game.chat_capture.take().unwrap_or_default()
}

/// The `Tags` key of the compound the entity saves (`Entity.saveWithoutId`).
fn saved_tags(game: &mut Game<'_>, target: Target) -> Option<Tag> {
    game.data_of(&DataRef::Entity(target))
        .unwrap()
        .get("Tags")
        .cloned()
}

fn tag_list(tags: &[&str]) -> Tag {
    Tag::List(tags.iter().map(|t| Tag::String((*t).into())).collect())
}

/// `TagCommand.listTags` sends through `CommandResponseTracker.sendFeedback`'s
/// default element type, `NON_ZERO`: one entity of the selection holding tags
/// prints the single-target form, and the count in the multiple form is the
/// number of entities with tags, not the number of targets.
#[test]
#[ignore = "daily"]
fn tag_list_counts_only_the_entities_that_have_tags() {
    with_command_game("tag-list", |game| {
        for (uuid, x) in [("1", "4.5"), ("2", "8.5")] {
            assert!(game.run_command(&format!(
                "summon pig {x} 200 4.5 {{UUID:[I;329,0,0,{uuid}],NoAI:1b,Silent:1b,NoGravity:1b}}"
            )));
        }
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000001 add audit"));
        let said = say(game, "tag @e[type=minecraft:pig] list");
        assert_eq!(said.len(), 1);
        assert_eq!(
            said[0].translation_key(),
            Some("commands.tag.list.single.success")
        );
        assert_eq!(said[0].plain(), "Pig has 1 tag(s): audit");
        // Both tagged: the multiple form, counting the tagged entities.
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000002 add audit"));
        let said = say(game, "tag @e[type=minecraft:pig] list");
        assert_eq!(
            said[0].translation_key(),
            Some("commands.tag.list.multiple.success")
        );
        assert_eq!(said[0].plain(), "The 2 entities have 1 total tag(s): audit");
    });
}

/// `TagCommand.listTags` sends through `sendFeedback(source, false, …)`,
/// while `addTag`/`removeTag` pass `true` (`CommandSourceStack.sendSuccess`):
/// only a change is repeated to the operators.
#[test]
#[ignore = "daily"]
fn tag_list_is_not_repeated_to_the_operators() {
    with_command_game("tag-list-broadcast", |game| {
        assert!(game.run_command(
            "summon pig 4.5 200 4.5 {UUID:[I;329,0,0,4],NoAI:1b,Silent:1b,NoGravity:1b}"
        ));
        // A source that informs the operators (`shouldInformAdmins`).
        game.console_broadcast = Some(true);
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000004 add audit"));
        assert_eq!(game.admin_feedback.len(), 1);
        game.admin_feedback.clear();
        let said = say(game, "tag @e[type=minecraft:pig] list");
        assert_eq!(said.len(), 1);
        assert_eq!(game.admin_feedback.len(), 0);
    });
}

/// `TagCommand.addTag`/`removeTag` store a tag on an ominous item spawner
/// (`OminousItemSpawner.readAdditionalSaveData` keeps the compound, its chunk
/// saves it), and `Entity.saveWithoutId` writes `Tags` only for a nonempty set,
/// so removing the last tag of a player or any entity leaves no key.
#[test]
#[ignore = "daily"]
fn the_last_tag_leaves_no_key_and_a_spawner_keeps_its_tags() {
    with_command_game("tag-remove", |game| {
        assert!(game.run_command("tag @s add audit"));
        assert_eq!(saved_tags(game, Target::Player), Some(tag_list(&["audit"])));
        assert!(game.run_command("tag @s remove audit"));
        assert_eq!(saved_tags(game, Target::Player), None);

        assert!(game.run_command(
            "summon ominous_item_spawner 4.5 200 4.5 {UUID:[I;329,0,0,3],NoGravity:1b}"
        ));
        let spawner = game.target_with_uuid([329, 0, 0, 3]).unwrap();
        assert!(matches!(spawner, Target::Spawner(_)));
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000003 add audit"));
        assert_eq!(game.tags_of(spawner), ["audit"]);
        assert_eq!(saved_tags(game, spawner), Some(tag_list(&["audit"])));
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000003 remove audit"));
        assert_eq!(saved_tags(game, spawner), None);
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000003 add audit"));
        // The entity the chunk saves carries it, and loading it back keeps it.
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        let saved = chunks
            .iter()
            .flat_map(|(_, entities)| entities)
            .find(|t| t.get("id") == Some(&Tag::String("minecraft:ominous_item_spawner".into())))
            .cloned()
            .expect("the spawner's saved entity");
        assert_eq!(saved.get("Tags"), Some(&tag_list(&["audit"])));
        let before = game.ominous_spawners.len();
        game.load_ominous_item_spawner(saved);
        assert_eq!(game.ominous_spawners.len(), before + 1);
        let reloaded = game.ominous_spawners.last().unwrap().id;
        assert_eq!(game.tags_of(Target::Spawner(reloaded)), ["audit"]);
    });
}
