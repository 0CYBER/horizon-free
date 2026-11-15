//! Still-fluid boxes under a block entity's pose (`BlockEntityDrawContext::fluid_box`), drawn
//! with their own sorted translucent material (`FLUID_OVERLAY`).
use super::{Game, native_block_entity_draw::BlockEntityDraw};
use glam::Vec3;
use horizon_client::{
    fluid_render::{NativeFluidRenderOptions, fluid_box, fluid_light},
    gfx::FLUID_OVERLAY,
    mesher::Vertex,
};
use mod_api::{FluidBoxError, FluidBoxOptions, FluidStack};

impl Game<'_> {
    /// Draws `stack` as a still fluid filling `bounds` under `draw`'s pose, sampling the fluid's
    /// still sprite from the block sprite arrays. See `BlockEntityDrawContext::fluid_box`.
    pub(super) fn native_draw_fluid_box(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &FluidStack,
        bounds: [f32; 6],
        options: FluidBoxOptions,
    ) -> Result<(), FluidBoxError> {
        let layers = &self.pack.render.sprite_layers;
        draw_fluid_box(
            draw,
            |sprite| {
                layers
                    .get(sprite)
                    .or_else(|| layers.get(sprite.strip_prefix("minecraft:")?))
                    .copied()
            },
            stack,
            bounds,
            options,
        )
    }
}

/// The still sprite, `0xAARRGGBB` stack tint and luminosity of a fluid the engine draws.
/// Components and other fluids are refused rather than guessed.
fn fluid_visual(stack: &FluidStack) -> Option<(&'static str, u32, u8)> {
    if stack.has_components() {
        return None;
    }
    match mod_api::__private::fluid_id(stack.key()?)? {
        "minecraft:water" => Some(("minecraft:block/water_still", 0xFF3F_76E4, 0)),
        "minecraft:lava" => Some(("minecraft:block/lava_still", 0xFFFF_FFFF, 15)),
        _ => None,
    }
}

fn draw_fluid_box(
    draw: &mut BlockEntityDraw,
    sprite_layer: impl FnOnce(&str) -> Option<u32>,
    stack: &FluidStack,
    bounds: [f32; 6],
    options: FluidBoxOptions,
) -> Result<(), FluidBoxError> {
    let (sprite, tint, luminosity) = fluid_visual(stack).ok_or(FluidBoxError::UnsupportedFluid)?;
    if stack.amount == 0 {
        return Ok(());
    }
    let layer = sprite_layer(sprite).ok_or(FluidBoxError::MissingSprite)?;
    let pose = draw.pose();
    let light = fluid_light(draw.light(), luminosity);
    let argb = options.argb.unwrap_or(tint);
    // The vertex colour is ABGR.
    let color = (argb & 0xFF00_FF00) | ((argb >> 16) & 0xFF) | ((argb & 0xFF) << 16);
    let options = NativeFluidRenderOptions {
        argb: options.argb,
        render_bottom: options.render_bottom,
        uv_shrink_ratio: options.uv_shrink_ratio,
    };
    let vertices = draw.vertices_mut();
    let drawn = fluid_box(bounds, options, |v| {
        vertices.push(Vertex {
            pos: pose.transform_point3(Vec3::from_array(v.pos)).to_array(),
            uv: v.uv,
            layer: FLUID_OVERLAY,
            color,
            light,
            // The source TRANSLUCENT shader does not use normals for lighting.
            normal: [0; 4],
            glint_uv: [layer as f32, 0.0],
        });
    });
    drawn.then_some(()).ok_or(FluidBoxError::InvalidBox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;
    use horizon_client::block_entity_render::diffuse_lights;

    fn stack(fluid: &str, amount: u32) -> FluidStack {
        mod_platform::__private::part_tests::game_thread();
        FluidStack::new(mod_api::__private::fluid_key(fluid).unwrap(), amount)
    }

    /// A frame a block east of the camera, lit by sky 12 and block 3.
    fn frame() -> BlockEntityDraw {
        BlockEntityDraw::posed(
            0.0,
            0,
            0x30C0,
            [0; 3],
            Mat4::from_translation(Vec3::X),
            diffuse_lights(false),
            Vec::new(),
        )
    }

    fn drawn(
        stack: &FluidStack,
        bounds: [f32; 6],
        options: FluidBoxOptions,
    ) -> (Result<(), FluidBoxError>, Vec<Vertex>) {
        let mut draw = frame();
        let result = draw_fluid_box(&mut draw, |_| Some(7), stack, bounds, options);
        (result, draw.into_vertices())
    }

    #[test]
    fn water_tiles_under_the_pose_in_its_own_material_with_the_stack_tint() {
        let (result, vertices) = drawn(
            &stack("minecraft:water", 1000),
            [0.0, 0.0, 0.0, 2.0, 1.0, 1.0],
            FluidBoxOptions::new(false, 0.0),
        );
        assert_eq!(result, Ok(()));
        // Two tiles on top, front, back; one on each end; the bottom left out.
        assert_eq!(vertices.len(), (2 + 2 + 2 + 1 + 1) * 4);
        assert!(vertices.iter().all(|v| v.layer == FLUID_OVERLAY
            && v.glint_uv == [7.0, 0.0]
            && v.color == 0xFFE4_763F
            && v.light == 0x30C0));
        // The pose moved the box a block east: x spans 1..=3.
        let xs = vertices.iter().map(|v| v.pos[0]);
        assert_eq!(xs.clone().fold(f32::MAX, f32::min), 1.0);
        assert_eq!(xs.fold(f32::MIN, f32::max), 3.0);
    }

    #[test]
    fn lava_raises_block_light_and_an_explicit_tint_replaces_the_stack_tint() {
        let (result, vertices) = drawn(
            &stack("minecraft:lava", 1),
            [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            FluidBoxOptions::new(true, 0.0).with_argb(0x8011_2233),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(vertices.len(), 6 * 4);
        // Sky 12 kept, block light 3 raised to lava's 15.
        assert!(
            vertices
                .iter()
                .all(|v| v.light == 0xF0C0 && v.color == 0x8033_2211)
        );
    }

    #[test]
    fn refusals_and_empty_stacks_append_nothing() {
        let unit = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let options = FluidBoxOptions::new(true, 0.0);
        let with_components = {
            use horizon_core::nbt::{Compound, Tag};
            let mut components = Compound::new();
            components.insert(
                "minecraft:custom_data".into(),
                Tag::Compound(Compound::new()),
            );
            FluidStack {
                components: Some(Box::new(components.into())),
                ..stack("minecraft:water", 1000)
            }
        };
        let cases = [
            (
                stack("test:potion", 1000),
                unit,
                options,
                Err(FluidBoxError::UnsupportedFluid),
            ),
            (
                with_components,
                unit,
                options,
                Err(FluidBoxError::UnsupportedFluid),
            ),
            (
                stack("minecraft:water", 1000),
                [1.0, 0.0, 0.0, 0.0, 1.0, 1.0],
                options,
                Err(FluidBoxError::InvalidBox),
            ),
            (
                stack("minecraft:water", 1000),
                [0.0, 0.0, 0.0, f32::NAN, 1.0, 1.0],
                options,
                Err(FluidBoxError::InvalidBox),
            ),
            (
                stack("minecraft:water", 1000),
                [0.0, 0.0, 0.0, 1_000.0, 1_000.0, 1_000.0],
                options,
                Err(FluidBoxError::InvalidBox),
            ),
            (
                stack("minecraft:water", 1000),
                unit,
                FluidBoxOptions::new(true, 1.5),
                Err(FluidBoxError::InvalidBox),
            ),
            // An empty stack is drawn as nothing without its bounds being read.
            (stack("minecraft:water", 0), [f32::NAN; 6], options, Ok(())),
        ];
        for (stack, bounds, options, expected) in cases {
            let (result, vertices) = drawn(&stack, bounds, options);
            assert_eq!(result, expected, "{bounds:?} {options:?}");
            assert!(vertices.is_empty(), "{bounds:?} {options:?}");
        }
        let mut draw = frame();
        assert_eq!(
            draw_fluid_box(
                &mut draw,
                |_| None,
                &stack("minecraft:lava", 1000),
                unit,
                options
            ),
            Err(FluidBoxError::MissingSprite)
        );
        assert!(draw.into_vertices().is_empty());
    }
}
