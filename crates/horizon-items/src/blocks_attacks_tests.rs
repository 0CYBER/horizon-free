//! `BlocksAttacks.CODEC`: the stack's own `blocks_attacks` component and the
//! forms its fields take (`RegistryCodecs.holderSet`, `SoundEvent.CODEC`).
use super::*;
use serde_json::json;

/// `stack` carrying `component` as its own `blocks_attacks`.
fn with_component(stack: ItemStack, component: Value) -> ItemStack {
    let mut stack = stack;
    stack.data_mut().other.push((
        "minecraft:blocks_attacks".into(),
        horizon_client_core::scoreboard::json_to_tag(&component),
    ));
    stack
}

fn parse(component: Value) -> BlocksAttacks {
    let stack = with_component(ItemStack::new("minecraft:shield", 1), component);
    BlocksAttacks::of_stack(&stack, None)
        .expect("the stack's own component")
        .into_owned()
}

#[test]
fn a_stacks_own_blocks_attacks_component_counts() {
    let plain = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(BlocksAttacks::of_stack(&plain, None).is_none());
    let b = parse(json!({
        "block_delay_seconds": 0.0,
        "disable_cooldown_scale": 0.0,
        "damage_reductions": [{"horizontal_blocking_angle": 180.0, "base": -0.5, "factor": 0.5}],
        "item_damage": {"threshold": 1.0e9, "base": 0.0, "factor": 0.0},
        "bypassed_by": "#minecraft:bypasses_armor",
    }));
    assert_eq!(b.delay_seconds, 0.0);
    assert_eq!(b.disable_scale, 0.0);
    assert_eq!(b.reductions.len(), 1);
    assert_eq!(b.reductions[0].horizontal_angle, 180.0);
    assert_eq!((b.reductions[0].base, b.reductions[0].factor), (-0.5, 0.5));
    assert_eq!(b.item_damage.0, 1.0e9);
    assert!(b.bypassed_by.is_some(), "the tag is kept");
    // An empty compound is the codec's defaults: a shield's 90 degrees.
    let b = parse(json!({}));
    assert_eq!(b.reductions[0].horizontal_angle, 90.0);
    assert_eq!(b.disable_scale, 1.0);
    assert_eq!(b.bypassed_by, None);
}

/// `RegistryCodecs.holderSet` takes a tag, one id or a list of them, so a
/// `bypassed_by` of any of the three bypasses those damage types; the sounds
/// take `SoundEvent.CODEC`'s inline `{sound_id, range}` as well as an id.
#[test]
fn the_component_takes_every_form_of_a_holder_set_and_a_sound_event() {
    for bypassed in [
        json!("minecraft:mob_attack"),
        json!(["minecraft:mob_attack"]),
        json!(["#minecraft:is_projectile", "minecraft:mob_attack"]),
    ] {
        let b = parse(json!({
            "bypassed_by": bypassed,
            "block_sound": {"sound_id": "minecraft:block.anvil.land", "range": 8.0},
            "disabled_sound": {"sound_id": "minecraft:item.shield.break"},
        }));
        assert!(
            b.bypassed_by.is_some(),
            "{bypassed} is a holder set the component keeps"
        );
        assert_eq!(b.block_sound.as_deref(), Some("minecraft:block.anvil.land"));
        assert_eq!(
            b.disabled_sound.as_deref(),
            Some("minecraft:item.shield.break")
        );
    }
    // An id alone is the other form of the same codec.
    let b = parse(json!({
        "bypassed_by": "#minecraft:is_projectile",
        "block_sound": "minecraft:item.shield.block",
    }));
    assert!(b.bypassed_by.is_some(), "the tag is kept");
    assert_eq!(
        b.block_sound.as_deref(),
        Some("minecraft:item.shield.block")
    );
    assert_eq!(b.disabled_sound, None);
}
