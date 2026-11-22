use super::*;

/// A block's states are its template's times the combinations of the state
/// properties it declares, in a fixed order with every default first.
#[test]
fn content_block_state_properties_fan_out_its_states() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"},
        "facing":{"type":"enum","values":["north","south","east"]}}}"#;
    let (base, mods) = block_base("block-states", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    // Two template states times the six combinations, after the template's.
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    assert_eq!(rows.len(), 2 + 12);
    // The default is the first combination of the template's default state.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    let properties = |i: usize| -> BTreeMap<String, String> {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        let Some((_, J::Obj(p))) = state.iter().find(|(k, _)| k == "properties") else {
            panic!("row {i} has no properties")
        };
        p.iter()
            .map(|(k, v)| match v {
                J::Str(s) => (k.clone(), s.clone()),
                _ => panic!("{k} is not a string"),
            })
            .collect()
    };
    // By name `facing` varies slowest, and each one's default comes first.
    assert_eq!(
        properties(2),
        BTreeMap::from([
            ("axis".to_owned(), "x".to_owned()),
            ("facing".to_owned(), "north".to_owned()),
            ("lit".to_owned(), "false".to_owned()),
        ])
    );
    // And the last is the second template state and the last combination.
    assert_eq!(
        properties(13),
        BTreeMap::from([
            ("axis".to_owned(), "y".to_owned()),
            ("facing".to_owned(), "east".to_owned()),
            ("lit".to_owned(), "true".to_owned()),
        ])
    );
    // Every state of the block is a different one: the registry indexes
    // states by their name and properties, so a repeat would be a collision.
    let states: BTreeSet<BTreeMap<String, String>> = (2..14).map(properties).collect();
    assert_eq!(states.len(), 12);
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A declared property whose name sorts before the template's goes in vanilla's order
/// (`StateDefinition`): all the names sorted, the first slowest, so `alpha` varies slower than the
/// template's `axis`. A state table that read the rows in the other order stopped the game at
/// load ("registry state order differs from vanilla's state definition").
#[test]
fn content_block_state_properties_follow_vanilla_name_order() {
    let (base, mods) = block_base(
        "block-name-order",
        &[r#"{"template":"minecraft:test","item":false,
           "state_properties":{"alpha":{"type":"boolean"}}}"#],
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    assert_eq!(rows.len(), 2 + 4);
    let properties = |i: usize| -> (String, String) {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        let Some((_, J::Obj(p))) = state.iter().find(|(k, _)| k == "properties") else {
            panic!("row {i} has no properties")
        };
        let value = |name: &str| match p.iter().find(|(k, _)| k == name) {
            Some((_, J::Str(s))) => s.clone(),
            _ => panic!("row {i} has no {name}"),
        };
        (value("alpha"), value("axis"))
    };
    let own = |a: &str, b: &str| (a.to_owned(), b.to_owned());
    // `alpha` slowest, then `axis`: the first name varies slowest, as in vanilla.
    assert_eq!(
        (2..6).map(properties).collect::<Vec<_>>(),
        [
            own("false", "x"),
            own("false", "y"),
            own("true", "x"),
            own("true", "y")
        ]
    );
    // The default is the template's default state with every declared property at its default.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// `state_order` over the template's and the declared properties together.
#[test]
fn block_states_are_laid_out_in_vanilla_order() {
    let p = |name: &str, len: u32| (name.to_owned(), len);
    // Every declared name after the template's: the template's states times the combinations,
    // as before, so ids of existing blocks do not move.
    let order = state_order(&[p("axis", 2)], &[p("lit", 2)]);
    assert_eq!(order, [(0, 0), (0, 1), (1, 0), (1, 1)]);
    // No declared property, or no template property: the identity, whichever side it is on.
    assert_eq!(state_order(&[p("axis", 3)], &[]), [(0, 0), (1, 0), (2, 0)]);
    assert_eq!(state_order(&[], &[p("lit", 2)]), [(0, 0), (0, 1)]);
    assert_eq!(state_order(&[], &[]), [(0, 0)]);
    // `bites` (4 values) before `facing` (4 values): the declared one slowest.
    let order = state_order(&[p("facing", 4)], &[p("bites", 4)]);
    assert_eq!(order.len(), 16);
    assert!(
        order
            .iter()
            .enumerate()
            .all(|(j, &(s, c))| (s, c) == (j as u32 % 4, j as u32 / 4))
    );
    // Interleaved: `b` between the template's `a` and `c`. a (2), b (2), c (3), a slowest.
    let order = state_order(&[p("a", 2), p("c", 3)], &[p("b", 2)]);
    assert_eq!(order.len(), 12);
    // Template state = a * 3 + c, combo = b.
    assert_eq!(order[0], (0, 0));
    assert_eq!(order[1], (1, 0));
    assert_eq!(order[3], (0, 1));
    assert_eq!(order[6], (3, 0));
    assert_eq!(order[11], (5, 1));
    // Every pair appears once.
    let mut seen = order.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 12);
}

/// The declared default comes first in the run, so its id is the block's
/// default with no arithmetic of its own.
#[test]
fn content_block_state_properties_put_the_declared_default_first() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"facing":{"type":"enum","values":["north","south","east"]}},
      "default_state":{"facing":"south"}}"#;
    let (base, mods) = block_base("block-default-first", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    // Two template states times three values.
    assert_eq!(rows.len(), 2 + 6);
    let facing = |i: usize| -> String {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        match state
            .iter()
            .find(|(k, _)| k == "properties")
            .map(|(_, p)| p)
        {
            Some(p) => match p.get("facing") {
                Some(J::Str(s)) => s.clone(),
                _ => panic!("row {i} has no facing"),
            },
            None => panic!("row {i} has no properties"),
        }
    };
    // The declared default is the block's default, and it comes first.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    assert_eq!(facing(2), "south");
    // The rest keep their declared order, rotated to start at the default.
    assert_eq!(
        (facing(3), facing(4)),
        ("east".to_owned(), "north".to_owned())
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// What a declaration may not do: shadow its template's property, name a
/// value its own property does not have, or ask for more states than a world
/// holds.
#[test]
fn content_block_state_properties_are_validated() {
    // The template's `axis` is not the mod's to redefine.
    let (base, mods) = block_base(
        "block-shadow",
        &[r#"{"template":"minecraft:test","state_properties":{"axis":{"type":"boolean"}}}"#],
    );
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a shadowed template property was accepted"),
    };
    assert!(error.contains("axis is already on"), "{error}");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // A default is one of the values the block declared.
    let (base, mods) = block_base(
        "block-bad-default",
        &[r#"{"template":"minecraft:test",
             "state_properties":{"lit":{"type":"boolean"}},"default_state":{"lit":"maybe"}}"#],
    );
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a default outside a property's values was accepted"),
    };
    assert!(error.contains("is not one of its values"), "{error}");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // Twenty-one booleans are more combinations than a world can hold, and
    // are refused before the product is built.
    let declared: Vec<String> = (0..21)
        .map(|i| format!(r#""p{i}":{{"type":"boolean"}}"#))
        .collect();
    let declaration = format!(
        r#"{{"template":"minecraft:test","item":false,"state_properties":{{{}}}}}"#,
        declared.join(",")
    );
    let (base, mods) = block_base("block-too-many", &[&declaration]);
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("more states than a world holds were accepted"),
    };
    assert!(
        error.contains("more than the") && error.contains("world can hold"),
        "{error}"
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A `default_state` that names only properties the block itself declares
/// asks the template for nothing: `stone` has a single state and no properties
/// at all, and a block that gives it `lit` can still default `lit`.
#[test]
fn a_declared_property_defaults_on_a_template_with_none() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "default_state":{"lit":"true"}}"#;
    let (base, mods) = block_base_of(
        "block-propertyless-default",
        &[declaration],
        r#"{"states":[["minecraft:test",true,true,"",{"id":"minecraft:test"}]],
      "defaults":{"minecraft:test":0}}"#,
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let mut doc: J =
        serde_json::from_str(&fs::read_to_string(base.join("facts.json")).unwrap()).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    // One template state times the two combinations of `lit`.
    assert_eq!(rows.len(), 1 + 2);
    let lit = |i: usize| -> Option<String> {
        let J::Arr(row) = rows.get(i)? else {
            return None;
        };
        let J::Obj(state) = row.get(4)? else {
            return None;
        };
        let (_, properties) = state.iter().find(|(k, _)| k == "properties")?;
        match properties.get("lit") {
            Some(J::Str(s)) => Some(s.clone()),
            _ => None,
        }
    };
    // The declared default opens the block's run, and the other value follows.
    assert_eq!(lit(1).as_deref(), Some("true"));
    assert_eq!(lit(2).as_deref(), Some("false"));
    // The block's default state is the first of its run.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(1.into()))
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

#[test]
fn named_definitions_follow_the_block_state_fanout() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "default_state":{"lit":"true"}}"#;
    let (base, mods) = block_base("named-state-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut models: J = serde_json::from_str(
        r#"{"minecraft:test":{
        "schema":{"axis":{"named":["x","y"]}},
        "states":[{"axis":"x"},{"axis":"y"}]}}"#,
    )
    .unwrap();
    plan.json("model-states.json", &mut models).unwrap();
    let added = models.get("testmod:test").unwrap();
    let expected: J = serde_json::from_str(
        r#"{"axis":{"named":["x","y"]},
        "lit":{"named":["false","true"]}}"#,
    )
    .unwrap();
    assert_eq!(added.get("schema"), Some(&expected));
    let expected: J = serde_json::from_str(
        r#"[
        {"axis":"x","lit":"true"},{"axis":"x","lit":"false"},
        {"axis":"y","lit":"true"},{"axis":"y","lit":"false"}]"#,
    )
    .unwrap();
    assert_eq!(added.get("states"), Some(&expected));

    let mut render: J = serde_json::from_str(
        r#"{"minecraft:test":{
        "default":0,"can_occlude":true,"bars_tag":false,"rule":"never",
        "states":[0,1]}}"#,
    )
    .unwrap();
    plan.json("render-definitions.json", &mut render).unwrap();
    let added = render.get("testmod:test").unwrap();
    let expected: J = serde_json::from_str("[2,3,4,5]").unwrap();
    assert_eq!(added.get("states"), Some(&expected));
    assert_eq!(added.get("default"), Some(&J::Num(2.into())));

    for (file, json) in [
        (
            "landing-definitions.json",
            r#"{"minecraft:test":{"fall":"Block","bounce":false}}"#,
        ),
        (
            "worldgen/blocks/survival-rules.json",
            r#"{"minecraft:test":"Always"}"#,
        ),
    ] {
        let mut doc: J = serde_json::from_str(json).unwrap();
        plan.json(file, &mut doc).unwrap();
        assert_eq!(doc.get("testmod:test"), doc.get("minecraft:test"), "{file}");
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}
