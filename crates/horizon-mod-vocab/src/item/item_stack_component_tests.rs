struct Other;
impl Component for Other {
    type Value = i32;
    fn id(&self) -> &str {
        "test:other"
    }
    fn decode(&self, value: &Tag) -> Option<i32> {
        Number.decode(value)
    }
    fn encode(&self, value: i32) -> Tag {
        Number.encode(value)
    }
}

#[test]
fn applying_components_merges_values_and_removals_without_losing_other_entries() {
    let mut stack = ItemStack::new(ItemKey(0), 16).unwrap();
    stack
        .set_components(
            [
                ("minecraft:custom_name".into(), Tag::String("kept".into())),
                ("minecraft:max_damage".into(), Tag::Int(20)),
                ("!minecraft:lore".into(), Tag::Compound(Compound::new())),
            ]
            .into(),
        )
        .unwrap();
    let lore = Tag::List(vec![Tag::String("line".into())]);
    stack
        .apply_components(
            &[
                ("minecraft:lore".into(), lore.clone()),
                (
                    "!minecraft:max_damage".into(),
                    Tag::Compound(Compound::new()),
                ),
            ]
            .into(),
        )
        .unwrap();
    let expected: Compound = [
        ("minecraft:custom_name".into(), Tag::String("kept".into())),
        ("minecraft:lore".into(), lore),
        (
            "!minecraft:max_damage".into(),
            Tag::Compound(Compound::new()),
        ),
    ]
    .into();
    assert_eq!(stack.components().unwrap().as_nbt(), &expected);
    assert_eq!(stack.count(), 16);

    // A later value cancels a removal; a removal wins over a value in the same patch.
    stack
        .apply_components(&[("minecraft:max_damage".into(), Tag::Int(30))].into())
        .unwrap();
    assert!(
        !stack
            .components()
            .unwrap()
            .as_nbt()
            .contains_key(&"!minecraft:max_damage".into())
    );
    stack
        .apply_components(
            &[
                ("minecraft:max_damage".into(), Tag::Int(40)),
                (
                    "!minecraft:max_damage".into(),
                    Tag::Compound(Compound::new()),
                ),
            ]
            .into(),
        )
        .unwrap();
    assert_eq!(stack.components().unwrap().as_nbt(), &expected);
}

#[test]
fn applying_components_normalizes_damage_and_refuses_bad_patches_atomically() {
    let mut stack = ItemStack::of(ItemKey(0));
    stack.set(&Number, 7).unwrap();
    stack.set_damage(9);
    for (damage, removal, error) in [
        (
            Some(Tag::String("x".into())),
            None,
            StackError::InvalidDamage,
        ),
        (Some(Tag::Int(-1)), None, StackError::InvalidDamage),
        (Some(Tag::Long(i64::MAX)), None, StackError::InvalidDamage),
        (None, Some(Tag::Int(0)), StackError::InvalidDamage),
        (
            Some(Tag::Int(0)),
            Some(Tag::Compound(Compound::new())),
            StackError::ConflictingDamage,
        ),
    ] {
        let mut incoming: Compound = [("test:number".into(), Tag::Int(8))].into();
        if let Some(value) = damage {
            incoming.insert("minecraft:damage".into(), value);
        }
        if let Some(value) = removal {
            incoming.insert("!minecraft:damage".into(), value);
        }
        let before = stack.clone();
        assert_eq!(stack.apply_components(&incoming), Err(error));
        assert_eq!(stack, before);
    }
    stack
        .apply_components(&[("minecraft:damage".into(), Tag::Short(0))].into())
        .unwrap();
    assert_eq!(stack.damage_component(), Some(0));
    assert_eq!(stack.get(&Number), Some(7));
    assert!(
        !stack
            .components()
            .unwrap()
            .as_nbt()
            .contains_key(&"minecraft:damage".into())
    );
    stack
        .apply_components(&[("!minecraft:damage".into(), Tag::Compound(Compound::new()))].into())
        .unwrap();
    assert_eq!(stack.damage_component(), None);
    assert!(
        stack
            .components()
            .unwrap()
            .as_nbt()
            .contains_key(&"!minecraft:damage".into())
    );
    stack
        .apply_components(&[("minecraft:damage".into(), Tag::Long(u32::MAX.into()))].into())
        .unwrap();
    assert_eq!(stack.damage_component(), Some(u32::MAX));
    assert!(
        !stack
            .components()
            .unwrap()
            .as_nbt()
            .contains_key(&"!minecraft:damage".into())
    );
}

#[test]
fn copying_a_component_preserves_own_value_removal_and_inheritance() {
    let mut source = ItemStack::of(ItemKey(0));
    let mut target = ItemStack::new(ItemKey(1), 16).unwrap();
    target.set(&Other, 99).unwrap();
    for id in ["test:number", "minecraft:damage"] {
        if id == "minecraft:damage" {
            source.set_damage(0);
        } else {
            source.set(&Number, 42).unwrap();
        }
        target.copy_component_from(&source, id);
        if id == "minecraft:damage" {
            assert_eq!(target.damage_component(), Some(0));
        } else {
            assert_eq!(target.get(&Number), Some(42));
        }
        source
            .apply_components(
                &[(
                    format!("!{id}").as_str().into(),
                    Tag::Compound(Compound::new()),
                )]
                .into(),
            )
            .unwrap();
        target.copy_component_from(&source, id);
        assert!(
            target
                .components()
                .unwrap()
                .as_nbt()
                .contains_key(&format!("!{id}").as_str().into())
        );
        assert_eq!(target.damage_component(), None);
        assert_eq!(target.get(&Number), None);

        // An unpatched source inherits its item default; copying it must clear, not remove.
        target.copy_component_from(&ItemStack::of(ItemKey(0)), id);
        assert!(
            !target
                .components()
                .unwrap()
                .as_nbt()
                .contains_key(&format!("!{id}").as_str().into())
        );
        target.copy_component_from(&source, id);
        if id == "minecraft:damage" {
            source.set_damage(5);
        } else {
            source.set(&Number, 6).unwrap();
        }
        target.copy_component_from(&source, id);
        assert!(
            !target
                .components()
                .unwrap()
                .as_nbt()
                .contains_key(&format!("!{id}").as_str().into())
        );
        target.copy_component_from(&ItemStack::of(ItemKey(0)), id);
        assert_eq!(target.damage_component(), None);
        assert_eq!(target.get(&Number), None);
        assert_eq!(target.get(&Other), Some(99));
        assert_eq!(target.item(), ItemKey(1));
        assert_eq!(target.count(), 16);
    }
}

#[test]
fn copying_an_id_that_is_not_a_component_id_changes_nothing() {
    let mut source = ItemStack::of(ItemKey(0));
    source.clear_damage();
    let mut target = ItemStack::new(ItemKey(1), 16).unwrap();
    target.set_damage(1);
    // The removal marker is not a component id: it must not be read as a value and refused.
    target.copy_component_from(&source, "!minecraft:damage");
    target.copy_component_from(&source, "Damage");
    assert_eq!(target.damage_component(), Some(1));
}
