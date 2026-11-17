//! A `!minecraft:<component>` removal patch beats the item prototype's
//! default. Vanilla's `PatchedDataComponentMap.get` returns null once a
//! component is removed, and a call site's `getOrDefault` then takes its own
//! default, never the prototype value; the readers here must do the same.

use crate::cooldowns::{self, UseCooldown};
use crate::item_data::stack_from_hover;
use crate::item_nbt::item_from_nbt;
use crate::items::{BlocksAttacks, ItemInfo, ItemStack, inline_instrument, instrument_id};
use horizon_core::nbt::Tag;
use std::sync::Arc;

/// A `!minecraft:<component>` patch value: an empty compound, as a command
/// writes it.
fn removed() -> Tag {
    Tag::compound([] as [(&str, Tag); 0])
}

/// `ItemCooldowns.addCooldown` reads `ItemStack.get(USE_COOLDOWN)`: a removal
/// patch leaves it null, so no cooldown starts and the group stays the item's
/// id instead of the removed component's group.
#[test]
fn a_removed_use_cooldown_starts_no_cooldown() {
    let default = UseCooldown {
        seconds: 1.0,
        group: Some(Arc::from("minecraft:ender_pearls")),
    };
    let mut pearl = ItemStack::new("minecraft:ender_pearl", 1);
    assert_eq!(
        cooldowns::component(&pearl, Some(&default)),
        Some(default.clone())
    );
    pearl
        .data_mut()
        .set_other("!minecraft:use_cooldown", Some(removed()));
    assert_eq!(cooldowns::component(&pearl, Some(&default)), None);
    assert_eq!(
        &*cooldowns::group(&pearl, Some(&default)),
        "minecraft:ender_pearl"
    );
}

/// `InstrumentItem.getInstrument` reads `ItemStack.get(INSTRUMENT)`: a removal
/// patch leaves the optional empty, so a goat horn whose prototype names the
/// ponder horn fails to use instead of playing it.
#[test]
fn a_removed_instrument_leaves_the_goat_horn_silent() {
    let mut horn = ItemStack::new("minecraft:goat_horn", 1);
    assert_eq!(
        instrument_id(&horn).as_deref(),
        Some("minecraft:ponder_goat_horn")
    );
    horn.data_mut()
        .set_other("!minecraft:instrument", Some(removed()));
    assert_eq!(instrument_id(&horn), None);
    assert!(inline_instrument(&horn).is_none());
}

/// `PotionContents.applyToLivingEntity` and the potion tooltip call
/// `getOrDefault(POTION_DURATION_SCALE, 1.0F)`: a removal patch makes the
/// tipped arrow's and the lingering potion's effects last unscaled instead of
/// shrinking them by the prototype's scale.
#[test]
fn a_removed_potion_duration_scale_is_unscaled() {
    for item in ["minecraft:tipped_arrow", "minecraft:lingering_potion"] {
        let mut stack = ItemStack::new(item, 1);
        assert!(stack.potion_duration_scale() < 1.0, "{item}");
        stack
            .data_mut()
            .set_other("!minecraft:potion_duration_scale", Some(removed()));
        assert_eq!(stack.potion_duration_scale(), 1.0, "{item}");
    }
}

/// `LivingEntity.getItemBlockingWith` and `Item.use` read
/// `ItemStack.get(BLOCKS_ATTACKS)`: a removal patch leaves it null, so a
/// shield whose prototype carries the component does not block.
#[test]
fn a_removed_blocks_attacks_component_does_not_block() {
    let info = ItemInfo {
        blocks_attacks: Some(BlocksAttacks::bare().clone()),
        components: vec!["minecraft:blocks_attacks".into()],
        ..ItemInfo::default()
    };
    let mut shield = ItemStack::new("minecraft:shield", 1);
    assert!(BlocksAttacks::of_stack(&shield, Some(&info)).is_some());
    shield
        .data_mut()
        .set_other("!minecraft:blocks_attacks", Some(removed()));
    assert!(BlocksAttacks::of_stack(&shield, Some(&info)).is_none());
}

/// `PatchedDataComponentMap.applyPatch` drops a component equal to the item
/// prototype. `Items.FIREWORK_ROCKET` carries `Fireworks(1, [])`, so that
/// exact value belongs to the prototype on a rocket only; on any other item it
/// is the stack's own component and must survive decoding.
#[test]
fn only_the_firework_rocket_elides_a_prototype_equal_fireworks_component() {
    let saved = |item: &str| {
        let tag = horizon_core::nbt::parse_snbt(&format!(
            r#"{{id:"{item}",components:{{"minecraft:fireworks":{{flight_duration:1}}}}}}"#
        ))
        .unwrap()
        .0;
        item_from_nbt(&tag).unwrap()
    };
    assert_eq!(
        saved("minecraft:paper").data().unwrap().fireworks,
        Some((1, Vec::new()))
    );
    assert!(
        saved("minecraft:firework_rocket")
            .data()
            .is_none_or(|d| d.fireworks.is_none())
    );

    // `HoverEvent.ShowItem` decodes through the same item-aware template path.
    let hover = |item: &str| {
        stack_from_hover(&serde_json::json!({
            "id": item,
            "components": {"minecraft:fireworks": {"flight_duration": 1}},
        }))
        .unwrap()
    };
    assert_eq!(
        hover("minecraft:paper").data().unwrap().fireworks,
        Some((1, Vec::new()))
    );
    assert!(
        hover("minecraft:firework_rocket")
            .data()
            .is_none_or(|d| d.fireworks.is_none())
    );
}
