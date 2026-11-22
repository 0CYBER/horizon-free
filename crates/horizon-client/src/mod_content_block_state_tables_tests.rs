use super::*;

/// The NBT tables fan out like the JSON ones: `cached-collision.nbt`,
/// `occlusion.nbt` and the rest get one row per combination, so `states` and
/// `cached_shapes` stay the same length, and each row carries the properties
/// its combination declares.
#[test]
fn the_nbt_tables_fan_out_with_the_states() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}}}"#;
    let (base, mods) = block_base("block-nbt-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("cached_shapes"), Tag::IntArray(vec![7, 9])),
    ]));
    plan.nbt(&mut tag);
    let Some(Tag::List(states)) = tag.get("states") else {
        panic!("no states")
    };
    let Some(Tag::IntArray(shapes)) = tag.get("cached_shapes") else {
        panic!("no cached_shapes")
    };
    // Two template states times the two combinations of `lit`, and the shapes
    // stay parallel to the states — that is what makes the file load.
    assert_eq!(states.len(), 2 + 4);
    assert_eq!(shapes.len(), states.len());
    // A combination keeps the shape of the template state it came from.
    assert_eq!(shapes, &vec![7, 9, 7, 7, 9, 9]);
    let properties = |i: usize| -> BTreeMap<String, String> {
        let Some(Tag::Compound(c)) = states.get(i) else {
            panic!("row {i} is not a compound")
        };
        let Some(Tag::Compound(p)) = c.get(&JavaString::from("properties")) else {
            panic!("row {i} has no properties")
        };
        p.iter()
            .filter_map(|(k, v)| match v {
                Tag::String(s) => Some((k.to_string_lossy(), s.to_string_lossy())),
                _ => None,
            })
            .collect()
    };
    let mut expected: Vec<String> = Vec::new();
    for axis in ["x", "y"] {
        for lit in ["false", "true"] {
            expected.push(format!("{axis}/{lit}"));
        }
    }
    let seen: Vec<String> = (2..states.len())
        .map(|i| {
            let p = properties(i);
            // The declared properties are the only ones on the row.
            assert_eq!(p.len(), 2, "row {i} is {p:?}");
            let axis = p.get("axis").cloned().unwrap_or_default();
            let lit = p.get("lit").cloned().unwrap_or_default();
            format!("{axis}/{lit}")
        })
        .collect();
    assert_eq!(seen, expected);
    for i in 2..states.len() {
        let Some(Tag::String(id)) = states[i].get("id") else {
            panic!("row {i} has no id")
        };
        assert_eq!(id.to_string_lossy(), "testmod:test");
    }
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A block's shape and its count in `structure-blocks.json` cover every
/// combination too: the table's declared total and the lengths of its lists
/// grow by the fan-out, and every state of the block maps to a shape.
#[test]
fn a_shape_and_the_state_count_cover_every_combination() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "shapes":{"properties":["axis"],"values":{"x":[[0,0,0,1,1,1]],"y":[[0,0,0,0.5,1,1]]}}}"#;
    let (base, mods) = block_base("block-shape-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    // The template's two states, no shapes yet, every vanilla state unmapped.
    // `nbt` grows the arrays to the block's four states and `collision_shapes`
    // writes into them, in the order the real path applies them.
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("shapes"), Tag::List(Vec::new())),
        (
            JavaString::from("cached_shapes"),
            Tag::IntArray(vec![-1, -1]),
        ),
    ]));
    plan.nbt(&mut tag);
    plan.declared_shapes(&mut tag, true).unwrap();
    let Some(Tag::List(entries)) = tag.get("shapes") else {
        panic!("no shapes")
    };
    let Some(Tag::IntArray(mapping)) = tag.get("cached_shapes") else {
        panic!("no cached_shapes")
    };
    // One shape per template state, shared by that state's combinations.
    assert_eq!(entries.len(), 2);
    assert_eq!(mapping.len(), 2 + 4);
    let added = &mapping[2..];
    assert!(added.iter().all(|id| *id >= 0), "{added:?}");
    assert_eq!((added[0], added[1]), (added[0], added[0]));
    assert_eq!((added[2], added[3]), (added[2], added[2]));
    assert_ne!(added[0], added[2]);

    let rows = "[[false,false],[false,false]]";
    let text = format!(r#"{{"rotate":{rows},"mirror":{rows},"full_block":{rows},"states":2}}"#);
    let mut doc: J = serde_json::from_str(&text).unwrap();
    plan.json("worldgen/blocks/structure-blocks.json", &mut doc)
        .unwrap();
    for key in ["rotate", "mirror", "full_block"] {
        let Some(J::Arr(list)) = doc.get(key) else {
            panic!("{key} is not a list")
        };
        assert_eq!(list.len(), 2 + 4, "{key}");
    }
    // The declared total agrees with the lists, which is what a loader checks.
    assert_eq!(doc.get("states"), Some(&J::Num(6.into())));
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}
