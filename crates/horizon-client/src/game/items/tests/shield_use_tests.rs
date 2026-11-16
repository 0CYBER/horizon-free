//! What a shield's use and the hits it takes do at the edges: the hit that
//! empties the stack (`hurtBlockingItem` wears the stack before `hurtServer`
//! reads its component again), a `bypassed_by` list, and a stack carrying both
//! a `consumable` and `blocks_attacks` (`Item.use` reads the consumable first).
use crate::game::*;
use super::OFFHAND_SLOT;
use horizon_client::{
    health::{BlockOutcome, HealthEvent, HurtSource},
    items::ItemStack,
    motion::MotionEvent,
};
use horizon_core::nbt::Tag;

/// A server's game with the player standing at (0.5, 64, 0.5), or `None`
/// without prepared resources.
fn game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    Some(game)
}

/// `stack` carrying a `blocks_attacks` of its own whose `bypassed_by` is `bypassed`.
fn bypassing_stack(stack: ItemStack, bypassed: Tag) -> ItemStack {
    let mut stack = stack;
    stack.data_mut().other.push((
        "minecraft:blocks_attacks".into(),
        Tag::compound([("bypassed_by", bypassed)]),
    ));
    stack
}

/// `LivingEntity.hurtServer` reads `BLOCKS_ATTACKS` off the used stack after
/// `applyItemBlocking` wore it: the hit that empties the stack leaves no
/// component, so it plays the break sound alone and broadcasts the damage
/// event instead of the block sound. A stack with one left keeps its component.
#[test]
fn a_breaking_hit_plays_no_block_sound_and_broadcasts_the_damage() {
    let source = HurtSource {
        damage_type: "minecraft:mob_attack",
        causing: 1,
        direct: 1,
        position: None,
    };
    // One point short of the shield's 336: the hit's 6 points of wear break it.
    // `sound` is `onBlocked` having run (`blocked` with the component there, on
    // the fresh hurt path); the hits without it broadcast from
    // `finish_hurt_server`'s else branch already.
    let Some(mut game) = game() else {
        return;
    };
    for (count, sound, block_sound, broadcasts) in [
        (1, true, None, 1),
        (1, false, None, 0),
        (2, true, Some(&1u32), 0),
    ] {
        // The sound log and the events are the last case's.
        game.audio.heard.clear();
        if let Some(p) = game.player.as_mut() {
            p.events.clear();
        }
        game.offhand = Some(ItemStack {
            count,
            damage: 335,
            ..ItemStack::new("minecraft:shield", 1)
        });
        game.using = Some((OFFHAND_SLOT, 0));
        let outcome = BlockOutcome {
            blocking_item_used: true,
            hurt: true,
            blocked: 5.0,
            shield_damage: 5,
            fully_blocked: true,
            sound,
            ..Default::default()
        };
        game.after_block(
            outcome,
            (None, false),
            source,
            &horizon_client::health::NO_ATTACK,
        );
        let left = game.offhand.as_ref().map(|s| s.count);
        assert_eq!(left, (count > 1).then(|| count - 1), "one of {count}");
        assert_eq!(
            game.audio.heard.get("minecraft:item.shield.break"),
            Some(&1),
            "the break sound, count={count} sound={sound}"
        );
        assert_eq!(
            game.audio.heard.get("minecraft:item.shield.block"),
            block_sound,
            "the block sound, count={count} sound={sound}"
        );
        let player = game.player.as_ref().unwrap();
        let broadcast = player
            .events
            .iter()
            .filter(|e| matches!(e, MotionEvent::Health(HealthEvent::DamageBroadcast { .. })))
            .count();
        assert_eq!(
            broadcast, broadcasts,
            "the broadcast, count={count} sound={sound}"
        );
    }
}

/// `RegistryCodecs.holderSet` takes a tag, one id or a list of them, so a
/// `bypassed_by` in any of those forms lets that damage type through.
#[test]
fn a_bypassed_by_list_lets_the_hit_through() {
    let Some(mut game) = game() else {
        return;
    };
    let forms = [
        Tag::List(vec![Tag::String("minecraft:mob_attack".into())]),
        Tag::String("minecraft:mob_attack".into()),
        Tag::String("#minecraft:panic_causes".into()),
    ];
    for bypassed in forms {
        game.offhand = Some(bypassing_stack(
            ItemStack::new("minecraft:shield", 1),
            bypassed.clone(),
        ));
        game.using = Some((OFFHAND_SLOT, 0));
        let blocking = game
            .player_blocking(Some([0.5, 64.0, 2.5]), "minecraft:mob_attack", false)
            .expect("the shield is up");
        assert!(blocking.bypassed, "{bypassed:?} bypasses the mob attack");
        assert_eq!(blocking.blocked(5.0), 0.0, "{bypassed:?} blocks nothing");
    }
    // A list that does not name it still blocks.
    game.offhand = Some(bypassing_stack(
        ItemStack::new("minecraft:shield", 1),
        Tag::List(vec![Tag::String("minecraft:fall".into())]),
    ));
    game.using = Some((OFFHAND_SLOT, 0));
    // In front of the player (yaw 0 faces +z), inside the shield's 90 degrees.
    let blocking = game
        .player_blocking(Some([0.5, 64.0, 2.5]), "minecraft:mob_attack", false)
        .expect("the shield is up");
    assert!(!blocking.bypassed);
    assert!(blocking.blocked(5.0) > 0.0);
}

/// `Item.use` reads `consumable` before `blocks_attacks`: a stack carrying
/// both is eaten, and never raised as a shield.
#[test]
fn a_stack_with_a_consumable_is_eaten_not_raised() {
    let Some(mut game) = game() else {
        return;
    };
    let mut apple = ItemStack::new("minecraft:golden_apple", 1);
    // Eaten at once: a `Consumable` of 0 seconds consumes without a use.
    apple.data_mut().set_other(
        "minecraft:consumable",
        Some(Tag::compound([(
            "consume_seconds",
            Tag::Float(0.0_f32.to_bits()),
        )])),
    );
    apple.data_mut().other.push((
        "minecraft:blocks_attacks".into(),
        Tag::compound([("block_delay_seconds", Tag::Float(0.0_f32.to_bits()))]),
    ));
    *game.held_mut() = Some(apple);
    assert!(game.use_hand(None).unwrap(), "the use acts");
    assert!(game.using.is_none(), "nothing is held up");
    assert!(game.held().is_none(), "the apple is eaten");
    assert_eq!(
        game.audio.heard.get("minecraft:entity.player.burp"),
        Some(&1),
        "eating it burps"
    );
}
