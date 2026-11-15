use super::super::EntityType;
use super::mod_entity_row;

#[test]
fn a_living_mod_types_row_is_its_templates_with_its_attributes() {
    let zombie = r#"{"width":0.6,"height":1.95,"eye_height":1.74,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":20.0,"minecraft:movement_speed":0.23000000417232513,"minecraft:armor":2.0}}"#;
    let row = serde_json::value::RawValue::from_string(zombie.to_owned()).unwrap();
    let ghoul = |attributes: &[(&str, f64)], attachments: &[[f32; 3]]| EntityType {
        id: "ruby:ghoul".into(),
        template: "minecraft:zombie".into(),
        shadow_radius: Some(0.2),
        shadow_strength: Some(0.6),
        item: None,
        damage: 0.0,
        sounds: Default::default(),
        attributes: attributes.iter().map(|&(a, v)| (a.into(), v)).collect(),
        passenger_attachments: attachments.to_vec(),
    };
    let modded = ghoul(&[("minecraft:max_health", 40.0)], &[[0.0, 0.7, -0.1]]);
    let written: serde_json::Value =
        serde_json::from_slice(&mod_entity_row(&row, &modded).unwrap()).unwrap();
    let mut expected: serde_json::Value = serde_json::from_str(zombie).unwrap();
    expected["attributes"]["minecraft:max_health"] = 40.0.into();
    expected["shadow_radius"] = serde_json::json!(0.2);
    expected["shadow_strength"] = serde_json::json!(0.6);
    expected["template"] = "minecraft:zombie".into();
    expected["passenger_attachments"] = serde_json::json!([[0.0, 0.7, -0.1]]);
    assert_eq!(written, expected);
    // the untouched speed keeps the template's text, not a reparsed float.
    let text = String::from_utf8(mod_entity_row(&row, &ghoul(&[], &[])).unwrap()).unwrap();
    assert!(text.contains("0.23000000417232513"), "{text}");
    assert!(!text.contains("passenger_attachments"), "{text}");
    let e = mod_entity_row(&row, &ghoul(&[("minecraft:flying_speed", 1.0)], &[])).unwrap_err();
    assert!(e.contains("not one of minecraft:zombie's"), "{e}");
}
