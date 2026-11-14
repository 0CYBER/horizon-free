//! Byte and independently typed field-schema regressions for derived and checked codecs.
use super::*;
use crate::*;
use horizon_core::nbt::{Compound, JavaString};

pub(super) fn install_layout_resources() {
    let _ = crate::wire::install_resource_access(crate::wire::ResourceAccess {
        intern: |value| {
            if value.len() > crate::wire::MAX_STATIC_STRING_BYTES {
                return Err(WireError::Invalid("static string too long"));
            }
            Ok(Box::leak(value.to_owned().into_boxed_str()))
        },
        store: |value| {
            let provider = value
                .downcast::<std::sync::Arc<dyn std::any::Any + Send + Sync>>()
                .map_err(|_| WireError::Invalid("no resource context"))?;
            if provider.downcast_ref::<u32>() != Some(&73) {
                return Err(WireError::Invalid("no resource context"));
            }
            Ok(crate::wire::Id(0x100000007))
        },
        load: |_, _| Err(WireError::Invalid("no resource context")),
        refused: |_| {},
    });
}

fn fields<T: Wire, F: Wire + std::fmt::Debug + PartialEq>(value: T, expected: F) {
    install_layout_resources();
    assert_eq!(decode::<T, F>(value), expected);
}
fn id(s: &str) -> Identifier {
    Identifier::parse(s).unwrap()
}

#[test]
fn checked_records_have_declared_field_bytes() {
    fields(
        id("example:gear"),
        ("example".to_owned(), "gear".to_owned()),
    );
    fields(
        ConfigPath::new("gear/options.toml").unwrap(),
        ("gear/options.toml".to_owned(),),
    );
    fields(
        SpawnerKey::new("example:spawn").unwrap(),
        ("example:spawn".to_owned(),),
    );
    fields(
        LevelKey::try_from("example:moon".to_owned()).unwrap(),
        ("example:moon".to_owned(),),
    );
    fields(
        FileToIdConverter::new("recipes", ".json").unwrap(),
        ("recipes".to_owned(), ".json".to_owned()),
    );
    fields(
        ReloadResourceError::from((id("example:bad"), "broken".to_owned())),
        (id("example:bad"), "broken".to_owned()),
    );
    fields(JavaString(vec![65, 0xd800, 90]), (vec![65u16, 0xd800, 90],));
    let mut nbt = Compound::new();
    nbt.insert("answer".into(), Tag::Int(42));
    fields(Components::from(nbt.clone()), (nbt.clone(),));
    let _keys = Keys::new();
    let key = ItemKey::from_slot(0).unwrap();
    let mut stack = ItemStack::new(key, 7).unwrap();
    stack.set_damage(11);
    fields(stack.clone(), (key, 7u32, Some(11u32), None::<Components>));
    fields(
        ItemResource::of(&stack),
        (ItemStack::new(key, 1)
            .map(|mut s| {
                s.set_damage(11);
                s
            })
            .unwrap(),),
    );
    let fluid = FluidStack::new(FluidKey::from_slot(0).unwrap(), 123);
    fields(
        FluidResource::of(&fluid),
        (FluidStack::new(FluidKey::from_slot(0).unwrap(), 1),),
    );
    let block = BlockKey::from_slot(0).unwrap();
    let state: BlockState = BlockStateRecord::new(block, 9).into();
    fields(state, (block, 9u16));
    fields(BlockStateRange::new(state, 3).unwrap(), (state, 3u32));
    fields(
        FeatureFlags::new(vec!["example:a".to_owned(), "example:b".to_owned()].into()),
        (vec!["example:a".to_owned(), "example:b".to_owned()],),
    );
    fields(
        MobGoalEntry {
            priority: -3,
            id: "example:goal",
            running: true,
        },
        (-3i32, "example:goal".to_owned(), true),
    );
    fields(
        Corners {
            width: 13.0,
            height: 17.0,
            used: [1.0, 2.0, 3.0, 4.0],
        },
        (13.0f32, 17.0f32, [1.0f32, 2.0, 3.0, 4.0]),
    );
    fields(
        BlockPos {
            x: -3,
            y: 71,
            z: 19,
        },
        (-3i32, 71i32, 19i32),
    );
    fields(ChunkPos { x: -8, z: 11 }, (-8i32, 11i32));
    fields(
        BoundingBox {
            min: [-3, 4, 5],
            max: [6, 17, 28],
        },
        ([-3i32, 4, 5], [6i32, 17, 28]),
    );
    fields(
        serde_json::json!({"a": [1, true]}),
        ("{\"a\":[1,true]}".to_owned(),),
    );
    let map = serde_json::Map::from_iter([("b".to_owned(), serde_json::json!(2))]);
    fields(map, ("{\"b\":2}".to_owned(),));
    fields(
        LegacyRandom::from((123u64, Some(0.75f64))),
        (123u64, Some(0.75f64)),
    );
}

#[test]
fn resource_decoders_normalize_non_unit_wire_quantities() {
    let _keys = Keys::new();
    // Item key slot 0, count 7, no damage or component patch.
    let item: ItemResource = Reader::decode(&[0, 0, 0, 0, 7, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(item.as_stack().count(), 1);
    let mut out = Writer::new();
    out.put(&item);
    assert_eq!(out.bytes(), &[0, 0, 0, 0, 1, 0, 0, 0, 0, 0]);
    // Resolved-fluid tag 0, fluid slot 0, amount 7, no components.
    let fluid: FluidResource = Reader::decode(&[0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0]).unwrap();
    out.clear();
    out.put(&fluid);
    assert_eq!(out.bytes(), &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
}

#[test]
fn empty_feature_flag_storage_encodes_the_effective_vanilla_default() {
    let flags = FeatureFlags::new(std::sync::Arc::from([]));
    assert!(flags.contains("minecraft:vanilla"));
    assert!(flags.contains("vanilla"));
    assert_eq!(flags.iter().collect::<Vec<_>>(), ["minecraft:vanilla"]);
    // Independent bytes: one flag, then the 17-byte UTF-8 resource id.
    let expected = b"\x01\0\0\0\x11\0\0\0minecraft:vanilla";
    let mut out = Writer::new();
    out.put(&flags);
    assert_eq!(out.bytes(), expected);
    let decoded: FeatureFlags = Reader::decode(expected).unwrap();
    assert_eq!(decoded.iter().collect::<Vec<_>>(), ["minecraft:vanilla"]);
    // A zero-entry input still takes the constructor's empty-default path,
    // and re-encoding it emits the same effective list as before the migration.
    let empty: FeatureFlags = Reader::decode(&[0, 0, 0, 0]).unwrap();
    out.clear();
    out.put(&empty);
    assert_eq!(out.bytes(), expected);
}

#[test]
fn handle_records_have_declared_field_bytes() {
    let epoch = WorldEpoch::new(47);
    let level = LevelRef::new(epoch, DimensionKey::OVERWORLD);
    let entity = EntityRef::new(epoch, 29);
    let player = crate::__private::game::player_ref(entity, 0x1234);
    let pos = BlockPos { x: 7, y: -8, z: 9 };
    fields(level, (epoch, DimensionKey::OVERWORLD, 0u32));
    let reopened = LevelRef::with_generation(epoch, DimensionKey::NETHER, 5);
    fields(reopened, (epoch, DimensionKey::NETHER, 5u32));
    fields(ChunkRef::new(level, [3, -4]), (level, [3i32, -4]));
    fields(RaidRef::new(level, 53), (level, 53i32));
    fields(RaidRef::new(level, -1), (level, -1i32));
    fields(WaterTicket::new(level, 59), (level, 59u64));
    fields(player, (entity, 0x1234u128));
    fields(
        GlobalPos::new(id("example:moon"), pos),
        (id("example:moon"), pos),
    );
    fields(
        BossBarRef {
            epoch,
            uuid: 0x5678,
        },
        (epoch, 0x5678u128),
    );
    fields(
        MenuRef {
            player,
            window: -2,
            generation: 91,
        },
        (player, -2i32, 91u64),
    );
    fields(
        BlockEntityRef {
            level,
            position: [7, -8, 9],
            generation: 83,
        },
        (level, [7i32, -8, 9], 83u64),
    );
    fields(UnspawnedMobRef::new(epoch, 61), (epoch, 61u64));
    fields(ModelRef::from_parts(31, 7).unwrap(), (31u64, 7u32));
    fields(InstanceGroupRef::from_parts(37, 9).unwrap(), (37u64, 9u32));
    fields(RenderPipelineRef::from_parts(41, 5).unwrap(), (41u64, 5u32));
    fields(AttachmentHolder::Entity(entity), (0u32, entity));
    fields(AttachmentHolder::Player(player), (1u32, player));
    fields(
        AttachmentHolder::BlockEntity(BlockEntityRef {
            level,
            position: [7, -8, 9],
            generation: 83,
        }),
        (
            2u32,
            BlockEntityRef {
                level,
                position: [7, -8, 9],
                generation: 83,
            },
        ),
    );
    fields(
        AttachmentHolder::Chunk(ChunkRef::new(level, [3, -4])),
        (3u32, ChunkRef::new(level, [3, -4])),
    );
    fields(AttachmentHolder::Level(level), (4u32, level));
}

#[test]
fn payload_enum_cases_have_declared_field_bytes() {
    fields(
        SpawnerEntry::Builtin(SpawnerKind::Cat),
        (0u32, SpawnerKind::Cat),
    );
    fields(
        SpawnerEntry::Mod(SpawnerKey::new("example:spawn").unwrap()),
        (1u32, SpawnerKey::new("example:spawn").unwrap()),
    );
    fields(ReductionChange::Add(1.25), (0u8, 1.25f32));
    fields(ReductionChange::Scale(2.5), (1u8, 2.5f32));
    fields(ReductionChange::AtLeast(3.75), (2u8, 3.75f32));
    fields(ReductionChange::AtMost(4.5), (3u8, 4.5f32));
    let pos = [2i32, -4, 6];
    fields(
        crate::client::particles::PositionSource::Block([2, -4, 6]),
        (0u32, pos),
    );
    fields(
        crate::client::particles::PositionSource::Entity {
            id: 19,
            y_offset: 1.75,
        },
        (1u32, 19i32, 1.75f32),
    );
    fields(
        crate::client::particles::PositionSource::Entity {
            id: -1,
            y_offset: 1.5,
        },
        (1u32, -1i32, 1.5f32),
    );
    fields(TicketOwner::Block([1, -2, 3]), (0u32, [1i32, -2, 3]));
    fields(TicketOwner::Entity(0x1234), (1u32, 0x1234u128));
    fields(Holder::Entity(17), (0u32, 17u32));
    fields(Holder::Player(23), (1u32, 23u128));
    fields(Holder::BlockEntity([1, -2, 3]), (2u32, [1i32, -2, 3]));
    fields(Holder::Chunk([7, -9]), (3u32, [7i32, -9]));
    fields(
        LevelDimension::Custom(LevelKey::try_from("example:moon".to_owned()).unwrap()),
        (3u32, LevelKey::try_from("example:moon".to_owned()).unwrap()),
    );
    fields(Tag::Int(-123), (3u32, -123i32));
    fields(Tag::LongArray(vec![7, -19]), (12u32, vec![7i64, -19]));
}

// Registration tables are scoped to this test thread, just like registrar callbacks.
struct Keys(Option<std::rc::Rc<std::cell::RefCell<horizon_mod_runtime::registry_keys::Requests>>>);
impl Keys {
    fn new() -> Self {
        let mut requests = horizon_mod_runtime::registry_keys::Requests::default();
        requests.items.request("example", "example:gear").unwrap();
        requests.blocks.request("example", "example:block").unwrap();
        requests.fluids.request("example", "example:fluid").unwrap();
        requests
            .block_entity_types
            .request("example", "example:machine")
            .unwrap();
        requests
            .particles
            .request("example", "example:particle")
            .unwrap();
        Self(
            crate::parts::REGISTERING_KEYS.with(|keys| {
                keys.replace(Some(std::rc::Rc::new(std::cell::RefCell::new(requests))))
            }),
        )
    }
}
impl Drop for Keys {
    fn drop(&mut self) {
        crate::parts::REGISTERING_KEYS.with(|keys| keys.replace(self.0.take()));
    }
}

#[test]
fn snapshot_and_reload_records_have_declared_field_bytes() {
    let _keys = Keys::new();
    let block = BlockKey::from_slot(0).unwrap();
    fields(
        StatePredicate::from(StatePredicateRecord::new(
            StatePredicateTarget::Block(block),
            vec![],
        )),
        (
            StatePredicateTarget::Block(block),
            Vec::<StateConstraint>::new(),
        ),
    );
    let values = DamageValues {
        original: 31.0,
        new_damage: 29.0,
        blocked: 2.0,
        shield_damage: 3.0,
        post_attack_invulnerability_ticks: 17,
        should_cause_side_effects: true,
        reductions: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        reduction_modifiers: vec![(Reduction::Armor, ReductionChange::Scale(0.5))],
        entered_pre: 23.0,
        inflicted: 19.0,
        health_damage: 11.0,
    };
    fields(DamageContainer::from(values.clone()), (values,));
    let revision = crate::item::containers::ModRevision::from_snapshot(31, 47);
    fields(revision, (31u64, 47u64));
    let snapshot = crate::item::containers::ModBlockEntitySnapshot {
        id: "example:machine".into(),
        fields: crate::item::containers::ModBlockEntityFields {
            nbt: Compound::new(),
            items: vec![None],
            data: vec![19, -3],
        },
        revision,
        client_update_fields: Some(Compound::new()),
    };
    let (decoded,): (crate::item::containers::ModBlockEntitySnapshot,) =
        decode(ModBlockEntity::from(snapshot));
    assert_eq!(decoded.id, "example:machine");
    assert_eq!(decoded.fields.nbt, Compound::new());
    assert_eq!(decoded.fields.items, vec![None]);
    assert_eq!(decoded.fields.data, vec![19, -3]);
    assert_eq!(decoded.revision.identity(), 31);
    assert_eq!(decoded.revision.changes(), 47);
    assert_eq!(decoded.client_update_fields, Some(Compound::new()));
    let files = std::collections::BTreeMap::from([(id("example:recipe"), vec![3u8, 5, 7])]);
    fields(
        crate::world::reload::ReloadFiles::new(files.clone()),
        (files.clone(),),
    );
    let (server,): (crate::world::reload::ReloadFiles,) = decode(ServerReloadContext::new(
        crate::world::reload::ReloadFiles::new(files.clone()),
    ));
    assert_eq!(server.0, files);
    let (client,): (crate::world::reload::ReloadFiles,) = decode(ClientReloadContext::new(
        crate::world::reload::ReloadFiles::new(files.clone()),
    ));
    assert_eq!(client.0, files);
    fields(
        AttachmentPolicy {
            copy_on_death: true,
            sync: false,
        },
        (true, false),
    );
    fields(
        AttachmentForm {
            encoded: Some(Compound::new()),
            policy: AttachmentPolicy {
                copy_on_death: false,
                sync: true,
            },
            write: AttachmentWrite::Set,
        },
        (
            Some(Compound::new()),
            AttachmentPolicy {
                copy_on_death: false,
                sync: true,
            },
            AttachmentWrite::Set,
        ),
    );
    let token = AttachmentWriteToken::from(
        horizon_mod_runtime::attachments::index::AttachmentOwner::from_parts(
            AttachmentHolder::Entity(EntityRef::new(WorldEpoch::new(43), 29)),
            71,
        ),
    );
    fields(
        token,
        (
            AttachmentHolder::Entity(EntityRef::new(WorldEpoch::new(43), 29)),
            71u64,
        ),
    );
}

fn decode<T: Wire, F: Wire>(value: T) -> F {
    let mut out = Writer::new();
    out.put(&value);
    let declared = T::__field_layouts().expect("compound codec must declare its fields");
    let expected = F::__tuple_layouts().expect("expected fields must be a tuple");
    let actual = if declared.first().is_some_and(|(name, _)| *name == "tag") {
        let tag_layout = declared[0].1;
        let mut reader = Reader::new(out.bytes());
        let tag = if tag_layout == u8::layout(&mut Vec::new()) {
            u32::from(reader.take::<u8>().unwrap())
        } else {
            assert_eq!(
                tag_layout,
                u32::layout(&mut Vec::new()),
                "unsupported enum tag width"
            );
            reader.take::<u32>().unwrap()
        };
        let prefix = format!("{tag}:");
        let unit = <()>::layout(&mut Vec::new());
        std::iter::once(tag_layout)
            .chain(declared.iter().skip(1).filter_map(|(name, layout)| {
                (name.starts_with(&prefix) && *layout != unit).then_some(*layout)
            }))
            .collect::<Vec<_>>()
    } else {
        declared.into_iter().map(|(_, layout)| layout).collect()
    };
    assert_eq!(
        actual,
        expected,
        "{} field schema differs from the independently typed expected tuple",
        std::any::type_name::<T>()
    );
    Reader::decode::<F>(out.bytes()).unwrap()
}

#[test]
fn registration_and_attribute_records_have_declared_field_bytes() {
    fields(
        PackageInfo::new(
            "example".into(),
            "1.2.3".into(),
            PackageKind::Code,
            Side::Both,
            vec!["example".into()],
            vec![PackageDependency::new(
                "library".into(),
                "^1".into(),
                crate::DependencyKind::Optional,
                crate::DependencyOrder::After,
                crate::Side::Both,
                None,
                None,
            )],
        ),
        (
            "example".to_owned(),
            "1.2.3".to_owned(),
            PackageKind::Code,
            Side::Both,
            vec!["example".to_owned()],
            // Nested tuples flatten into one field list.
            (
                vec![PackageDependency::new(
                    "library".into(),
                    "^1".into(),
                    crate::DependencyKind::Optional,
                    crate::DependencyOrder::After,
                    crate::Side::Both,
                    None,
                    None,
                )],
                PackageDetails::default(),
            ),
        ),
    );
    fields(PropertyValue::Integer(3), (1u32, 3i64));
    let dependency = PackageDependency::new(
        "library".into(),
        "^1".into(),
        crate::DependencyKind::Optional,
        crate::DependencyOrder::After,
        crate::Side::Both,
        None,
        None,
    );
    fields(
        dependency.clone(),
        (
            "library".to_owned(),
            "^1".to_owned(),
            crate::DependencyKind::Optional,
            crate::DependencyOrder::After,
            crate::Side::Both,
            (None::<String>, None::<String>),
        ),
    );
    // Independently check actual enum discriminants, not just a round trip
    // through the same enum codec: Optional = 1, After = 0, Both = 1.
    let mut bytes = Writer::new();
    bytes.put(&dependency);
    let raw = Reader::decode::<(
        String,
        String,
        u32,
        u32,
        u32,
        (Option<String>, Option<String>),
    )>(bytes.bytes())
    .unwrap();
    assert_eq!(
        raw,
        (
            "library".to_owned(),
            "^1".to_owned(),
            1u32,
            0u32,
            1u32,
            (None::<String>, None::<String>)
        )
    );
    let details = PackageDetails {
        name: "Example".into(),
        icon_path: Some("icon.png".into()),
        runtime: Some(PackageRuntime::Native),
        properties: [("ratio".into(), PropertyValue::Float(0.5))].into(),
        ..Default::default()
    };
    fields(
        details.clone(),
        (
            (
                "Example".to_owned(),
                String::new(),
                Vec::<String>::new(),
                String::new(),
                None::<String>,
            ),
            (
                None::<String>,
                Some("icon.png".to_owned()),
                Some(PackageRuntime::Native),
                details.properties.clone(),
            ),
        ),
    );
    let spawners = vec![
        SpawnerEntry::Builtin(SpawnerKind::Cat),
        SpawnerEntry::Mod(SpawnerKey::new("example:spawn").unwrap()),
    ];
    fields(
        CustomSpawners::try_from(spawners.clone()).unwrap(),
        (spawners,),
    );
    let modifier = AttributeModifier::new(id("example:boost"), 1.75, AttributeOperation::AddValue);
    let entry = ItemAttributeModifier::new(
        id("minecraft:generic.attack_damage"),
        modifier.clone(),
        EquipmentSlotGroup::MainHand,
    );
    fields(
        entry.clone(),
        (
            id("minecraft:generic.attack_damage"),
            modifier,
            EquipmentSlotGroup::MainHand,
        ),
    );
    fields(
        ItemAttributeModifiers {
            entries: vec![entry.clone()],
            changed: true,
        },
        (vec![entry], true),
    );
    fields(
        ItemEnchantments {
            entries: vec![(id("minecraft:sharpness"), 3)],
            changed: true,
        },
        (vec![(id("minecraft:sharpness"), 3u32)], true),
    );
    let (rows,): (Vec<(AttachmentWriteToken, Vec<u8>)>,) = decode(AttachmentBatch::from(
        horizon_mod_runtime::attachments::AttachmentBatch::<AttachmentHolder>::empty(),
    ));
    assert!(rows.is_empty());
    fields(
        ItemPlace::Mod {
            provider: "example:bag",
            key: 37,
        },
        (8u32, "example:bag".to_owned(), 37u64),
    );
}

#[test]
fn particle_record_has_declared_field_bytes() {
    let _keys = Keys::new();
    let ty = ParticleKey::from_slot(0).unwrap();
    let particle = ParticleOptions {
        ty,
        block: Some(("example:block".into(), 7)),
        color: [0.1, 0.2, 0.3, 0.4],
        scale: 2.5,
        to_color: [0.5, 0.6, 0.7],
        trail: Some(([1.0, 2.0, 3.0], 19)),
        item: Some(vec!["example:item".into()]),
        item_stack: ItemStack::new(ItemKey::from_slot(0).unwrap(), 7),
        geyser: Some((11, 0.75)),
        vibration: Some((
            crate::client::particles::PositionSource::Entity {
                id: 13,
                y_offset: 1.5,
            },
            [4.0, 5.0, 6.0],
            23,
        )),
        legacy_entity_effect: Some(true),
        legacy_trail_lifetime: true,
    };
    // Nested tuples add no framing: two groups allow all twelve fields to be decoded
    // with the same declared types despite Wire's six-element tuple limit.
    fields(
        particle,
        (
            (
                ty,
                Some(("example:block".to_owned(), 7u32)),
                [0.1f32, 0.2, 0.3, 0.4],
                2.5f32,
                [0.5f32, 0.6, 0.7],
                Some(([1.0f64, 2.0, 3.0], 19i32)),
            ),
            (
                Some(vec!["example:item".to_owned()]),
                ItemStack::new(ItemKey::from_slot(0).unwrap(), 7),
                Some((11i32, 0.75f32)),
                Some((
                    crate::client::particles::PositionSource::Entity {
                        id: 13,
                        y_offset: 1.5,
                    },
                    [4.0f64, 5.0, 6.0],
                    23i32,
                )),
                Some(true),
                true,
            ),
        ),
    );
    let kind = BlockEntityTypeKey::from_slot(0).unwrap();
    fields(BlockEntityPartError::VanillaType { kind }, (0u32, kind));
    fields(
        BlockEntityPartError::Duplicate {
            kind,
            part: "example::Machine",
        },
        (1u32, kind, "example::Machine".to_owned()),
    );
}

#[test]
fn saved_attachment_batch_fields_keep_scoped_identity_and_nbt() {
    use horizon_mod_runtime::attachments::{
        AttachmentBatch as Batch, AttachmentEntry,
        index::{AttachmentOwner, AttachmentSnapshot},
    };
    let holder = AttachmentHolder::Entity(EntityRef::new(WorldEpoch::new(43), 29));
    let mut saved = Compound::new();
    saved.insert("value".into(), Tag::Int(123));
    let row = AttachmentOwner::from_parts(holder, 71).into_row(std::sync::Arc::new(
        AttachmentEntry::from_saved(saved.clone()),
    ));
    let batch = AttachmentBatch::from(Batch::new(std::sync::Arc::new(
        AttachmentSnapshot::from_rows(vec![row]),
    )));
    let (rows,): (Vec<(AttachmentWriteToken, Vec<u8>)>,) = decode(batch);
    assert_eq!(rows.len(), 1);
    let (token, bytes) = rows.into_iter().next().unwrap();
    let (identity, generation): (AttachmentHolder, u64) = decode(token);
    assert_eq!(identity, holder);
    assert_eq!(generation, 71);
    let (tag, used) =
        horizon_core::nbt::read_network(&bytes, horizon_core::nbt::Limits::default()).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(tag, Tag::Compound(saved));
}

#[test]
fn remaining_enum_payload_fields_keep_tags_and_widths() {
    fields(
        NeighbourUpdate::NeighboursExcept(Direction::North),
        (2u32, Direction::North),
    );
    fields(
        AttachmentRowError::Value(AttachmentError::Decode),
        (0u32, AttachmentError::Decode),
    );
    fields(Tag::Byte(-7), (1u32, -7i8));
    fields(Tag::Short(-1234), (2u32, -1234i16));
    fields(Tag::Long(-1234567), (4u32, -1234567i64));
    fields(Tag::Float(1.25f32.to_bits()), (5u32, 1.25f32));
    fields(Tag::Double((-3.75f64).to_bits()), (6u32, -3.75f64));
    fields(Tag::ByteArray(vec![255, 2, 3]), (7u32, vec![255u8, 2, 3]));
    fields(
        Tag::String(JavaString(vec![65, 0xd800])),
        (8u32, JavaString(vec![65, 0xd800])),
    );
    fields(Tag::List(vec![Tag::Int(17)]), (9u32, vec![Tag::Int(17)]));
    fields(Tag::Compound(Compound::new()), (10u32, Compound::new()));
    fields(Tag::IntArray(vec![-4, 7]), (11u32, vec![-4i32, 7]));
}

#[test]
fn newtype_widths_and_unit_enum_tags_match_their_layouts() {
    let _keys = Keys::new();
    fields(BlockKey::from_slot(0).unwrap(), (0u32,));
    fields(ItemKey::from_slot(0).unwrap(), (0u32,));
    fields(FluidKey::from_slot(0).unwrap(), (0u32,));
    // Issuance is platform-owned; this layer pins the encoded 64-bit field only.
    fields(crate::UnresolvedFluid(0x100000003), (0x100000003u64,));
    fields(WorldEpoch::new(47), (47u64,));
    fields(TickId::new(19), (19u64,));
    fields(SoundRef { id: 71 }, (71u64,));
    fields(crate::__private::biome_id(11), (11u32,));
    fields(crate::__private::mod_color_resolver(13), (13u32,));
    fields(crate::wire::wire_slots::Id(0x100000003), (0x100000003u64,));
    fields(BlockUpdateFlags(17), (17u32,));
    fields(PackageKind::Code, (0u32,));
    fields(PackageRuntime::Wasm, (1u32,));
    fields(Side::Both, (1u32,));
    fields(Reduction::Armor, (1u8,));
    fields(SpawnerKind::Patrol, (1u32,));
    fields(Direction::North, (2u32,));
    fields(EquipmentSlotGroup::Head, (7u32,));
    fields(EquipmentSlot::Head, (5u32,));
    fields(
        ItemPlace::Equipment {
            entity: 17,
            slot: EquipmentSlot::Head,
        },
        (9u32, 17u32, EquipmentSlot::Head),
    );
    fields(Tag::End, (0u32,));
}

#[test]
fn every_item_place_case_has_declared_tag_and_field_types() {
    let epoch = WorldEpoch::new(47);
    let level = LevelRef::new(epoch, DimensionKey::OVERWORLD);
    let entity = EntityRef::new(epoch, 29);
    let player = crate::__private::game::player_ref(entity, 0x1234);
    let menu = MenuRef {
        player,
        window: -2,
        generation: 91,
    };
    fields(
        ItemPlace::PlayerSlot {
            player: 0x1234,
            slot: -106,
        },
        (0u32, 0x1234u128, -106i32),
    );
    fields(
        ItemPlace::PlayerHand {
            player: 0x1234,
            hand: Hand::Off,
        },
        (1u32, 0x1234u128, Hand::Off),
    );
    fields(ItemPlace::PlayerCursor { menu }, (2u32, menu));
    fields(
        ItemPlace::BlockEntity {
            level,
            pos: [7, -8, 9],
            side: Some(Direction::West),
            slot: 13,
        },
        (3u32, level, [7i32, -8, 9], Some(Direction::West), 13usize),
    );
    fields(ItemPlace::ItemEntity { id: 71 }, (4u32, 71u32));
    fields(
        ItemPlace::EntitySlot {
            entity: 29,
            slot: 17,
        },
        (5u32, 29u32, 17usize),
    );
    let parent = Box::new(ItemPlace::ItemEntity { id: 71 });
    fields(
        ItemPlace::Nested {
            parent: parent.clone(),
            slot: 19,
        },
        (6u32, parent, 19usize),
    );
    fields(
        ItemPlace::MenuSlot { menu, slot: 23 },
        (7u32, menu, 23usize),
    );
    fields(
        ItemPlace::Mod {
            provider: "example:bag",
            key: 37,
        },
        (8u32, "example:bag".to_owned(), 37u64),
    );
    fields(
        ItemPlace::Equipment {
            entity: 29,
            slot: EquipmentSlot::Saddle,
        },
        (9u32, 29u32, EquipmentSlot::Saddle),
    );
    fields(ItemPlace::Scratch(41), (10u32, 41u64));
    let parent = Box::new(ItemPlace::ItemEntity { id: 71 });
    fields(ItemPlace::Strict(parent.clone()), (11u32, parent.clone()));
    fields(ItemPlace::OneByOne(parent.clone()), (12u32, parent.clone()));
    fields(
        ItemPlace::InfiniteMaterials {
            player: 0x1234,
            inner: parent.clone(),
        },
        (13u32, 0x1234u128, parent),
    );
    // The other four holder cases are exercised in handle_records above.
    fields(AttachmentHolder::Level(level), (4u32, level));
    fields(AttachmentRowError::StaleHolder, (1u32,));
}

#[test]
fn every_direction_and_equipment_case_has_declared_tag_type() {
    for (value, tag) in [
        (Direction::Down, 0u32),
        (Direction::Up, 1),
        (Direction::North, 2),
        (Direction::South, 3),
        (Direction::West, 4),
        (Direction::East, 5),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (EquipmentSlot::MainHand, 0u32),
        (EquipmentSlot::OffHand, 1),
        (EquipmentSlot::Feet, 2),
        (EquipmentSlot::Legs, 3),
        (EquipmentSlot::Chest, 4),
        (EquipmentSlot::Head, 5),
        (EquipmentSlot::Body, 6),
        (EquipmentSlot::Saddle, 7),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (EquipmentSlotGroup::Any, 0u32),
        (EquipmentSlotGroup::MainHand, 1),
        (EquipmentSlotGroup::OffHand, 2),
        (EquipmentSlotGroup::Hand, 3),
        (EquipmentSlotGroup::Feet, 4),
        (EquipmentSlotGroup::Legs, 5),
        (EquipmentSlotGroup::Chest, 6),
        (EquipmentSlotGroup::Head, 7),
        (EquipmentSlotGroup::Armor, 8),
        (EquipmentSlotGroup::Body, 9),
        (EquipmentSlotGroup::Saddle, 10),
    ] {
        fields(value, (tag,));
    }
}

#[test]
fn every_reduction_spawner_package_and_neighbour_case_has_declared_tag_type() {
    for (value, tag) in [
        (Reduction::Invulnerability, 0u8),
        (Reduction::Armor, 1),
        (Reduction::Enchantments, 2),
        (Reduction::MobEffects, 3),
        (Reduction::Absorption, 4),
        (Reduction::InnateResistance, 5),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (SpawnerKind::Phantom, 0u32),
        (SpawnerKind::Patrol, 1),
        (SpawnerKind::Cat, 2),
        (SpawnerKind::VillageSiege, 3),
        (SpawnerKind::WanderingTrader, 4),
    ] {
        fields(value, (tag,));
    }
    fields(PackageKind::Code, (0u32,));
    fields(PackageKind::Content, (1u32,));
    fields(Side::Client, (0u32,));
    fields(Side::Both, (1u32,));
    fields(LevelDimension::Overworld, (0u32,));
    fields(LevelDimension::Nether, (1u32,));
    fields(LevelDimension::End, (2u32,));
    fields(NeighbourUpdate::OutputSignal, (0u32,));
    fields(NeighbourUpdate::Neighbours, (1u32,));
    fields(
        NeighbourUpdate::NeighboursExcept(Direction::East),
        (2u32, Direction::East),
    );
}

#[test]
fn every_game_mode_pose_animation_and_connection_case_has_declared_tag_type() {
    for (value, tag) in [
        (GameMode::Survival, 0u32),
        (GameMode::Creative, 1),
        (GameMode::Adventure, 2),
        (GameMode::Spectator, 3),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (ArmPose::Empty, 0u32),
        (ArmPose::Item, 1),
        (ArmPose::Block, 2),
        (ArmPose::Bow, 3),
        (ArmPose::CrossbowHold, 4),
        (ArmPose::CrossbowCharge, 5),
        (ArmPose::ThrowTrident, 6),
        (ArmPose::TootHorn, 7),
        (ArmPose::Spyglass, 8),
        (ArmPose::Spear, 9),
        (ArmPose::Brush, 10),
    ] {
        let mut output = Writer::new();
        output.put(&value);
        assert_eq!(output.bytes(), tag.to_le_bytes());
        assert_eq!(
            Reader::new(output.bytes()).take::<ArmPose>().unwrap(),
            value
        );
    }
    for (value, tag) in [
        (UseAnimation::None, 0u32),
        (UseAnimation::Eat, 1),
        (UseAnimation::Drink, 2),
        (UseAnimation::Block, 3),
        (UseAnimation::Bow, 4),
        (UseAnimation::Trident, 5),
        (UseAnimation::Crossbow, 6),
        (UseAnimation::Spyglass, 7),
        (UseAnimation::TootHorn, 8),
        (UseAnimation::Brush, 9),
        (UseAnimation::Bundle, 10),
        (UseAnimation::Spear, 11),
    ] {
        fields(value, (tag,));
    }
    fields(ConnectionKind::Fence, (0u32,));
    fields(ConnectionKind::Wall, (1u32,));
    fields(ConnectionKind::Bars, (2u32,));
}

#[test]
fn every_attachment_error_and_write_case_has_declared_tag_type() {
    for (value, tag) in [
        (AttachmentError::MissingHolder, 0u32),
        (AttachmentError::TypeMismatch, 1),
        (AttachmentError::Decode, 2),
        (AttachmentError::Encode, 3),
        (AttachmentError::InvalidNamespace, 4),
        (AttachmentError::InvalidCopyPolicy, 5),
        (AttachmentError::InvalidSyncPolicy, 6),
    ] {
        fields(value, (tag,));
    }
    fields(AttachmentError::HolderBusy, (7u32,));
    fields(
        AttachmentError::UnsupportedHolder("dragonpart"),
        (8u32, "dragonpart"),
    );
    fields(AttachmentRowError::HolderBusy, (2u32,));
    fields(
        AttachmentRowError::UnsupportedHolder("dragonpart"),
        (3u32, "dragonpart"),
    );
    fields(AttachmentWrite::Decoded, (0u32,));
    fields(AttachmentWrite::Default, (1u32,));
    fields(AttachmentWrite::Set, (2u32,));
}

#[test]
fn erased_provider_record_uses_active_resource_scope() {
    // Pin the layout at this layer; platform tests own active codec lifetimes.
    fields(
        OwnProvider {
            handler: "example:handler".into(),
            provider: std::sync::Arc::new(73u32),
        },
        ("example:handler".to_owned(), 0x100000007u64),
    );
}

#[test]
fn original_player_identity_and_attachment_selector_cross_the_boundary() {
    let original = crate::__private::game::original_player_ref(0x1234_5678);
    fields(original, (0x1234_5678u64,));
    fields(Holder::OriginalPlayer(0x1234_5678), (5u32, 0x1234_5678u64));
}
