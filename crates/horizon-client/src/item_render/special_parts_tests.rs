//! A special model of a type a mod registered draws the resource models its renderer answers
//! (`ItemEnv::special_model_parts`), with the `base` model's display transforms, particle and GUI
//! light, in every display context and after a reload. Original assets: one cube A, two cubes B.
use super::*;
use horizon_assets::{
    item_model::{PreparedParts, SpecialModelSlot, SpecialPart, register_special_model_types},
    resource_source::ResourceRoot,
};
use std::fs;

/// A for a stack of one, B for more, tinted; a stack of three names a model no pack has, and one
/// of four is refused (the renderer failed); stacks of six and seven draw A tinted white and grey
/// (encoded 0.5). The answer is prepared by the renderer that draws it.
struct Pan<'r>(&'r ItemRenderer);
impl ItemEnv for Pan<'_> {
    fn special_model_parts(
        &self,
        slot: SpecialModelSlot,
        base: &str,
        config: &Arc<horizon_core::nbt::Tag>,
        stack: &StackView,
    ) -> Option<PreparedParts> {
        assert_eq!((slot, base), (SpecialModelSlot(0), "spt:item/base"));
        assert_eq!(config.get("handle"), Some(&horizon_core::nbt::Tag::Byte(2)));
        let part = |model: &str| SpecialPart {
            model: model.to_owned(),
            pose: horizon_assets::math::Mat4::translation([0.25, 0.0, 0.0]),
            tints: vec![0xFFCC_2211_u32 as i32],
        };
        let parts = match stack.count {
            1 => vec![part("spt:item/a")],
            3 => vec![part("spt:item/a"), part("spt:item/nope")],
            4 => return None,
            5 => vec![SpecialPart {
                pose: horizon_assets::math::Mat4::IDENTITY.scale([2.0; 3]),
                ..part("spt:item/a")
            }],
            6 | 7 => vec![SpecialPart {
                tints: vec![[WHITE, GREY][stack.count as usize - 6]],
                ..part("spt:item/a")
            }],
            _ => vec![part("spt:item/b")],
        };
        Some(self.0.prepare_parts(base, &parts))
    }
}

/// White and an encoded 0.5 grey, packed vanilla-space ARGB.
const WHITE: i32 = 0xFFFF_FFFF_u32 as i32;
const GREY: i32 = 0xFF80_8080_u32 as i32;

/// `n` separated 4-pixel cubes, every face tinted by index 0.
fn cubes(n: usize) -> String {
    let faces = ["north", "east", "south", "west", "up", "down"]
        .map(|f| format!(r##""{f}":{{"uv":[0,0,4,4],"texture":"#all","tintindex":0}}"##))
        .join(",");
    let elements = (0..n)
        .map(|i| {
            let x = i * 5;
            format!(
                r#"{{"from":[{x},0,0],"to":[{},4,4],"faces":{{{faces}}}}}"#,
                x + 4
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"textures":{{"all":"spt:item/grey"}},"elements":[{elements}]}}"#)
}

fn write(root: &std::path::Path, name: &str, data: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// The packs in `root`, with B of `b_cubes` cubes.
fn renderer(root: &std::path::Path, b_cubes: usize) -> ItemRenderer {
    write(root, "items.json", "{}");
    write(
        root,
        "assets/minecraft/atlases/items.json",
        r#"{"sources":[{"type":"single","resource":"spt:item/grey"},{"type":"single","resource":"spt:item/flour"}]}"#,
    );
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/grey.png"),
        16,
        16,
        [128, 128, 128, 255],
    );
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/flour.png"),
        16,
        16,
        [250, 250, 250, 255],
    );
    write(
        root,
        "assets/spt/items/pan.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:pan","handle":2}}}"#,
    );
    write(
        root,
        "assets/spt/items/cube.json",
        r#"{"model":{"type":"minecraft:model","model":"spt:item/a"}}"#,
    );
    // Cube A tinted white or grey by vanilla's constant tint source.
    for (name, tint) in [("white_cube", WHITE), ("grey_cube", GREY)] {
        write(
            root,
            &format!("assets/spt/items/{name}.json"),
            &format!(
                r#"{{"model":{{"type":"minecraft:model","model":"spt:item/a","tints":[{{"type":"minecraft:constant","value":{tint}}}]}}}}"#
            ),
        );
    }
    write(
        root,
        "assets/spt/items/stranger.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:unregistered"}}}"#,
    );
    // The base has no geometry of its own: its display transforms differ by context, its
    // particle is flour and its GUI light is flat.
    write(
        root,
        "assets/spt/models/item/base.json",
        r#"{"gui_light":"front","textures":{"particle":"spt:item/flour"},"display":{
            "ground":{"scale":[0.5,0.5,0.5]},
            "fixed":{"rotation":[0,180,0]},
            "head":{"translation":[0,13,7]},
            "thirdperson_righthand":{"rotation":[75,45,0],"translation":[0,2.5,0]},
            "gui":{"rotation":[30,225,0],"scale":[0.625,0.625,0.625]}}}"#,
    );
    write(root, "assets/spt/models/item/a.json", &cubes(1));
    write(root, "assets/spt/models/item/b.json", &cubes(b_cubes));
    let data = crate::test_resources::item_data(
        root,
        horizon_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    ItemRenderer::load(&ResourceRoot::directory(root.to_path_buf()), &data).unwrap()
}

fn stack(item: &str, count: i32) -> StackView {
    let mut stack = StackView::new(item, count);
    stack
        .patch
        .insert("minecraft:item_model".into(), Some(serde_json::json!(item)));
    stack
}

fn sprites() -> HashMap<String, Rect> {
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    HashMap::from([
        ("spt:item/grey".to_owned(), rect),
        ("spt:item/flour".to_owned(), rect),
    ])
}

fn quads(renderer: &ItemRenderer, state: &ItemRenderState) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    state.submit(renderer, &sprites(), &NoHost, Mat4::IDENTITY, &mut quads);
    quads
}

#[test]
#[ignore = "daily"]
fn a_registered_special_model_draws_its_renderers_resource_models() {
    let root = std::env::temp_dir().join(format!("horizon-special-parts-{}", std::process::id()));
    register_special_model_types(vec!["spt:pan".into()]);
    let renderer = renderer(&root, 2);
    let resolve = |item: &str, count, display| {
        renderer.resolve(&stack(item, count), display, None, None, 0, &Pan(&renderer))
    };

    // A or B, in the inventory and in hand, tinted and with the base's particle and GUI light.
    for display in [DisplayContext::Gui, DisplayContext::FirstPersonRightHand] {
        let one = resolve("spt:pan", 1, display);
        let two = resolve("spt:pan", 2, display);
        assert_eq!(quads(&renderer, &one).len(), 6, "{display:?}");
        assert_eq!(quads(&renderer, &two).len(), 12, "{display:?}");
        assert!(
            quads(&renderer, &two)
                .iter()
                .all(|q| q.color == 0xFFCC_2211)
        );
        assert_eq!(two.layers[0].particle, "spt:item/flour");
        assert!(!two.uses_block_light());
        assert!(
            two.animated,
            "a mod's drawing is never kept as a static GUI item"
        );
    }

    // B on the ground, in a frame, on a head and in another player's hand: the base's
    // transform for each context, so each draws B somewhere else.
    let contexts = [
        DisplayContext::Ground,
        DisplayContext::Fixed,
        DisplayContext::Head,
        DisplayContext::ThirdPersonRightHand,
    ];
    let matrices: Vec<Mat4> = contexts
        .iter()
        .map(|&display| {
            let state = resolve("spt:pan", 2, display);
            assert_eq!(quads(&renderer, &state).len(), 12, "{display:?}");
            state.layers[0].matrix
        })
        .collect();
    for (i, a) in matrices.iter().enumerate() {
        for b in &matrices[i + 1..] {
            assert_ne!(a, b);
        }
    }
    // A part's pose is about the model's corner, as its elements are: A's cube spans x 0 to 1/4
    // there, moved a quarter block or doubled; the drawn item then sits half a block back.
    let x = |count| {
        let (lo, hi) = resolve("spt:pan", count, DisplayContext::None).bounding_box();
        (lo.x, hi.x)
    };
    assert_eq!(x(1), (-0.25, 0.0));
    assert_eq!(x(5), (-0.5, 0.0));

    // A part naming a model no pack has, or a refused draw, draws the missing model; a plain
    // cube item still draws its 6 quads.
    let missing = renderer.bake(MISSING_MODEL);
    let is_missing = |state: &ItemRenderState| {
        matches!(&state.layers[..], [layer]
            if matches!(&layer.geometry, LayerGeometry::Quads(baked) if Arc::ptr_eq(baked, &missing)))
    };
    assert!(is_missing(&resolve("spt:pan", 3, DisplayContext::Gui)));
    assert!(is_missing(&resolve("spt:pan", 4, DisplayContext::Gui)));
    assert!(!is_missing(&resolve("spt:pan", 1, DisplayContext::Gui)));
    assert_eq!(
        quads(&renderer, &resolve("spt:cube", 1, DisplayContext::Gui)).len(),
        6
    );

    // A special type no mod registered isn't loaded: its item draws the missing model.
    assert!(is_missing(&resolve("spt:stranger", 1, DisplayContext::Gui)));

    // A reload that makes B three cubes draws 18 quads for the same stack.
    let reloaded = super::special_parts_tests::renderer(&root, 3);
    let state = reloaded.resolve(
        &stack("spt:pan", 2),
        DisplayContext::Gui,
        None,
        None,
        0,
        &Pan(&reloaded),
    );
    assert_eq!(quads(&reloaded, &state).len(), 18);
    fs::remove_dir_all(root).unwrap();
}

/// Row 5 of the matrix (`docs/design/mod-api/resource-renderers.md`), an old state across a
/// reload. A part's quads are CPU vertices submitted every frame, not prepared GPU models, so the
/// state's own `Arc<BakedItemModel>` is the only lease and there is no retirement step: a state
/// resolved before a reload still draws its 12 quads after it, the old baked model is released
/// exactly once, when the last state holding it drops, and a new resolve draws the reload's 18.
#[test]
#[ignore = "daily"]
fn an_old_state_draws_across_a_reload_and_releases_its_models_once() {
    let root = std::env::temp_dir().join(format!(
        "horizon-special-parts-lease-{}",
        std::process::id()
    ));
    register_special_model_types(vec!["spt:pan".into()]);
    let before = renderer(&root, 2);
    let resolve = |renderer: &ItemRenderer| {
        renderer.resolve(
            &stack("spt:pan", 2),
            DisplayContext::Gui,
            None,
            None,
            0,
            &Pan(renderer),
        )
    };
    let (old, also_old) = (resolve(&before), resolve(&before));
    let LayerGeometry::Quads(b) = &old.layers[0].geometry else {
        panic!("not quads: {:?}", old.layers[0].geometry);
    };
    let b = Arc::downgrade(b);
    let after = renderer(&root, 3);
    drop(before);
    assert_eq!(quads(&after, &old).len(), 12);
    assert_eq!(quads(&after, &resolve(&after)).len(), 18);
    assert_eq!(b.strong_count(), 2, "the two old states hold the old B");
    drop(old);
    assert_eq!(b.strong_count(), 1);
    assert_eq!(quads(&after, &also_old).len(), 12);
    drop(also_old);
    assert_eq!(b.strong_count(), 0, "the old B outlived its last state");
    fs::remove_dir_all(root).unwrap();
}

/// Row 11 of the matrix, a part's tint. It is packed vanilla-space ARGB, as an item model's
/// tints are: white and an encoded 0.5 reach the vertices as the colour vanilla's constant tint
/// source gives the same cube, not decoded, so the frame decodes them once (vanilla's power 2.2:
/// shader values 1 and about 0.21763764, pinned by `gfx::linear_blend_tests`). A value above 1 has
/// no input.
#[test]
#[ignore = "daily"]
fn a_parts_tint_reaches_the_vertices_as_a_vanilla_tint_does() {
    let root =
        std::env::temp_dir().join(format!("horizon-special-parts-tint-{}", std::process::id()));
    register_special_model_types(vec!["spt:pan".into()]);
    let renderer = renderer(&root, 2);
    let colours = |item: &str, count| {
        let state = renderer.resolve(
            &stack(item, count),
            DisplayContext::Gui,
            None,
            None,
            0,
            &Pan(&renderer),
        );
        quads(&renderer, &state)
            .iter()
            .map(|q| q.color)
            .collect::<Vec<_>>()
    };
    for (count, vanilla, argb) in [(6, "spt:white_cube", WHITE), (7, "spt:grey_cube", GREY)] {
        let part = colours("spt:pan", count);
        assert_eq!(part, colours(vanilla, 1), "{vanilla}");
        assert_eq!(part, vec![argb as u32; 6], "{vanilla}");
    }
    fs::remove_dir_all(root).unwrap();
}
