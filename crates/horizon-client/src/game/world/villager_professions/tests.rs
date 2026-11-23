//! End-to-end registration and root publication, with unique resource and save roots.
use super::*;
use horizon_client::mobs;
use horizon_core::nbt::Tag;
use mod_api::{
    DataPackFilesEvent, Hooks, Identifier, ModEntry, PoiKey, ProfessionDefinition, ProfessionKey,
    ProfessionWorkstation, RegisterVillagerProfessionsEvent, RegistrationContext, ServerHost,
    TradeSetKey,
};
use std::sync::atomic::{AtomicBool, Ordering};

fn definition() -> ProfessionDefinition {
    let id = |name| Identifier::parse(name).unwrap();
    ProfessionDefinition {
        key: ProfessionKey(id("test:engineer")),
        workstations: vec![ProfessionWorkstation {
            poi: PoiKey(id("test:assembler")),
            block: id("test:assembler"),
            properties: vec![],
        }],
        held_poi: vec![PoiKey(id("test:assembler"))],
        acquirable_poi: vec![PoiKey(id("test:assembler"))],
        requested_items: vec![],
        secondary_blocks: vec![],
        work_sound: None,
        trades: vec![(1, TradeSetKey(id("test:engineer")))],
    }
}
struct Engineer {
    valid: Arc<AtomicBool>,
    key: &'static str,
}
#[mod_api::__subscriptions]
impl Hooks for Engineer {
    fn register_villager_professions(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterVillagerProfessionsEvent<'_>,
    ) {
        let mut value = definition();
        value.key = ProfessionKey(Identifier::parse(self.key).unwrap());
        event.professions.register(value);
    }
    fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
        if self.valid.load(Ordering::SeqCst) {
            event.files.push((
                "trade_set/test:engineer".into(),
                br#"{"amount":1,"trades":["test:engineer_offer"],"random_sequence":"test:engineer"}"#.to_vec(),
            ));
            event.files.push(("villager_trade/test:engineer_offer".into(), br#"{"gives":{"id":"minecraft:iron_ingot","count":2},"wants":{"id":"minecraft:emerald","count":3},"max_uses":12,"xp":7}"#.to_vec()));
        }
    }
}
fn resource_root(scratch: &std::path::Path) -> std::path::PathBuf {
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let module = scratch.join("module");
    std::fs::create_dir_all(module.join("data/test/horizon/block")).unwrap();
    std::fs::write(module.join("horizon-mod.toml"), "id = 'test'\nname = 'Profession fixture'\nversion = '1.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n").unwrap();
    std::fs::write(
        module.join("data/test/horizon/block/assembler.json"),
        r#"{"template":"minecraft:stone"}"#,
    )
    .unwrap();
    let package = horizon_mod_package::Package::read(&module).unwrap();
    let content = horizon_mod_package::Content::read(&module, &package.manifest).unwrap();
    let root = scratch.join("resources");
    horizon_client::mod_content::build(
        &base,
        std::path::Path::new(""),
        &[horizon_client::mod_content::ContentMod {
            id: "test".into(),
            content,
            jars: vec![],
        }],
        &root,
        &horizon_mod_package::enabled::EnabledMods::new([("test".into(), mod_api::Side::Both)]),
    )
    .unwrap();
    root
}
fn request(root: &std::path::Path) -> world_loading::WorldRequest {
    let mut value = world_loading::WorldRequest::new(
        "profession test",
        Some("test"),
        Some(1),
        true,
        None,
        Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
    )
    .unwrap();
    value.save_root = root.into();
    value
}
fn fixture(mods: mod_api::Mods, run: impl FnOnce(&mut Game<'_>)) {
    let scratch = tempfile::tempdir().unwrap();
    let root = resource_root(scratch.path());
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game_at(root, |game| {
            game.load_world(request(&scratch.path().join("save")))
                .unwrap();
            run(game);
        });
    });
}

#[test]
#[ignore = "daily"]
fn villager_engineer_registration_and_assembler_job_acquisition() {
    let valid = Arc::new(AtomicBool::new(true));
    fixture(
        vec![ModEntry::new(
            "engineer",
            Box::new(Engineer {
                valid,
                key: "test:engineer",
            }),
        )]
        .into(),
        assert_engineer_acquisition,
    );
}

#[test]
#[ignore = "daily"]
fn villager_failed_reload_keeps_profession_poi_and_trade_snapshot() {
    let valid = Arc::new(AtomicBool::new(true));
    fixture(
        vec![ModEntry::new(
            "engineer",
            Box::new(Engineer {
                valid: valid.clone(),
                key: "test:engineer",
            }),
        )]
        .into(),
        |game| {
            let old = game.stations().professions.clone();
            valid.store(false, Ordering::SeqCst);
            assert!(
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .is_err()
            );
            assert!(Arc::ptr_eq(&old, &game.stations().professions));
            assert!(Arc::ptr_eq(&old, &game.stations().trades.professions));
            assert_eq!(
                game.stations().trades.set_for("test:engineer", 1),
                "test:engineer"
            );
            valid.store(true, Ordering::SeqCst);
            game.reload_datapacks(true, &game.level.enabled_features.clone())
                .unwrap();
            assert!(!Arc::ptr_eq(&old, &game.stations().professions));
            assert_eq!(
                game.stations().professions.acquiring("test:assembler"),
                Some(15)
            );
        },
    );
}

fn refused(mods: mod_api::Mods, expected: &[&str]) {
    let scratch = tempfile::tempdir().unwrap();
    let root = resource_root(scratch.path());
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game_at(root, |game| {
            let old = game.stations().professions.clone();
            let error = game
                .load_world(request(&scratch.path().join("save")))
                .unwrap_err()
                .to_string();
            for text in expected {
                assert!(error.contains(text), "missing {text} in {error}");
            }
            assert!(Arc::ptr_eq(&old, &game.stations().professions));
            assert_eq!(game.stations().professions.entries().len(), 15);
            assert_eq!(
                game.stations().professions.resolve("minecraft:farmer"),
                Some(5)
            );
            assert_eq!(game.stations().professions.resolve("test:engineer"), None);
            assert!(!game.stations().trades.sets.contains_key("test:engineer"));
        });
    });
}
#[test]
#[ignore = "daily"]
fn villager_duplicate_owners_publish_zero_entries() {
    let valid = Arc::new(AtomicBool::new(true));
    refused(
        vec![
            ModEntry::new(
                "left",
                Box::new(Engineer {
                    valid: valid.clone(),
                    key: "test:engineer",
                }),
            ),
            ModEntry::new(
                "right",
                Box::new(Engineer {
                    valid,
                    key: "test:engineer",
                }),
            ),
        ]
        .into(),
        &["DuplicateProfession", "left", "right"],
    );
}
#[test]
#[ignore = "daily"]
fn villager_reserved_profession_publishes_zero_entries() {
    refused(
        vec![ModEntry::new(
            "reserved",
            Box::new(Engineer {
                valid: Arc::new(AtomicBool::new(true)),
                key: "minecraft:farmer",
            }),
        )]
        .into(),
        &["ReservedProfession"],
    );
}
#[test]
#[ignore = "daily"]
fn villager_overlapping_poi_publishes_zero_entries() {
    let valid = Arc::new(AtomicBool::new(true));
    refused(
        vec![
            ModEntry::new(
                "left",
                Box::new(Engineer {
                    valid: valid.clone(),
                    key: "test:engineer",
                }),
            ),
            ModEntry::new(
                "right",
                Box::new(Engineer {
                    valid,
                    key: "test:mechanic",
                }),
            ),
        ]
        .into(),
        &["OverlappingPoi", "assembler"],
    );
}
#[test]
#[ignore = "daily"]
fn villager_no_mods_world_retains_farmer_composter_and_five_levels() {
    let scratch = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(vec![].into(), || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.load_world(request(&scratch.path().join("save")))
                .unwrap();
            let table = &game.stations().professions;
            assert_eq!(table.entries().len(), 15);
            assert_eq!(table.resolve("minecraft:farmer"), Some(5));
            let composter = horizon_world::chunk::BlockState {
                name: "minecraft:composter".into(),
                properties: Default::default(),
            };
            assert_eq!(table.job(table.pois.classify(&composter).unwrap()), Some(5));
            assert_eq!(
                game.stations().trades.set_for("minecraft:farmer", 1),
                "minecraft:farmer/level_1"
            );
            assert_eq!(
                game.stations().trades.set_for("minecraft:farmer", 2),
                "minecraft:farmer/level_2"
            );
            assert_eq!(
                game.stations().trades.set_for("minecraft:farmer", 3),
                "minecraft:farmer/level_3"
            );
            assert_eq!(
                game.stations().trades.set_for("minecraft:farmer", 4),
                "minecraft:farmer/level_4"
            );
            assert_eq!(
                game.stations().trades.set_for("minecraft:farmer", 5),
                "minecraft:farmer/level_5"
            );
        });
    });
}

fn wasm_engineer() -> mod_api::ModEntry {
    wasm_profession(definition(), "wasm_engineer")
}
fn wasm_profession(value: ProfessionDefinition, owner: &str) -> mod_api::ModEntry {
    use mod_api::wire::Writer;
    fn data(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
    }
    let mut subscriptions = Writer::new();
    subscriptions.put(&vec![(
        "register_villager_professions".to_owned(),
        0i32,
        false,
    )]);
    let subscriptions = subscriptions.into_bytes();
    let mut declaration = Writer::new();
    declaration.put(&value);
    let declaration = declaration.into_bytes();
    let operation = horizon_mod_wasm::fixture_import_id("VillagerProfessions", "register").unwrap();
    let subscribed = horizon_mod_wasm::fixture_export_id("Hooks", "subscribed").unwrap();
    let hook =
        horizon_mod_wasm::fixture_export_id("Hooks", "register_villager_professions").unwrap();
    let std = wat::parse_str(
        r#"(module (@custom "dylink.0" "\01\04\00\00\00\00") (import "env" "memory" (memory 1)))"#,
    )
    .unwrap();
    let module = wat::parse_str(horizon_mod_wasm::fixture::by_name(&format!(r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (import "env" "memory" (memory 1))
        (import "env" "__indirect_function_table" (table 1 funcref))
        (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
        (import "horizon" "host_reply" (func $reply (param i32)))
        (data (i32.const 2048) "{subscriptions}")
        (data (i32.const 4096) "{declaration}")
        (func (export "horizon_layout") (result i64) i64.const {layout})
        (func (export "horizon_in") (param i32) (result i32) i32.const 512)
        (func (export "horizon_hook") (param i32) (result i64)
            local.get 0 i32.const {subscribed} i32.eq if (result i64)
                i64.const {sub_reply}
            else local.get 0 i32.const {hook} i32.eq if (result i64)
                i32.const {operation} i32.const 4096 i32.const {length} i32.const 0 i32.const 0 call $host
                i32.const 0 i32.ne if unreachable end
                i64.const 0
            else i64.const 0 end end))"#,
            subscriptions=data(&subscriptions), declaration=data(&declaration), layout=*mod_api::wire::LAYOUT,
            sub_reply=(2048u64<<32)|subscriptions.len() as u64, length=declaration.len()), &[("VillagerProfessions", "register")])).unwrap();
    let manifest = horizon_mod_package::manifest::Manifest::parse(&format!("id = '{owner}'\nname = 'Wasm engineer'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[wasm]\nsha512 = '{}'\ntoolchain = 'test'\nbuilt_with = 'test'\n", "a".repeat(128))).unwrap();
    let mut loaded = horizon_mod_wasm::load(
        &std,
        vec![horizon_mod_wasm::WasmPackage { manifest, module }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[]).unwrap();
    assert_eq!(loaded.mods.len(), 1);
    loaded.mods.remove(0)
}
struct TradeFiles;
#[mod_api::__subscriptions]
impl Hooks for TradeFiles {
    fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
        event.files.push((
            "trade_set/test:engineer".into(),
            br#"{"amount":1,"trades":["test:engineer_offer"],"random_sequence":"test:engineer"}"#
                .to_vec(),
        ));
        event.files.push(("villager_trade/test:engineer_offer".into(), br#"{"gives":{"id":"minecraft:iron_ingot","count":2},"wants":{"id":"minecraft:emerald","count":3},"max_uses":12,"xp":7}"#.to_vec()));
    }
}
#[test]
#[ignore = "daily"]
fn villager_wasm_engineer_declaration_resolves_with_assembler() {
    // Instantiate and invoke the real wasm guest on the production-sized stack.
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        fixture(
            vec![
                ModEntry::new("trade_files", Box::new(TradeFiles)),
                wasm_engineer(),
            ]
            .into(),
            assert_engineer_acquisition,
        );
    });
}

fn assert_engineer_acquisition(game: &mut Game<'_>) {
    let table = game.stations().professions.clone();
    assert_eq!(table.entries().len(), 16);
    assert_eq!(table.resolve("test:engineer"), Some(15));
    let host: &dyn mod_api::Host = game;
    let engineer = host
        .registry_entry("minecraft:villager_profession", "test:engineer")
        .expect("the registered gameplay profession is a generic registry entry");
    assert_eq!(engineer.id.to_string(), "test:engineer");
    assert!(
        host.registry_entries("minecraft:villager_profession")
            .unwrap()
            .contains(&Identifier::parse("test:engineer").unwrap())
    );
    assert!(
        host.registry_entries("minecraft:villager_profession")
            .unwrap()
            .contains(&Identifier::parse("minecraft:farmer").unwrap())
    );
    assert_eq!(table.acquiring("test:assembler"), Some(15));
    assert_eq!(table.trade_key("test:engineer", 1), Some("test:engineer"));
    assert_eq!(table.trade_key("test:engineer", 2), None);
    let registry = game.pack.assets.registry;
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    for x in 0..4 {
        for z in 0..4 {
            game.world
                .set(x, 63, z, registry.default_state("minecraft:stone").unwrap())
                .unwrap();
        }
    }
    game.world
        .set(1, 64, 0, registry.default_state("test:assembler").unwrap())
        .unwrap();
    let save_tables = horizon_client::save::SaveTables::new(&game.pack.assets);
    let poi =
        horizon_client::save::encode_poi(&game.world.chunks[&(0, 0)], &save_tables, Some(&table))
            .unwrap();
    let records = poi
        .tag
        .get("Sections")
        .unwrap()
        .get("4")
        .unwrap()
        .get("Records")
        .unwrap();
    let Tag::List(records) = records else {
        panic!("POI records must be a list");
    };
    assert_eq!(records.len(), 1);
    assert_eq!(
        horizon_client::save::text(records[0].get("type")).as_deref(),
        Some("test:assembler")
    );
    assert_eq!(records[0].get("pos"), Some(&Tag::IntArray(vec![1, 64, 0])));
    assert_eq!(records[0].get("free_tickets"), Some(&Tag::Int(1)));
    let raw = Tag::compound([
        ("id", Tag::String("minecraft:villager".into())),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(0.5f64.to_bits()),
                Tag::Double(64.0f64.to_bits()),
                Tag::Double(0.5f64.to_bits()),
            ]),
        ),
        (
            "VillagerData",
            Tag::compound([
                ("profession", Tag::String("minecraft:none".into())),
                ("level", Tag::Int(1)),
            ]),
        ),
    ]);
    let mut mob =
        horizon_client::entity_nbt::mob_from_nbt(&raw, game.res.mob_types(), 901, Some(registry))
            .unwrap();
    mob.on_ground = true;
    let mut world = game.mob_world_base(&[]);
    let mut damage = horizon_client::health::TestLevel::new(1);
    let mut level = damage.level();
    for time in 0..250 {
        world.game_time = time;
        world.day_time = 2000;
        mobs::villager_brain::tick(&mut mob, &world, &mut level, &mut vec![]);
        if table.state_index(&mob.extra.profession) == Some(15) {
            break;
        }
    }
    assert_eq!(mob.extra.profession.id(), "test:engineer");
    assert_eq!(mob.extra.home, Some([1, 64, 0]));
    assert_eq!(mob.extra.profession.index(), None);
    game.mobs.push(mob);
    let index = game.mobs.iter().position(|mob| mob.id == 901).unwrap();
    game.ensure_offers(index);
    let offers = &game.mobs[index].extra.trade.as_ref().unwrap().offers;
    assert_eq!(offers.len(), 1);
    assert_eq!(&*offers[0].base_cost_a.stack.item, "minecraft:emerald");
    assert_eq!(offers[0].base_cost_a.stack.count, 3);
    assert_eq!(&*offers[0].result.item, "minecraft:iron_ingot");
    assert_eq!(offers[0].result.count, 2);
}

fn saved_engineer(kind: &str) -> Tag {
    let item = |name: &str, count| {
        Tag::compound([("id", Tag::String(name.into())), ("count", Tag::Int(count))])
    };
    Tag::compound([
        ("id", Tag::String(kind.into())),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(0.5f64.to_bits()),
                Tag::Double(64.0f64.to_bits()),
                Tag::Double(0.5f64.to_bits()),
            ]),
        ),
        (
            "VillagerData",
            Tag::compound([
                ("profession", Tag::String("test:engineer".into())),
                ("type", Tag::String("minecraft:plains".into())),
                ("level", Tag::Int(3)),
            ]),
        ),
        ("Xp", Tag::Int(37)),
        (
            "Offers",
            Tag::compound([(
                "Recipes",
                Tag::List(vec![Tag::compound([
                    ("buy", item("minecraft:emerald", 3)),
                    ("sell", item("minecraft:iron_ingot", 2)),
                    ("uses", Tag::Int(2)),
                    ("maxUses", Tag::Int(12)),
                    ("xp", Tag::Int(7)),
                ])]),
            )]),
        ),
    ])
}
#[test]
#[ignore = "daily"]
fn villager_game_load_resolves_saved_names_for_both_species_without_reroll() {
    fixture(
        vec![ModEntry::new(
            "engineer",
            Box::new(Engineer {
                valid: Arc::new(AtomicBool::new(true)),
                key: "test:engineer",
            }),
        )]
        .into(),
        |game| {
            for kind in ["minecraft:villager", "minecraft:zombie_villager"] {
                let id = game.load_mob(saved_engineer(kind), None).unwrap();
                let index = game.mobs.iter().position(|mob| mob.id == id).unwrap();
                let mob = &game.mobs[index];
                assert!(matches!(
                    mob.extra.profession,
                    horizon_client::mobs::villager::ProfessionState::Registered { .. }
                ));
                assert_eq!(
                    game.stations()
                        .professions
                        .state_index(&mob.extra.profession),
                    Some(15)
                );
                assert_eq!(mob.extra.profession.id(), "test:engineer");
                let original = mob.extra.trade.as_ref().unwrap().clone();
                if kind == "minecraft:villager" {
                    game.ensure_offers(index);
                }
                assert_eq!(
                    game.mobs[index].extra.trade.as_deref(),
                    Some(original.as_ref())
                );
                let saved = horizon_client::entity_nbt::mob_to_nbt(
                    &game.mobs[index],
                    [11, 12, 13, 14],
                    None,
                    Some(game.pack.assets.registry),
                    |_| None,
                );
                assert_eq!(
                    horizon_client::save::text(
                        saved
                            .get("VillagerData")
                            .and_then(|data| data.get("profession"))
                    )
                    .as_deref(),
                    Some("test:engineer")
                );
            }
        },
    );
}
#[test]
#[ignore = "daily"]
fn villager_game_missing_mod_keeps_both_species_unresolved_and_offers_untouched() {
    fixture(vec![].into(), |game| {
        for kind in ["minecraft:villager", "minecraft:zombie_villager"] {
            let id = game.load_mob(saved_engineer(kind), None).unwrap();
            let index = game.mobs.iter().position(|mob| mob.id == id).unwrap();
            assert!(matches!(
                game.mobs[index].extra.profession,
                horizon_client::mobs::villager::ProfessionState::Unresolved(_)
            ));
            assert_eq!(game.mobs[index].extra.profession.index(), None);
            let original = game.mobs[index].extra.trade.as_ref().unwrap().clone();
            if kind == "minecraft:villager" {
                game.ensure_offers(index);
                let mut mob = horizon_client::entity_nbt::mob_from_nbt(
                    &saved_engineer(kind),
                    game.res.mob_types(),
                    999,
                    Some(game.pack.assets.registry),
                )
                .unwrap();
                let mut world = game.mob_world_base(&[]);
                let mut damage = horizon_client::health::TestLevel::new(7);
                let mut level = damage.level();
                for time in 0..100 {
                    world.game_time = time;
                    world.day_time = 2000;
                    mobs::villager_brain::tick(&mut mob, &world, &mut level, &mut vec![]);
                }
                assert_eq!(mob.extra.profession.id(), "test:engineer");
                assert_eq!(mob.extra.profession.index(), None);
                assert_eq!(mob.extra.trade.as_deref(), Some(original.as_ref()));
            }
            assert_eq!(game.mobs[index].extra.profession.id(), "test:engineer");
            assert_eq!(
                game.mobs[index].extra.trade.as_deref(),
                Some(original.as_ref())
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn villager_load_world_save_dir_preserves_custom_workstation() {
    fixture(
        vec![ModEntry::new(
            "engineer",
            Box::new(Engineer {
                valid: Arc::new(AtomicBool::new(true)),
                key: "test:engineer",
            }),
        )]
        .into(),
        |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.world
                .set(
                    1,
                    64,
                    0,
                    game.pack
                        .assets
                        .registry
                        .default_state("test:assembler")
                        .unwrap(),
                )
                .unwrap();
            game.save
                .tables
                .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&game.pack.assets)));
            let chunks = vec![game.world.chunks[&(0, 0)].clone()];
            game.save
                .save(&chunks, game.pack.assets.registry, world_biomes!(game))
                .unwrap();
            let mut region =
                horizon_world::region::RegionFile::open(game.save.root.join("poi/r.0.0.mca"))
                    .unwrap();
            let record = region
                .read(horizon_core::position::ChunkPos { x: 0, z: 0 })
                .unwrap()
                .unwrap();
            let root = record.nbt(horizon_core::nbt::Limits::saved()).unwrap();
            let sections = root.tag.get("Sections").unwrap();
            let records = sections.get("4").unwrap().get("Records").unwrap();
            let Tag::List(records) = records else {
                panic!("POI records list")
            };
            assert!(records.iter().any(|record| record.get("type") == Some(&Tag::String("test:assembler".into()))));
        },
    );
}

#[test]
#[ignore = "daily"]
fn villager_native_slots_ignore_declaring_mod_order() {
    struct NativeProfession(ProfessionDefinition);
    #[mod_api::__subscriptions]
    impl Hooks for NativeProfession {
        fn register_villager_professions(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterVillagerProfessionsEvent<'_>,
        ) {
            event.professions.register(self.0.clone());
        }
    }

    for reverse in [false, true] {
        let mut mechanic = definition();
        mechanic.key = ProfessionKey(Identifier::parse("test:mechanic").unwrap());
        let bench = PoiKey(Identifier::parse("other:bench").unwrap());
        mechanic.workstations = vec![ProfessionWorkstation {
            poi: bench.clone(),
            block: Identifier::parse("minecraft:furnace").unwrap(),
            properties: vec![],
        }];
        mechanic.held_poi = vec![bench.clone()];
        mechanic.acquirable_poi = vec![bench];
        let mut mods = vec![
            ModEntry::new("z_engineer", Box::new(NativeProfession(definition()))),
            ModEntry::new("a_mechanic", Box::new(NativeProfession(mechanic))),
        ];
        if reverse {
            mods.reverse();
        }
        mods.insert(0, ModEntry::new("trade_files", Box::new(TradeFiles)));
        fixture(mods.into(), |game| {
            let table = &game.stations().professions;
            assert_eq!(table.resolve("test:engineer"), Some(15));
            assert_eq!(table.resolve("test:mechanic"), Some(16));
            assert_eq!(table.resolve("minecraft:weaponsmith"), Some(14));
            assert_eq!(
                table.pois.resolve("other:bench"),
                Some(horizon_world::poi::PoiIdentity::Custom(0))
            );
            assert_eq!(
                table.pois.resolve("test:assembler"),
                Some(horizon_world::poi::PoiIdentity::Custom(1))
            );
        });
    }
}

#[test]
#[ignore = "daily"]
fn villager_wasm_slots_ignore_declaring_mod_order() {
    // Instantiate and invoke the real wasm guest on the production-sized stack.
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        for reverse in [false, true] {
            let mut mechanic = definition();
            mechanic.key = ProfessionKey(Identifier::parse("test:mechanic").unwrap());
            let bench = PoiKey(Identifier::parse("other:bench").unwrap());
            mechanic.workstations = vec![ProfessionWorkstation {
                poi: bench.clone(),
                block: Identifier::parse("minecraft:furnace").unwrap(),
                properties: vec![],
            }];
            mechanic.held_poi = vec![bench.clone()];
            mechanic.acquirable_poi = vec![bench];
            let mut mods = vec![
                wasm_profession(definition(), "z_engineer"),
                wasm_profession(mechanic, "a_mechanic"),
            ];
            if reverse {
                mods.reverse();
            }
            mods.insert(0, ModEntry::new("trade_files", Box::new(TradeFiles)));
            fixture(mods.into(), |game| {
                let table = &game.stations().professions;
                assert_eq!(table.resolve("test:engineer"), Some(15));
                assert_eq!(table.resolve("test:mechanic"), Some(16));
                assert_eq!(
                    table.pois.resolve("test:assembler"),
                    Some(horizon_world::poi::PoiIdentity::Custom(1))
                );
            });
        }
    });
}
#[test]
#[ignore = "daily"]
fn villager_registered_profession_keeps_a_guest_trade_screen_valid() {
    use horizon_client::lan::{Joined, PlayerSender, guest::Guest};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::atomic::{AtomicI32, AtomicUsize};
    fixture(
        vec![ModEntry::new(
            "engineer",
            Box::new(Engineer {
                valid: Arc::new(AtomicBool::new(true)),
                key: "test:engineer",
            }),
        )]
        .into(),
        |game| {
            let id = game
                .load_mob(saved_engineer("minecraft:villager"), None)
                .unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [1.5, 64.0, 0.5],
                10,
            );
            // `AbstractVillager.stillValid`: alive and in reach; a registered
            // profession resolves in the world's table, so the screen stays open
            // and the guest's trades are applied (not opaque).
            assert!(game.lan_merchant_valid(&guest, id));
            assert!(!game.lan_merchant_opaque(id));
            // An id no mod registers doesn't resolve: the screen closes.
            let index = game.mobs.iter().position(|mob| mob.id == id).unwrap();
            game.mobs[index].extra.profession =
                horizon_client::mobs::villager::ProfessionState::Unresolved(
                    "example:engineer".into(),
                );
            assert!(!game.lan_merchant_valid(&guest, id));
            assert!(game.lan_merchant_opaque(id));
        },
    );
}
