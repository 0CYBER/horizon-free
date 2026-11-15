use super::Game;
use crate::game::native_guest_turn_tests::with_guests;
use horizon_client::{advancements as a, chunk::Chunk, items::ItemStack as EngineStack};
use mod_api::{
    CriterionContext, CriterionTest, CriterionTestError, Identifier, TriggerCriterionError,
};
use serde_json::json;
use std::sync::Arc;

fn install(game: &mut Game<'_>) {
    game.build_registries();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let mut tree = a::Tree::default();
    for (id, trigger, conditions) in [
        (
            "default",
            "default_block_use",
            json!({"location":{"type":"minecraft:location_check","predicate":{"position":{"x":0.5,"y":64.5,"z":0.5}}}}),
        ),
        (
            "item2",
            "item_used_on_block",
            json!({"location":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stone","count":2}}}),
        ),
        (
            "any2",
            "any_block_use",
            json!({"location":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stone","count":2}}}),
        ),
        (
            "any1",
            "any_block_use",
            json!({"location":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stone","count":1}}}),
        ),
        ("any", "any_block_use", json!({})),
    ] {
        let trigger = format!("minecraft:{trigger}");
        let index = tree.list.len();
        let id = format!("test:{id}");
        tree.by_id.insert(id.clone(), index);
        tree.by_trigger
            .entry(trigger.clone())
            .or_default()
            .push((index, 0));
        tree.list.push(a::Advancement {
            id,
            parent: None,
            display: None,
            criteria: vec![a::Criterion {
                name: "seen".into(),
                trigger,
                conditions: conditions.into(),
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
    }
    game.adv.tree = Arc::new(tree);
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_all(|g| g.advancements_loaded = true);
}
fn has(game: &Game<'_>, id: &str) -> bool {
    game.lan
        .as_ref()
        .unwrap()
        .hub
        .guest(12)
        .unwrap()
        .advancements
        .has(&format!("test:{id}"), "seen")
}

#[test]
#[ignore = "daily"]
fn default_block_use_awards_only_default() {
    with_guests(|g| {
        install(g);
        let p = g.player(12).unwrap();
        assert_eq!(p.trigger_block_use(g, [0, 64, 0]), Ok(()));
        assert!(has(g, "default"));
        assert!(!has(g, "any"));
        assert!(!has(g, "item2"));
        assert!(!has(g, "any1"));
        assert!(!has(g, "any2"));
    });
}

#[test]
#[ignore = "daily"]
fn item_block_use_distinguishes_snapshots() {
    with_guests(|g| {
        install(g);
        let p = g.player(12).unwrap();
        let key = mod_api::__private::item_key("minecraft:stone").unwrap();
        let original = mod_api::ItemStack::new(key, 2).unwrap();
        g.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |g| {
                g.inventory[0] = Some(EngineStack::new("minecraft:dirt", 1))
            })
            .unwrap();
        assert_eq!(
            p.trigger_item_used_on_block(g, [0, 64, 0], &original),
            Ok(())
        );
        assert!(has(g, "item2"));
        assert!(!has(g, "any1"));
        assert!(!has(g, "any"));
        assert!(!has(g, "any2"));
        assert!(!has(g, "default"));
    });
}

#[test]
#[ignore = "daily"]
fn block_use_refusals_grant_nothing() {
    with_guests(|g| {
        install(g);
        let p = g.player(12).unwrap();
        assert_eq!(
            p.trigger_block_use(g, [100000, 64, 100000]),
            Err(TriggerCriterionError::UnloadedChunk)
        );
        let invalid = mod_api::ItemStack::new(
            mod_api::__private::item_key("minecraft:stone").unwrap(),
            999,
        )
        .unwrap();
        assert_eq!(
            p.trigger_item_used_on_block(g, [0, 64, 0], &invalid),
            Err(TriggerCriterionError::Stack(
                mod_api::StackWriteError::CountExceeded { maximum: 64 }
            ))
        );
        struct Never;
        impl CriterionTest for Never {
            fn test(&self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                panic!("vanilla guard must precede callback");
            }
        }
        assert_eq!(
            p.trigger_criterion(
                g,
                &Identifier::parse("minecraft:any_block_use").unwrap(),
                Arc::new(Never)
            ),
            Err(TriggerCriterionError::InvalidTrigger)
        );
        let b = g.player(13).unwrap();
        g.as_guest(12, |g| {
            g.as_guest(13, |g| {
                assert_eq!(
                    p.trigger_block_use(g, [0, 64, 0]),
                    Err(TriggerCriterionError::PlayerUnavailable)
                );
                assert_eq!(
                    b.trigger_block_use(g, [100000, 64, 100000]),
                    Err(TriggerCriterionError::UnloadedChunk)
                );
            })
            .unwrap();
        })
        .unwrap();
        let stale = mod_api::__private::game::player_ref(p.entity(), 999);
        assert_eq!(
            stale.trigger_block_use(g, [0, 64, 0]),
            Err(TriggerCriterionError::PlayerUnavailable)
        );
        for id in ["default", "item2", "any2", "any1", "any"] {
            assert!(!has(g, id));
        }
    });
}

#[test]
#[ignore = "daily"]
fn remote_block_use_refuses() {
    crate::game::native_entity_refs::tests::with_client_game(|g| {
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        g.connect_local(pipe, "CriteriaClient").unwrap();
        let p = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 1),
            12,
        );
        assert_eq!(
            p.trigger_block_use(g, [0, 64, 0]),
            Err(TriggerCriterionError::NotServer)
        );
    });
}
