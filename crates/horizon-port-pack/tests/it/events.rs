//! `events`: a jar's NeoForge listeners and capabilities, read from class
//! files, against the hook catalog's `neoforge` labels.
use horizon_port_pack::{
    classes,
    events::{self, Catalog, Status},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Write},
};

/// A NeoForge listener class: `onTick` takes `ServerTickEvent$Post` under
/// `@SubscribeEvent(priority = HIGH)`, `plain` takes `LevelEvent$Load` with
/// no annotation, a method type takes `ChunkEvent$Load` (a method
/// reference's), and a field reference reads `Capabilities$ItemHandler.BLOCK`.
fn neoforge_listener(name: &str) -> Vec<u8> {
    fn utf(pool: &mut Vec<Vec<u8>>, s: &str) -> u16 {
        let mut e = vec![1];
        e.extend((s.len() as u16).to_be_bytes());
        e.extend(s.as_bytes());
        pool.push(e);
        pool.len() as u16
    }
    fn entry(pool: &mut Vec<Vec<u8>>, tag: u8, refs: &[u16]) -> u16 {
        let mut e = vec![tag];
        for r in refs {
            e.extend(r.to_be_bytes());
        }
        pool.push(e);
        pool.len() as u16
    }
    let mut pool = Vec::new();
    let this = utf(&mut pool, name);
    let this = entry(&mut pool, 7, &[this]);
    let object = utf(&mut pool, "java/lang/Object");
    let object = entry(&mut pool, 7, &[object]);
    let subscribe = utf(&mut pool, "Lnet/neoforged/bus/api/SubscribeEvent;");
    let annotations = utf(&mut pool, "RuntimeVisibleAnnotations");
    let priority = utf(&mut pool, "priority");
    let priority_type = utf(&mut pool, "Lnet/neoforged/bus/api/EventPriority;");
    let high = utf(&mut pool, "HIGH");
    let on_tick = utf(&mut pool, "onTick");
    let event = "net/neoforged/neoforge/event";
    let tick = utf(
        &mut pool,
        &format!("(L{event}/tick/ServerTickEvent$Post;)V"),
    );
    let plain = utf(&mut pool, "plain");
    let level = utf(&mut pool, &format!("(L{event}/level/LevelEvent$Load;)V"));
    let chunk = utf(&mut pool, &format!("(L{event}/level/ChunkEvent$Load;)V"));
    entry(&mut pool, 16, &[chunk]);
    let caps = "net/neoforged/neoforge/capabilities";
    let item = utf(&mut pool, &format!("{caps}/Capabilities$ItemHandler"));
    let item = entry(&mut pool, 7, &[item]);
    let block = utf(&mut pool, "BLOCK");
    let block_type = utf(&mut pool, &format!("L{caps}/BlockCapability;"));
    let block = entry(&mut pool, 12, &[block, block_type]);
    entry(&mut pool, 9, &[item, block]);
    let mut out = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 65];
    out.extend(((pool.len() + 1) as u16).to_be_bytes());
    for e in &pool {
        out.extend(e);
    }
    out.extend([0, 0x21]);
    out.extend(this.to_be_bytes());
    out.extend(object.to_be_bytes());
    out.extend([0, 0, 0, 0]); // interfaces, fields
    out.extend([0, 2]);
    let mut body = vec![0, 1];
    body.extend(subscribe.to_be_bytes());
    body.extend([0, 1]);
    body.extend(priority.to_be_bytes());
    body.push(b'e');
    body.extend(priority_type.to_be_bytes());
    body.extend(high.to_be_bytes());
    out.extend([0, 1]);
    out.extend(on_tick.to_be_bytes());
    out.extend(tick.to_be_bytes());
    out.extend([0, 1]);
    out.extend(annotations.to_be_bytes());
    out.extend((body.len() as u32).to_be_bytes());
    out.extend(body);
    out.extend([0, 1]);
    out.extend(plain.to_be_bytes());
    out.extend(level.to_be_bytes());
    out.extend([0, 0]); // no attributes
    out.extend([0, 0]); // class attributes
    out
}

const LABELS: &str = r#"{"hooks": [
    {"name": "server_tick", "neoforge": "ServerTickEvent.Pre / ServerTickEvent.Post"},
    {"name": "chunk_load", "neoforge": "ChunkEvent.Load / .Unload"},
    {"name": "arrow_nock", "neoforge": "net.neoforged.neoforge.event.entity.player.ArrowNockEvent (NeoForge.EVENT_BUS, ICancellableEvent)"},
    {"name": "level_tick_post", "neoforge": "LevelTickEvent.Post"},
    {"name": "config_loading", "neoforge": "ModConfigEvent.Loading"},
    {"name": "container_opened", "neoforge": "none (not NeoForge's PlayerContainerEvent.Open: this hook can cancel the opening)"},
    {"name": "open_menu", "neoforge": "IPlayerExtension.openMenu(MenuProvider)"},
    {"name": "block_entity_tick", "neoforge": "Capabilities.Item.BLOCK"},
    {"name": "recipes_received", "neoforge": "RecipesReceivedEvent"},
    {"name": "two_events", "neoforge": "OneEvent, TwoEvent"},
    {"name": "standalone_models", "neoforge": "ModelEvent.RegisterStandalone"},
    {"name": "render_tooltip_texture", "neoforge": "RenderTooltipEvent.Texture"},
    {"name": "anvil_craft_pre", "neoforge": "AnvilCraftEvent.Pre"},
    {"name": "hud"}
]}"#;

fn hooks(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn listeners_are_subscribed_methods_and_method_references() {
    let class = classes::parse(&neoforge_listener("a/Listener")).unwrap();
    let listens: Vec<&str> = class.listens.iter().map(String::as_str).collect();
    assert_eq!(
        listens,
        [
            "net/neoforged/neoforge/event/level/ChunkEvent$Load",
            "net/neoforged/neoforge/event/tick/ServerTickEvent$Post",
        ]
    );
    let capabilities: Vec<&str> = class.capabilities.iter().map(String::as_str).collect();
    assert_eq!(capabilities, ["Capabilities.ItemHandler.BLOCK"]);
    assert_eq!(class.methods, ["onTick", "plain"]);
}

#[test]
fn labels_give_counterparts_phases_notes_and_new_names() {
    let c = Catalog::parse(LABELS).unwrap();
    let hooked = |h: &[&str]| (Status::Hooked(hooks(h)), None);
    assert_eq!(c.status("ServerTickEvent.Post"), hooked(&["server_tick"]));
    assert_eq!(c.status("ChunkEvent.Unload"), hooked(&["chunk_load"]));
    assert_eq!(c.status("ArrowNockEvent"), hooked(&["arrow_nock"]));
    let phase = |h: &[&str]| (Status::Phase(hooks(h)), None);
    assert_eq!(c.status("LevelTickEvent.Pre"), phase(&["level_tick_post"]));
    assert_eq!(c.status("ModConfigEvent"), phase(&["config_loading"]));
    assert_eq!(
        c.status("PlayerContainerEvent.Open"),
        (Status::Noted(hooks(&["container_opened"])), None)
    );
    assert_eq!(
        c.status("Capabilities.ItemHandler.BLOCK"),
        (
            Status::Hooked(hooks(&["block_entity_tick"])),
            Some("Capabilities.Item.BLOCK".to_owned())
        )
    );
    assert_eq!(
        c.status("RecipesUpdatedEvent"),
        (
            Status::Hooked(hooks(&["recipes_received"])),
            Some("RecipesReceivedEvent".to_owned())
        )
    );
    assert_eq!(
        c.status("ModelEvent.RegisterAdditional"),
        (
            Status::Hooked(hooks(&["standalone_models"])),
            Some("ModelEvent.RegisterStandalone".to_owned())
        )
    );
    assert_eq!(
        c.status("RenderTooltipEvent.Color"),
        (
            Status::Hooked(hooks(&["render_tooltip_texture"])),
            Some("RenderTooltipEvent.Texture".to_owned())
        )
    );
    assert_eq!(
        c.status("AnvilRepairEvent"),
        (
            Status::Hooked(hooks(&["anvil_craft_pre"])),
            Some("AnvilCraftEvent.Pre".to_owned())
        )
    );
    // A label names each event of a comma-separated list.
    assert_eq!(c.status("OneEvent"), hooked(&["two_events"]));
    assert_eq!(c.status("TwoEvent"), hooked(&["two_events"]));
    for missing in [
        "ModConfigEvent.Unloading",
        "RegisterPayloadHandlersEvent",
        "IPlayerExtension.openMenu",
    ] {
        assert_eq!(c.status(missing), (Status::Missing, None), "{missing}");
    }
}

#[test]
fn trade_events_are_answered_by_the_trade_registries() {
    let c = Catalog::parse(LABELS).unwrap();
    let data = (
        Status::Data(vec!["trade_set".to_owned(), "villager_trade".to_owned()]),
        None,
    );
    assert_eq!(c.status("VillagerTradesEvent"), data);
    assert_eq!(c.status("WandererTradesEvent"), data);
    let mut mods = BTreeMap::new();
    mods.insert(
        "trader".to_owned(),
        BTreeSet::from(["VillagerTradesEvent".to_owned()]),
    );
    let report = events::table(&mods, &c);
    assert!(
        report.contains("1 names: 0 with a hook, 0 with another phase's, 0 noted as not quite one, 1 made data, 0 answered elsewhere in the mod API, 0 answered by the load phases, 0 that need no hook, 0 with none."),
        "{report}"
    );
    assert!(
        report.contains(
            "| 1 | `VillagerTradesEvent` | data: `trade_set`, `villager_trade` | trader |"
        ),
        "{report}"
    );
}

#[test]
fn setup_and_intermod_events_are_answered_by_the_load_phases() {
    let c = Catalog::parse(LABELS).unwrap();
    let direct = (
        Status::Loading("a direct call, in dependency order".to_owned()),
        None,
    );
    assert_eq!(c.status("InterModEnqueueEvent"), direct);
    assert_eq!(c.status("InterModProcessEvent"), direct);
    let setup = "`Hooks::register`, then the first server reload";
    assert_eq!(
        c.status("FMLCommonSetupEvent"),
        (Status::Loading(setup.to_owned()), None)
    );
    let complete = "after the initial config, before `server_about_to_start`";
    assert_eq!(
        c.status("FMLLoadCompleteEvent"),
        (Status::Loading(complete.to_owned()), None)
    );
    assert_eq!(
        c.status("FMLConstructModEvent"),
        (Status::Loading("`Hooks::register`".to_owned()), None)
    );
    let dedicated = "`server_about_to_start`, when `dedicated`";
    assert_eq!(
        c.status("FMLDedicatedServerSetupEvent"),
        (Status::Loading(dedicated.to_owned()), None)
    );
    let mut mods = BTreeMap::new();
    mods.insert(
        "ae2".to_owned(),
        BTreeSet::from([
            "FMLCommonSetupEvent".to_owned(),
            "InterModEnqueueEvent".to_owned(),
        ]),
    );
    let report = events::table(&mods, &c);
    assert!(
        report.contains("2 names: 0 with a hook, 0 with another phase's, 0 noted as not quite one, 0 made data, 0 answered elsewhere in the mod API, 2 answered by the load phases, 0 that need no hook, 0 with none."),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "| 1 | `FMLCommonSetupEvent` | load phases: {setup} | ae2 |"
        )),
        "{report}"
    );
}

#[test]
fn registrar_methods_and_capability_keys_answer_what_no_hook_does() {
    let c = Catalog::parse(LABELS).unwrap();
    let api = |what: &str| Status::Api(what.to_owned());
    assert_eq!(
        c.status("RegisterKeyMappingsEvent"),
        (api("Registrar::register_key_mapping"), None)
    );
    assert_eq!(
        c.status("EntityAttributeModificationEvent"),
        (api("Registrar::modify_entity_attributes"), None)
    );
    // A capability is found under its 26.3 name first, then answered by its key.
    assert_eq!(
        c.status("Capabilities.FluidHandler.ITEM"),
        (
            api("mod_api::ITEM_FLUIDS"),
            Some("Capabilities.Fluid.ITEM".to_owned())
        )
    );
    assert_eq!(
        c.status("Capabilities.EnergyStorage.ENTITY"),
        (
            api("mod_api::ENTITY_ENERGY"),
            Some("Capabilities.Energy.ENTITY".to_owned())
        )
    );
    // A hook whose label claims the capability still answers it.
    assert_eq!(
        c.status("Capabilities.Item.BLOCK"),
        (Status::Hooked(hooks(&["block_entity_tick"])), None)
    );
    let unlabelled = Catalog::parse(r#"{"hooks": [{"name": "hud"}]}"#).unwrap();
    assert_eq!(
        unlabelled.status("Capabilities.ItemHandler.BLOCK"),
        (
            api("mod_api::ITEMS"),
            Some("Capabilities.Item.BLOCK".to_owned())
        )
    );
    let mut mods = BTreeMap::new();
    mods.insert(
        "mekanism".to_owned(),
        BTreeSet::from([
            "Capabilities.FluidHandler.BLOCK".to_owned(),
            "RegisterKeyMappingsEvent".to_owned(),
        ]),
    );
    let report = events::table(&mods, &c);
    assert!(
        report.contains("2 names: 0 with a hook, 0 with another phase's, 0 noted as not quite one, 0 made data, 2 answered elsewhere in the mod API, 0 answered by the load phases, 0 that need no hook, 0 with none."),
        "{report}"
    );
    assert!(
        report.contains(
            "| 1 | `Capabilities.FluidHandler.BLOCK` | mod API: `mod_api::FLUIDS` (as `Capabilities.Fluid.BLOCK`) | mekanism |"
        ),
        "{report}"
    );
    assert!(
        report.contains(
            "| 1 | `RegisterKeyMappingsEvent` | mod API: `Registrar::register_key_mapping` | mekanism |"
        ),
        "{report}"
    );
}

#[test]
fn events_made_data_since_1_21_1_name_what_a_port_writes() {
    let c = Catalog::parse(LABELS).unwrap();
    for (event, written) in [
        ("RegisterBrewingRecipesEvent", "recipe"),
        ("RegisterDimensionSpecialEffectsEvent", "dimension_type"),
        ("FurnaceFuelBurnTimeEvent", "minecraft:cooking_fuel"),
    ] {
        assert_eq!(
            c.status(event),
            (Status::Data(vec![written.to_owned()]), None),
            "{event}"
        );
    }
}

#[test]
fn events_nothing_in_the_game_fires_say_why_they_have_no_hook() {
    let c = Catalog::parse(LABELS).unwrap();
    let datagen = "data generation runs at development time, not in the game";
    assert_eq!(
        c.status("GatherDataEvent"),
        (Status::Unhooked(datagen.to_owned()), None)
    );
    for gone in [
        "IdMappingEvent",
        "RegisterNamedRenderTypesEvent",
        "RenderLevelStageEvent.RegisterStageEvent",
    ] {
        assert_eq!(
            c.status(gone),
            (Status::Unhooked("not in NeoForge 26.3".to_owned()), None),
            "{gone}"
        );
    }
    let mut mods = BTreeMap::new();
    mods.insert(
        "create".to_owned(),
        BTreeSet::from(["GatherDataEvent".to_owned()]),
    );
    let report = events::table(&mods, &c);
    assert!(
        report.contains("1 names: 0 with a hook, 0 with another phase's, 0 noted as not quite one, 0 made data, 0 answered elsewhere in the mod API, 0 answered by the load phases, 1 that need no hook, 0 with none."),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "| 1 | `GatherDataEvent` | no hook: {datagen} | create |"
        )),
        "{report}"
    );
}

#[test]
fn a_helper_nested_in_an_event_is_answered_as_the_event() {
    let c = Catalog::parse(
        r#"{"hooks": [
            {"name": "blocks", "neoforge": "RegisterEvent"},
            {"name": "items", "neoforge": "RegisterEvent"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(
        c.status("RegisterEvent.RegisterHelper"),
        (
            Status::Hooked(hooks(&["blocks", "items"])),
            Some("RegisterEvent".to_owned())
        )
    );
}

/// The 1.21.1 names of `RENAMED` against the catalog this build was made
/// from: each one's hook is still there under the 26.3 name.
#[test]
fn the_built_in_catalog_has_the_hooks_of_names_changed_since_1_21_1() {
    let c = Catalog::parse(events::CATALOG).unwrap();
    for (old, new, hook) in [
        (
            "RegisterClientReloadListenersEvent",
            "AddClientReloadListenersEvent",
            "add_client_reload_listeners",
        ),
        (
            "ModelEvent.RegisterGeometryLoaders",
            "ModelEvent.RegisterLoaders",
            "register_model_loaders",
        ),
        ("BlockEvent.BreakEvent", "BreakBlockEvent", "break_block"),
        ("EnderManAngerEvent", "EndermanAngerEvent", "enderman_anger"),
        (
            "EntityTeleportEvent.ChorusFruit",
            "EntityTeleportEvent.ItemConsumption",
            "item_consumption_teleport",
        ),
        (
            "ContainerScreenEvent.Render.Foreground",
            "ScreenEvent.Render.Foreground",
            "screen_render_foreground",
        ),
        (
            "ContainerScreenEvent.Render.Background",
            "ScreenEvent.Render.Background",
            "screen_render_background",
        ),
        (
            "ScreenEvent.BackgroundRendered",
            "ScreenEvent.Render.Background",
            "screen_render_background",
        ),
        (
            "RenderHighlightEvent.Block",
            "ExtractBlockOutlineRenderStateEvent",
            "block_highlight",
        ),
        (
            "RenderBlockScreenEffectEvent",
            "ExtractBlockScreenEffectEvent",
            "extract_block_screen_effect",
        ),
        (
            "RegisterColorHandlersEvent.Block",
            "RegisterColorHandlersEvent.BlockTintSources",
            "register_block_tint_sources",
        ),
        (
            "RegisterColorHandlersEvent.Item",
            "RegisterColorHandlersEvent.ItemTintSources",
            "register_item_tint_sources",
        ),
        (
            "CustomizeGuiOverlayEvent.DebugText",
            "RegisterDebugEntriesEvent",
            "register_debug_entries",
        ),
    ] {
        let (Status::Hooked(hooks), Some(under)) = c.status(old) else {
            panic!("{old} has no hook");
        };
        assert_eq!(under, new, "{old}");
        assert!(hooks.contains(&hook.to_owned()), "{old}: {hooks:?}");
    }
    // 26.3's event registers search categories only, which the hook's label notes.
    assert_eq!(
        c.status("RegisterRecipeBookCategoriesEvent"),
        (
            Status::Noted(hooks(&["recipe_book_categories"])),
            Some("RegisterRecipeBookSearchCategoriesEvent".to_owned())
        )
    );
}

#[test]
fn a_parent_is_hooked_when_each_of_its_events_is() {
    let all = r#"{"hooks": [
        {"name": "config_loading", "neoforge": "ModConfigEvent.Loading"},
        {"name": "config_reloading", "neoforge": "ModConfigEvent.Reloading"},
        {"name": "config_unloading", "neoforge": "ModConfigEvent.Unloading"}
    ]}"#;
    let c = Catalog::parse(all).unwrap();
    assert_eq!(
        c.status("ModConfigEvent"),
        (
            Status::Hooked(hooks(&[
                "config_loading",
                "config_reloading",
                "config_unloading"
            ])),
            None
        )
    );
}

#[test]
fn the_built_in_catalog_parses() {
    let c = Catalog::parse(events::CATALOG).unwrap();
    let (Status::Hooked(hooks), None) = c.status("LivingDeathEvent") else {
        panic!("LivingDeathEvent has no hook");
    };
    assert!(hooks.contains(&"living_death".to_owned()), "{hooks:?}");
    for (counterpart, hook) in [
        ("AnvilUpdateEvent", "anvil_update"),
        ("RenderTooltipEvent.Pre", "render_tooltip_pre"),
        // The system subclass is a name of its own to a tool reading listeners.
        ("ClientChatReceivedEvent.System", "client_chat_received"),
        ("ClientChatReceivedEvent.Player", "client_chat_received"),
    ] {
        let (Status::Hooked(hooks), None) = c.status(counterpart) else {
            panic!("{counterpart} has no hook");
        };
        assert_eq!(hooks, [hook]);
    }
    // A label that names two events with a comma, as `mob_spawned` does:
    // a tool listening to the name finds it beside the other hooks of the event.
    let (Status::Hooked(hooks), None) = c.status("EntityJoinLevelEvent") else {
        panic!("EntityJoinLevelEvent has no hook");
    };
    assert!(hooks.contains(&"mob_spawned".to_owned()), "{hooks:?}");
    // A 1.21.1 mod's core shaders: answered by the hook of the event that replaced theirs.
    assert_eq!(
        c.status("RegisterShadersEvent"),
        (
            Status::Hooked(vec!["register_render_pipelines".to_owned()]),
            Some("RegisterRenderPipelinesEvent".to_owned())
        )
    );
}

#[test]
fn a_jar_and_a_pack_are_ranked_by_how_many_mods_use_each_name() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("a/Listener.class", options).unwrap();
    zip.write_all(&neoforge_listener("a/Listener")).unwrap();
    zip.start_file("a/notes.txt", options).unwrap();
    zip.write_all(b"not a class").unwrap();
    let jar = zip.finish().unwrap();
    let names = events::scan(Cursor::new(jar.into_inner())).unwrap();
    let listed: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(
        listed,
        [
            "Capabilities.ItemHandler.BLOCK",
            "ChunkEvent.Load",
            "ServerTickEvent.Post"
        ]
    );
    let mut mods = BTreeMap::new();
    mods.insert("alpha".to_owned(), names);
    let one: BTreeSet<String> = ["ServerTickEvent.Post".to_owned()].into();
    mods.insert("beta".to_owned(), one);
    mods.insert("gamma".to_owned(), BTreeSet::new());
    let report = events::table(&mods, &Catalog::parse(LABELS).unwrap());
    assert!(report.starts_with("3 mods; 2 listen"), "{report}");
    assert!(
        report.contains("3 names: 3 with a hook, 0 with another phase's"),
        "{report}"
    );
    let rows: Vec<&str> = report.lines().filter(|l| l.starts_with("| ")).collect();
    assert_eq!(
        rows[1..],
        [
            "| 2 | `ServerTickEvent.Post` | `server_tick` | alpha, beta |",
            "| 1 | `Capabilities.ItemHandler.BLOCK` | `block_entity_tick` (as `Capabilities.Item.BLOCK`) | alpha |",
            "| 1 | `ChunkEvent.Load` | `chunk_load` | alpha |",
        ]
    );
}

#[test]
fn all_api_mapping_targets_compile() {
    let _ = <dyn mod_api::Registrar as mod_api::KeyMappingsRegistrar>::register_key_mapping;
    let _ =
        <dyn mod_api::Registrar as mod_api::EntityAttributesRegistrar>::declare_entity_attributes;
    let _ =
        <dyn mod_api::Registrar as mod_api::EntityAttributesRegistrar>::modify_entity_attributes;
    let _ = <dyn mod_api::Registrar as mod_api::SpawnPlacementsRegistrar>::declare_spawn_placement;
    let _ = <dyn mod_api::Registrar as mod_api::TicketsRegistrar>::register_ticket_controller;
    let _ = (
        mod_api::ITEMS,
        mod_api::ENTITY_ITEMS,
        mod_api::ENTITY_AUTOMATION,
        mod_api::ITEM_ITEMS,
        mod_api::FLUIDS,
        mod_api::ENTITY_FLUIDS,
        mod_api::ITEM_FLUIDS,
        mod_api::ENERGY,
        mod_api::ENTITY_ENERGY,
        mod_api::ITEM_ENERGY,
    );
}

#[test]
fn every_api_mapping_has_the_expected_target() {
    let catalog = Catalog::parse(r#"{"hooks":[]}"#).unwrap();
    for (event, target) in [
        (
            "RegisterKeyMappingsEvent",
            "Registrar::register_key_mapping",
        ),
        (
            "EntityAttributeCreationEvent",
            "Registrar::declare_entity_attributes",
        ),
        (
            "EntityAttributeModificationEvent",
            "Registrar::modify_entity_attributes",
        ),
        (
            "RegisterSpawnPlacementsEvent",
            "Registrar::declare_spawn_placement",
        ),
        (
            "RegisterTicketControllersEvent",
            "Registrar::register_ticket_controller",
        ),
        ("Capabilities.Item.BLOCK", "mod_api::ITEMS"),
        ("Capabilities.Item.ENTITY", "mod_api::ENTITY_ITEMS"),
        (
            "Capabilities.Item.ENTITY_AUTOMATION",
            "mod_api::ENTITY_AUTOMATION",
        ),
        ("Capabilities.Item.ITEM", "mod_api::ITEM_ITEMS"),
        ("Capabilities.Fluid.BLOCK", "mod_api::FLUIDS"),
        ("Capabilities.Fluid.ENTITY", "mod_api::ENTITY_FLUIDS"),
        ("Capabilities.Fluid.ITEM", "mod_api::ITEM_FLUIDS"),
        ("Capabilities.Energy.BLOCK", "mod_api::ENERGY"),
        ("Capabilities.Energy.ENTITY", "mod_api::ENTITY_ENERGY"),
        ("Capabilities.Energy.ITEM", "mod_api::ITEM_ENERGY"),
    ] {
        assert_eq!(
            catalog.status(event).0,
            Status::Api(target.to_owned()),
            "{event}"
        );
    }
}
