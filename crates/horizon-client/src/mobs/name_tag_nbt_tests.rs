//! A named mob's `CustomName` in its save: `NameTagItem.interactLivingEntity`
//! sets the tag's whole `custom_name` component on the mob, and
//! `Entity.saveWithoutId` writes that component back with
//! `ComponentSerialization.CODEC` -- so a name tag that carried
//! `{"text":"Bob","color":"red"}` leaves `CustomName: {text: "Bob", color: "red"}`
//! behind, not plain text.
use super::super::{Mob, MobType, Part, Species};
use crate::entity_nbt::{mob_from_nbt, mob_to_nbt, name_text};
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;
use std::sync::Arc;

/// A cow type, without the prepared resources a test machine may not have.
fn kind() -> Arc<MobType> {
    let part = Part {
        name: "root".into(),
        pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        polygons: Vec::new(),
        children: Vec::new(),
    };
    Arc::new(MobType {
        id: "minecraft:cow".into(),
        species: Species::Cow,
        width: 0.9,
        height: 1.4,
        eye_height: 1.3,
        category: "creature".into(),
        max_health: 10.0,
        speed: 0.2,
        flying_speed: 0.02,
        attack_damage: 1.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 0.0,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Default::default(),
        item_stack_limits: Default::default(),
        gravity: 0.08,
        safe_fall_distance: 3.0,
        fall_damage_multiplier: 1.0,
        water_efficiency: 0.0,
        water_slowdown: 0.8,
        air_speed: 0.02,
        baby_scale: 0.5,
        head_rot_speed: 10.0,
        max_head_x_rot: 40.0,
        max_head_y_rot: 75.0,
        ambient_interval: 80,
        flags: Default::default(),
        model: Arc::new(part),
        layers: Default::default(),
        texture: String::new(),
        brain: super::super::Brain::Passive,
        anim: super::super::Anim::Humanoid { zombie_arms: false },
        sounds: ["entity.cow.hurt", "entity.cow.hurt", "entity.cow.death"],
        food: Default::default(),
        tempt: Default::default(),
        ..(*crate::mobs::sensor_scan_tests::kind()).clone()
    })
}

#[test]
fn a_named_mobs_custom_name_saves_its_whole_component() {
    // The component `NameTagItem` puts on the mob, style and all.
    let styled = Tag::compound([
        ("text", Tag::String("Bob".into())),
        ("color", Tag::String("red".into())),
    ]);
    let mut m = Mob::new(1, kind(), Vec3::new(0.0, 64.0, 0.0), 0.0);
    m.custom_name = Some("Bob".into());
    m.custom_name_text = Some(name_text(&styled));

    let saved = mob_to_nbt(&m, [0; 4], None, None, |_| None);
    let saved_name = saved.get("CustomName").expect("CustomName is saved");
    // `/data get entity @e[type=minecraft:cow,limit=1] CustomName` keeps the
    // colour the tag gave it.
    assert_eq!(
        name_text(saved_name),
        name_text(&styled),
        "the colour survives the save"
    );
    assert_ne!(
        name_text(saved_name),
        name_text(&Tag::String("Bob".into())),
        "the name was saved as plain text"
    );
    // Reading the save back names the mob the same way again.
    let types = std::collections::HashMap::from([("minecraft:cow".to_owned(), kind())]);
    let back = mob_from_nbt(&saved, &types, 2, None).expect("the save loads");
    assert_eq!(back.custom_name_text, Some(name_text(&styled)));

    // A name with no style is the plain string `ComponentSerialization`
    // collapses it to.
    m.custom_name_text = None;
    let plain = mob_to_nbt(&m, [0; 4], None, None, |_| None);
    assert_eq!(plain.get("CustomName"), Some(&Tag::String("Bob".into())));

    // Renaming "Bob" with a red "Bob" tag changes the style, not the text.
    m.custom_name_text = Some(name_text(&styled));
    let restyled = mob_to_nbt(&m, [0; 4], Some(&plain), None, |_| None);
    assert_eq!(
        name_text(restyled.get("CustomName").expect("CustomName is saved")),
        name_text(&styled),
        "a same-text rename keeps its new colour"
    );
}

/// A mob named with plain text only (no component, as a mod names it) is not
/// renamed by its own save: the saved `CustomName`, a string or a styled
/// compound with the same text, stays as it is.
#[test]
fn a_plain_name_matches_its_saved_text() {
    let mut m = Mob::new(1, kind(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    m.custom_name = Some("Bob".into());
    m.custom_name_text = None;
    let plain = Tag::String("Bob".into());
    assert!(!m.custom_name_changed(Some(&plain)), "same plain text");
    let styled = Tag::compound([
        ("text", Tag::String("Bob".into())),
        ("color", Tag::String("red".into())),
    ]);
    assert!(!m.custom_name_changed(Some(&styled)), "same text, styled");
    let other = Tag::String("Alice".into());
    assert!(m.custom_name_changed(Some(&other)), "another name");
    assert!(m.custom_name_changed(None), "no saved name");
}
