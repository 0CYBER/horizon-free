//! Isolated process: the load callbacks and custom-component IDs install once.
use glam::Mat4;
use horizon_assets::item_model::{DisplayContext, NoEnv, StackView};
use horizon_client_lib::{
    self as client,
    hud::Rect,
    item_render::{ItemRenderer, NoHost},
};
use horizon_world::{
    item_defaults::ItemData,
    loot::{LootData, PackOverlay},
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    native_load_hooks::{self, NativeLoadHooks},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static CALLS: AtomicUsize = AtomicUsize::new(0);

/// The producer fires once; gameplay/enchantment metadata, both render loaders,
/// banners, loot and the content handshake borrow its published snapshot.
#[test]
fn default_components_reader_baseline() -> client::Result<()> {
    let root = std::env::temp_dir().join(format!("horizon-default-reader-{}", std::process::id()));
    for directory in [
        "assets/probe/items",
        "assets/probe/models/item",
        "assets/probe/textures/item",
        "data/probe",
    ] {
        fs::create_dir_all(root.join(directory))?;
    }
    for (path, contents) in [
        (
            "items.json",
            r#"{"probe:token":{"minecraft:max_stack_size":16,"minecraft:item_model":"probe:token","probe:marker":"kept"}}"#,
        ),
        ("facts.json", r#"{"states":[]}"#),
        (
            "registries.json",
            r#"{"minecraft:item":{"entries":{"probe:token":{"protocol_id":0}}},"minecraft:data_component_type":{"entries":{}}}"#,
        ),
        (
            "assets/probe/items/token.json",
            r#"{"model":{"type":"minecraft:model","model":"probe:item/token","tints":[{"type":"minecraft:constant","value":16711680}]}}"#,
        ),
        (
            "assets/probe/models/item/token.json",
            r##"{"textures":{"particle":"probe:item/token","all":"probe:item/token"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#all","tintindex":0},"south":{"texture":"#all","tintindex":0},"east":{"texture":"#all","tintindex":0},"west":{"texture":"#all","tintindex":0},"up":{"texture":"#all","tintindex":0},"down":{"texture":"#all","tintindex":0}}}]}"##,
        ),
    ] {
        fs::write(root.join(path), contents)?;
    }
    // An original one-pixel white texture, not a Minecraft asset.
    fs::write(
        root.join("assets/probe/textures/item/token.png"),
        [
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 248, 15, 4,
            0, 9, 251, 3, 253, 251, 94, 107, 43, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ],
    )?;
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            item_ability_default: |_, _, _| false,
            loot_ability: |_, _, _| {},
            loot_table_load: |_, _, _| {},
            modify_default_components: |context, item, _| {
                assert_eq!(context.origin(), DataLoadOrigin::ClientPreload);
                assert_eq!(
                    *context.registry("minecraft:item").unwrap(),
                    ["probe:token"]
                );
                if item == "probe:token" {
                    CALLS.fetch_add(1, Ordering::Relaxed);
                }
            },
            loot_function: |_, _, _, _, _| {},
            loot_condition: |_, _, _, _, _| {},
        })
        .is_ok()
    );
    horizon_protocol::item::custom::install(0, vec!["probe:marker".into()]).unwrap();

    let context = LoadContext::new(
        DataLoadOrigin::ClientPreload,
        Arc::new(LoadInputs::new(
            BTreeMap::from([("minecraft:item".into(), vec!["probe:token".into()])]),
            root.join("settings"),
        )),
        [],
    );
    let item_data = ItemData::load(&root, context)?;
    let defaults = item_data.defaults();
    let items = horizon_items::items::Items::catalog(&root, &PackOverlay::NONE, defaults)?;
    assert_eq!(items.get("probe:token").unwrap().max_stack, 16);
    let library = client::model_library::ParsedModels::load(
        &client::resource_source::ResourceSource::directory(root.clone()),
    )
    .into_item_library();
    let client = ItemRenderer::load_with_library(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        library.clone(),
        &item_data,
    )?;
    let server = ItemRenderer::load_for_server(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        library,
        &item_data,
    )?;
    let _banners = client::banner::Banners::load(&root, defaults);
    let _loot = LootData::load(&root, &PackOverlay::NONE, defaults)?;
    let identity = client::content_identity::identity(&root, defaults)?;
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(client.models.prototypes, server.models.prototypes);
    assert_eq!(
        client.models.prototypes["probe:token"]["minecraft:max_stack_size"],
        defaults.entries()["probe:token"]["minecraft:max_stack_size"]
    );

    let state = client.resolve(
        &StackView::new("probe:token", 1),
        DisplayContext::Gui,
        None,
        None,
        0,
        &NoEnv,
    );
    let sprites = HashMap::from([(
        "probe:item/token".to_owned(),
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        },
    )]);
    let mut quads = Vec::new();
    state.submit(&client, &sprites, &NoHost, Mat4::IDENTITY, &mut quads);
    assert_eq!(
        quads.len(),
        6,
        "parity must compare actual submitted geometry"
    );
    let submitted: Vec<_> = quads
        .iter()
        .map(|q| {
            serde_json::json!({
                "pos": q.pos.map(|p| p.to_array()),
                "uv": q.uv,
                "foil_uv": q.foil_uv,
                "normal": q.normal.to_array(),
                "color": q.color,
                "foil": match q.foil {
                    client::item_render::Foil::None => "none",
                    client::item_render::Foil::Standard => "standard",
                    client::item_render::Foil::Special => "special",
                },
                "light_emission": q.light_emission,
                "cull_back": q.cull_back,
                "translucent": q.translucent,
                "oit": q.oit,
            })
        })
        .collect();
    println!("snapshot calls={}", CALLS.load(Ordering::Relaxed));
    let prototypes = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&*client.models.prototypes)?)
    );
    let rendered = format!("{:x}", Sha256::digest(serde_json::to_vec(&submitted)?));
    let identity = identity
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // Captured before replacing per-reader hooks with a shared snapshot.
    assert_eq!(
        prototypes,
        "589cbe331228acea9effa9530430c764ce788be92f1116410004b84eacf80915"
    );
    assert_eq!(
        rendered,
        "83256455fb1f378382229fb0859d052acac47c54547f1aef4fa14ea086961867"
    );
    assert_eq!(
        identity,
        "0df20aa03226c10182811a27c1cbcd9ad9ba957e48b1713a5ebc6ee8156af0aa"
    );
    fs::remove_dir_all(&root)?;
    Ok(())
}
