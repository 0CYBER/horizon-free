use super::*;

fn node(json: &str) -> Result<ItemModelNode> {
    ItemModelNode::decode(&serde_json::from_str(json).unwrap())
}
fn container(json: &str) -> FluidContainer {
    match node(json).unwrap() {
        ItemModelNode::FluidContainer(container) => container,
        other => panic!("not a fluid container: {other:?}"),
    }
}

/// The tint this fluid's still sprite is multiplied by: `0xFF223344`.
const GAS_TINT: i32 = 0xFF22_3344u32 as i32;

/// What the host answers for a stack: the fluid it holds and, for `mod:gas` alone,
/// its still sprite and tint.
struct Fluids {
    held: Option<&'static str>,
}
impl ItemEnv for Fluids {
    fn stack_fluid(&self, _stack: &StackView) -> Option<&str> {
        self.held
    }
    fn fluid_appearance(&self, fluid: &str) -> Option<FluidAppearance> {
        (fluid == "mod:gas").then(|| FluidAppearance {
            still: "mod:block/gas_still".into(),
            tint: GAS_TINT,
        })
    }
}

fn layers(definition: &str, env: &dyn ItemEnv) -> Vec<Layer> {
    let mut models = ItemModels::default();
    models.definitions.insert(
        "mod:flask".into(),
        ClientItem::from_json(&format!(r#"{{"model":{definition}}}"#)).unwrap(),
    );
    Arc::make_mut(&mut models.prototypes).insert(
        "mod:flask".into(),
        BTreeMap::from([(
            "minecraft:item_model".into(),
            Value::String("mod:flask".into()),
        )]),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    models
        .resolve(&StackView::new("mod:flask", 1), &context, env)
        .layers
}
fn sprite(sprite: &str, mask: Option<&str>) -> LayerContent {
    LayerContent::Sprite {
        sprite: sprite.into(),
        mask: mask.map(Into::into),
    }
}

#[test]
fn decodes_every_layer_and_the_declared_fluid() {
    let decoded = container(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water",
            "textures":{"particle":"mod:item/flask","base":"mod:item/flask",
                        "fluid":"neoforge:item/mask/bucket_fluid","cover":"mod:item/flask_cover"}}"#,
    );
    assert_eq!(decoded.fluid, "minecraft:water");
    assert!(decoded.cover_is_mask);
    assert_eq!(
        decoded.textures,
        FluidContainerTextures {
            base: Some("mod:item/flask".into()),
            fluid: Some("neoforge:item/mask/bucket_fluid".into()),
            cover: Some("mod:item/flask_cover".into()),
        }
    );
}

#[test]
fn optional_textures_and_cover_is_mask() {
    let decoded = container(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty","cover_is_mask":false,
            "textures":{"base":"mod:item/jar"}}"#,
    );
    assert_eq!(
        decoded.textures,
        FluidContainerTextures {
            base: Some("mod:item/jar".into()),
            fluid: None,
            cover: None,
        }
    );
    assert!(!decoded.cover_is_mask);
    // `particle` alone satisfies the codec's at-least-one-texture rule.
    let decoded = container(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:lava",
            "textures":{"particle":"mod:item/can"}}"#,
    );
    assert_eq!(decoded.textures.base, None);
    assert_eq!(decoded.textures.fluid, None);
    assert_eq!(decoded.textures.cover, None);
}

#[test]
fn bad_shapes_error_instead_of_guessing() {
    for json in [
        // textures is required, and must be an object.
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water"}"#,
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","textures":"mod:item/jar"}"#,
        // ... with at least one of its four textures; a misspelt one is not one.
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","textures":{}}"#,
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","textures":{"bse":"mod:item/jar"}}"#,
        // fluid is required and is an identifier.
        r#"{"type":"neoforge:fluid_container","textures":{"base":"mod:item/jar"}}"#,
        r#"{"type":"neoforge:fluid_container","fluid":"Minecraft:Water","textures":{"base":"mod:item/jar"}}"#,
        // cover_is_mask and the other three flags are booleans.
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","cover_is_mask":"yes","textures":{"base":"mod:item/jar"}}"#,
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","flip_gas":1,"textures":{"base":"mod:item/jar"}}"#,
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","apply_fluid_luminosity":"no","textures":{"base":"mod:item/jar"}}"#,
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","force_opaque_fluid":[true],"textures":{"base":"mod:item/jar"}}"#,
    ] {
        assert!(node(json).is_err(), "{json}");
    }
}

/// The codec's defaults (`DynamicFluidContainerModel$Unbaked`, `optionalFieldOf` at offsets
/// 45-117): `flip_gas` false, `cover_is_mask`, `apply_fluid_luminosity` and
/// `force_opaque_fluid` true; each read as written when set.
#[test]
fn the_flags_default_as_neoforges_codec_does() {
    let defaults = container(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water",
            "textures":{"base":"mod:item/jar"}}"#,
    );
    assert_eq!(
        (
            defaults.flip_gas,
            defaults.cover_is_mask,
            defaults.apply_fluid_luminosity,
            defaults.force_opaque_fluid
        ),
        (false, true, true, true)
    );
    let set = container(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water","flip_gas":true,
            "cover_is_mask":false,"apply_fluid_luminosity":false,"force_opaque_fluid":false,
            "textures":{"base":"mod:item/jar"}}"#,
    );
    assert_eq!(
        (
            set.flip_gas,
            set.cover_is_mask,
            set.apply_fluid_luminosity,
            set.force_opaque_fluid
        ),
        (true, false, false, false)
    );
}

/// `DynamicFluidContainerModel.bakeModelForFluid` passes the cover as the mask and the base as
/// the drawn material (`bakeItemMaskQuads(baker, 0, cover, base, ...)`, offsets 631-639): the
/// cover picks out the parts of the base drawn again over the fluid.
#[test]
fn a_full_container_draws_base_fluid_and_base_through_the_cover_mask() {
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
            "textures":{"base":"mod:item/flask","fluid":"neoforge:item/mask/bucket_fluid",
                        "cover":"mod:item/flask_cover"}}"#,
        &Fluids {
            held: Some("mod:gas"),
        },
    );
    let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
    assert_eq!(
        contents,
        vec![
            sprite("mod:item/flask", None),
            sprite(
                "mod:block/gas_still",
                Some("neoforge:item/mask/bucket_fluid")
            ),
            sprite("mod:item/flask", Some("mod:item/flask_cover")),
        ]
    );
    assert_eq!(drawn[0].tints, Vec::<i32>::new());
    // The fluid's own tint applies to the fluid layer only.
    assert_eq!(drawn[1].tints, vec![GAS_TINT]);
    assert_eq!(drawn[2].tints, Vec::<i32>::new());
    assert!(drawn.iter().all(|l| l.foil == FoilType::None));
}

/// [`Fluids`], counting how often the resolver asks how a fluid draws.
struct CountedFluids {
    fluids: Fluids,
    lookups: std::cell::Cell<usize>,
}
impl ItemEnv for CountedFluids {
    fn stack_fluid(&self, stack: &StackView) -> Option<&str> {
        self.fluids.stack_fluid(stack)
    }
    fn fluid_appearance(&self, fluid: &str) -> Option<FluidAppearance> {
        self.lookups.set(self.lookups.get() + 1);
        self.fluids.fluid_appearance(fluid)
    }
}

/// A stack's held fluid is also the one drawn, so one lookup gives both the sprite and the
/// tint: a held bucket asks once per draw, not once for each.
#[test]
fn a_held_fluid_is_looked_up_once_per_draw() {
    let env = CountedFluids {
        fluids: Fluids {
            held: Some("mod:gas"),
        },
        lookups: std::cell::Cell::new(0),
    };
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
            "textures":{"base":"mod:item/flask","fluid":"neoforge:item/mask/bucket_fluid"}}"#,
        &env,
    );
    assert_eq!(drawn[1].tints, vec![GAS_TINT]);
    assert_eq!(env.lookups.get(), 1);
}

/// `m` applied to a point of the model's 0..1 space.
fn transformed(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        m.0[0][row] * p[0] + m.0[1][row] * p[1] + m.0[2][row] * p[2] + m.0[3][row]
    })
}

/// `FLUID_TRANSFORM` scales the fluid layer by 1.002 in Z and `COVER_TRANSFORM` the cover
/// by 1.004, about the block's middle: the extruded front at z = 8.5/16 moves out to
/// 0.5 + 0.03125 * 1.002 and 0.5 + 0.03125 * 1.004, the back at 7.5/16 the same distance
/// the other way, and X and Y stay put. The base is not scaled.
#[test]
fn the_fluid_and_cover_layers_sit_in_front_of_the_base() {
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
            "textures":{"base":"mod:item/flask","fluid":"neoforge:item/mask/bucket_fluid",
                        "cover":"mod:item/flask_cover"}}"#,
        &Fluids {
            held: Some("mod:gas"),
        },
    );
    assert_eq!(drawn.len(), 3);
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-6);
    let corner = [0.25, 0.75, 8.5 / 16.0];
    let back = [0.25, 0.75, 7.5 / 16.0];
    assert!(close(transformed(&drawn[0].transform, corner), corner));
    assert!(close(
        transformed(&drawn[1].transform, corner),
        [0.25, 0.75, 0.531_312_5]
    ));
    assert!(close(
        transformed(&drawn[1].transform, back),
        [0.25, 0.75, 0.468_687_5]
    ));
    assert!(close(
        transformed(&drawn[2].transform, corner),
        [0.25, 0.75, 0.531_375]
    ));
}

#[test]
fn an_empty_container_draws_its_declared_fluid_or_none() {
    let empty = r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
        "textures":{"base":"mod:item/flask","fluid":"neoforge:item/mask/bucket_fluid"}}"#;
    let drawn = layers(empty, &Fluids { held: None });
    let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
    assert_eq!(contents, vec![sprite("mod:item/flask", None)]);

    // NeoForge falls back to the model's `fluid`: a container with no contents still
    // draws the fluid it declares.
    let declared = r#"{"type":"neoforge:fluid_container","fluid":"mod:gas",
        "textures":{"base":"mod:item/flask","fluid":"neoforge:item/mask/bucket_fluid"}}"#;
    let drawn = layers(declared, &Fluids { held: None });
    let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
    assert_eq!(
        contents,
        vec![
            sprite("mod:item/flask", None),
            sprite(
                "mod:block/gas_still",
                Some("neoforge:item/mask/bucket_fluid")
            ),
        ]
    );
    // ... but `FluidContentsTint` reads the stack, which holds nothing: the empty fluid's
    // model has no tint source, so the declared fluid is drawn white (-1), not in its tint.
    assert_eq!(drawn[1].tints, vec![-1]);
}

/// A fluid with no client model gets `FluidStateModelSet`'s missing model: the missing
/// sprite through the mask, with no tint source, so -1 whether the stack holds it or the
/// model declares it.
#[test]
fn an_unknown_fluid_draws_the_missing_sprite_and_no_mask_no_fluid_layer() {
    for (declared, held) in [("mod:gas", Some("mod:unknown")), ("mod:unknown", None)] {
        let drawn = layers(
            &format!(
                r#"{{"type":"neoforge:fluid_container","fluid":"{declared}",
                    "textures":{{"base":"mod:item/flask","fluid":"neoforge:item/mask/bucket_fluid"}}}}"#
            ),
            &Fluids { held },
        );
        let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
        assert_eq!(
            contents,
            vec![
                sprite("mod:item/flask", None),
                sprite(
                    "minecraft:missingno",
                    Some("neoforge:item/mask/bucket_fluid")
                ),
            ]
        );
        assert_eq!(drawn[1].tints, vec![-1]);
    }

    // The type's `fluid` texture is the mask; without it there is no fluid layer.
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"mod:gas",
            "textures":{"base":"mod:item/flask"}}"#,
        &Fluids {
            held: Some("mod:gas"),
        },
    );
    let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
    assert_eq!(contents, vec![sprite("mod:item/flask", None)]);
}

/// The flask's sprites for a bake: the base is a 2x2 opaque square, the cover a 2x2 mask
/// whose only opaque pixel is the top-left one.
struct FlaskSprites;
impl crate::item_bake::SpriteLookup for FlaskSprites {
    fn mask(&self, sprite: &str) -> Option<crate::item_bake::SpriteMask> {
        let frame = match sprite {
            "mod:item/flask" => vec![false, false, false, false],
            "mod:item/flask_cover" => vec![false, true, true, true],
            _ => return None,
        };
        Some(crate::item_bake::SpriteMask {
            width: 2,
            height: 2,
            frames: vec![frame],
        })
    }
}

/// The cover route end to end, resolved and then baked. NeoForge bakes it with
/// `bakeItemMaskQuads(baker, 0, cover, coverIsMask ? base : cover, state)` (offsets 598-639),
/// whose 5-argument overload bakes the sides from `MaterialInfo.of(mask, ..., -1, ...)`
/// (offsets 17-50) and the front and back from the drawn material's info with tint index 0.
/// So with the default `cover_is_mask`: the cover pixel's 4 sides show the cover sprite with
/// tint -1, and its front and back show the base sprite with tint 0, over the top-left pixel
/// only (x 0..0.5, y 0.5..1; texture 0..8 on both axes).
#[test]
fn the_cover_bakes_its_sides_from_the_cover_and_its_faces_from_the_base() {
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
            "textures":{"base":"mod:item/flask","cover":"mod:item/flask_cover"}}"#,
        &Fluids { held: None },
    );
    assert_eq!(drawn.len(), 2);
    let LayerContent::Sprite { sprite, mask } = &drawn[1].content else {
        panic!("not a sprite layer: {:?}", drawn[1].content);
    };
    let library = crate::resolve::ModelLibrary::with_item_generator(BTreeMap::new()).unwrap();
    let baked =
        crate::item_bake::bake_sprite_layer(&library, &FlaskSprites, sprite, mask.as_deref())
            .unwrap();
    assert_eq!(baked.quads.len(), 4 + 2);
    let mut sides = 0;
    for quad in &baked.quads {
        match quad.facing {
            crate::model::Direction::South | crate::model::Direction::North => {
                assert_eq!(quad.material.sprite, "mod:item/flask");
                assert_eq!(quad.tint_index, 0);
                let (xs, ys): (Vec<f32>, Vec<f32>) =
                    quad.positions.iter().map(|p| (p[0], p[1])).unzip();
                let span = |v: &[f32]| {
                    let min = v.iter().copied().fold(f32::INFINITY, f32::min);
                    let max = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    [min, max]
                };
                assert_eq!(span(&xs), [0.0, 0.5]);
                assert_eq!(span(&ys), [0.5, 1.0]);
                for uv in &quad.uv {
                    assert!(uv.iter().all(|c| (0.0..=0.5).contains(c)), "{uv:?}");
                }
            }
            _ => {
                sides += 1;
                assert_eq!(quad.material.sprite, "mod:item/flask_cover");
                assert_eq!(quad.tint_index, -1);
            }
        }
    }
    assert_eq!(sides, 4);
}

#[test]
fn a_cover_is_a_mask_only_over_a_base() {
    // Without `cover_is_mask` the cover is drawn through itself, with or without a base
    // (`coverIsMask ? base : cover` at offsets 598-615, the cover still the mask)...
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty","cover_is_mask":false,
            "textures":{"cover":"mod:item/flask_cover"}}"#,
        &Fluids { held: None },
    );
    let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
    assert_eq!(
        contents,
        vec![sprite("mod:item/flask_cover", Some("mod:item/flask_cover"))]
    );
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty","cover_is_mask":false,
            "textures":{"base":"mod:item/flask","cover":"mod:item/flask_cover"}}"#,
        &Fluids { held: None },
    );
    let contents: Vec<LayerContent> = drawn.iter().map(|l| l.content.clone()).collect();
    assert_eq!(
        contents,
        vec![
            sprite("mod:item/flask", None),
            sprite("mod:item/flask_cover", Some("mod:item/flask_cover")),
        ]
    );

    // ... while a mask cover needs the base it masks and is left out without one.
    let drawn = layers(
        r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
            "textures":{"cover":"mod:item/flask_cover"}}"#,
        &Fluids { held: None },
    );
    assert!(drawn.is_empty());
}
