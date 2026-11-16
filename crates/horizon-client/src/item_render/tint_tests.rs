use super::*;
use horizon_assets::{
    item_model::{FluidAppearance, register_tint_sources},
    resource_source::ResourceRoot,
};
use std::fs;

/// Red for one, green for two, the entry's own `default` for more.
struct Charge;
impl ItemEnv for Charge {
    fn item_tint_source(
        &self,
        source: &str,
        config: &horizon_core::nbt::Tag,
        stack: &StackView,
    ) -> i32 {
        assert_eq!(source, "tintdraw:charge");
        match stack.count {
            1 => 0xFFCC_2211_u32 as i32,
            2 => 0xFF11_CC22_u32 as i32,
            _ => {
                (match config.get("default") {
                    Some(Tag::Int(v)) => *v,
                    _ => panic!("expected integer default"),
                }) as i32
            }
        }
    }
}

fn renderer(root: &std::path::Path) -> ItemRenderer {
    let write = |name: &str, data: &str| {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    };
    write("items.json", "{}");
    write(
        "assets/minecraft/atlases/items.json",
        r#"{"sources":[{"type":"single","resource":"tintdraw:item/grey"}]}"#,
    );
    super::glint_tests::png(
        &root.join("assets/tintdraw/textures/item/grey.png"),
        16,
        16,
        [128, 128, 128, 255],
    );
    write(
        "assets/tintdraw/items/grey.json",
        r#"{"model":{"type":"minecraft:model","model":"tintdraw:item/grey",
            "tints":[{"type":"tintdraw:charge","default":-16776961}]}}"#,
    );
    write(
        "assets/tintdraw/models/item/grey.json",
        r##"{"gui_light":"front","textures":{"layer0":"tintdraw:item/grey"},"elements":[{"from":[0,0,8],"to":[16,16,8],
            "faces":{"south":{"uv":[0,0,16,16],"texture":"#layer0","tintindex":0}}}]}"##,
    );
    let data = crate::test_resources::item_data(
        root,
        horizon_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    ItemRenderer::load(&ResourceRoot::directory(root.to_path_buf()), &data).unwrap()
}

fn rect() -> Rect {
    Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    }
}

fn stack(count: i32) -> StackView {
    let mut stack = StackView::new("tintdraw:grey", count);
    stack.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("tintdraw:grey")),
    );
    stack
}

/// A greyscale model takes the mod's colour per stack, in hand and in the inventory.
#[test]
fn a_registered_source_tints_each_stack_in_hand_and_in_the_inventory() {
    let root = std::env::temp_dir().join(format!("horizon-item-tint-{}", std::process::id()));
    register_tint_sources(vec!["tintdraw:charge".into()]);
    let renderer = renderer(&root);
    let sprites = HashMap::from([(
        "tintdraw:item/grey".to_owned(),
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        },
    )]);
    let in_hand = |count| {
        let state = renderer.resolve(
            &stack(count),
            DisplayContext::FirstPersonRightHand,
            None,
            None,
            0,
            &Charge,
        );
        let mut quads = Vec::new();
        state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut quads);
        quads.iter().map(|q| q.color).collect::<Vec<_>>()
    };
    assert_eq!(in_hand(1), [0xFFCC_2211]);
    assert_eq!(in_hand(2), [0xFF11_CC22]);
    assert_eq!(in_hand(3), [0xFF00_00FF]);

    let in_inventory = |count| {
        let state = renderer.resolve(&stack(count), DisplayContext::Gui, None, None, 0, &Charge);
        let mut batch = crate::hud::HudBatch::scratch([64, 64]);
        gui::draw(
            &state, &renderer, &sprites, &NoHost, &mut batch, 0.0, 0.0, 0.0, 0.0, 0.0,
        );
        let mut colors: Vec<_> = batch.vertices.iter().map(|v| v.color).collect();
        colors.dedup();
        colors
    };
    // Vertex colours are `0xAABBGGRR`, and the same slot draws each stack's own colour.
    assert_eq!(in_inventory(1), [0xFF11_22CC]);
    assert_eq!(in_inventory(2), [0xFF22_CC11]);
    assert_eq!(in_inventory(1), [0xFF11_22CC]);
    fs::remove_dir_all(root).unwrap();
}

/// The host's answer for `tintdraw:gas`, and for no fluid at all.
struct Gas(Option<&'static str>);
impl ItemEnv for Gas {
    fn stack_fluid(&self, _stack: &StackView) -> Option<&str> {
        self.0
    }
    fn fluid_appearance(&self, fluid: &str) -> Option<FluidAppearance> {
        (fluid == "tintdraw:gas").then(|| FluidAppearance {
            still: "tintdraw:block/gas_still".into(),
            tint: 0xFF22_8844_u32 as i32,
        })
    }
}

/// A fluid container draws its fluid layer, in the fluid's sprite and tint, only
/// while the stack holds a fluid. The tint reaches the layer's front and back; its sides
/// show the mask sprite untinted, as `bakeItemMaskQuads` bakes them.
#[test]
fn a_fluid_containers_fluid_layer_takes_the_fluids_sprite_and_tint() {
    const FLUID_TINT: u32 = 0xFF22_8844;
    let root = std::env::temp_dir().join(format!("horizon-item-fluid-{}", std::process::id()));
    let write = |name: &str, data: &str| {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    };
    write("items.json", "{}");
    write("assets/minecraft/atlases/items.json", r#"{"sources":[]}"#);
    // A fully opaque base and mask: both layers cover the item, so each extrudes
    // 16*4 edge faces and a front and back face.
    super::glint_tests::png(
        &root.join("assets/tintdraw/textures/item/flask.png"),
        16,
        16,
        [0, 0, 0, 255],
    );
    super::glint_tests::png(
        &root.join("assets/tintdraw/textures/item/flask_mask.png"),
        16,
        16,
        [255, 255, 255, 255],
    );
    super::glint_tests::png(
        &root.join("assets/tintdraw/textures/block/gas_still.png"),
        16,
        16,
        [200, 100, 50, 255],
    );
    write(
        "assets/tintdraw/items/flask.json",
        r#"{"model":{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
            "textures":{"base":"tintdraw:item/flask","fluid":"tintdraw:item/flask_mask"}}}"#,
    );
    let data = crate::test_resources::item_data(
        &root,
        horizon_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(&ResourceRoot::directory(root.to_path_buf()), &data).unwrap();
    let mut stack = StackView::new("tintdraw:flask", 1);
    stack.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("tintdraw:flask")),
    );
    let sprites = HashMap::from([
        ("tintdraw:item/flask".to_owned(), rect()),
        ("tintdraw:block/gas_still".to_owned(), rect()),
        ("tintdraw:item/flask_mask".to_owned(), rect()),
    ]);
    let colours = |held: Option<&'static str>| {
        let state = renderer.resolve(&stack, DisplayContext::Gui, None, None, 0, &Gas(held));
        let mut quads = Vec::new();
        state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut quads);
        quads.iter().map(|q| q.color).collect::<Vec<_>>()
    };
    let drawn = colours(Some("tintdraw:gas"));
    // The base's 66 quads and the fluid layer's 64 sides are white; its front and back
    // take the fluid's tint.
    assert_eq!(drawn.iter().filter(|c| **c == 0xFFFF_FFFF).count(), 66 + 64);
    assert_eq!(drawn.iter().filter(|c| **c == FLUID_TINT).count(), 2);
    assert_eq!(colours(None), [0xFFFF_FFFF; 66]);
    fs::remove_dir_all(root).unwrap();
}
