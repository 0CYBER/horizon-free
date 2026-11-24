//! Native subscriptions, real sky materials and texture readback on one offscreen world view.
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use super::{Frame, HudBatch, Renderer, loading_frame};
use glam::{Mat4, Vec3};
use horizon_client::environment::Sky;
use horizon_client::gfx::ScreenshotRequest;
use horizon_client::hud::GuiScale;
use horizon_client_render::core_shader::{CoreProgram, Glue, translate_core};
use mod_api::{
    BuildStarsEvent, Flow, Hooks, MAX_STAR_VERTICES, ModEntry, RenderStarsEvent, StarMeshRefusal,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

const SIZE: [u32; 2] = [320, 240];
const ANGLE: f32 = 37.0;
const BRIGHTNESS: f32 = 0.5;

#[derive(Default)]
struct Probe {
    builds: AtomicUsize,
    draws: AtomicUsize,
    edit: AtomicBool,
    cancel: AtomicBool,
}

struct Stars(Arc<Probe>);

#[mod_api::__subscriptions]
impl Hooks for Stars {
    fn build_stars(&self, event: &mut BuildStarsEvent<'_, '_>) {
        self.0.builds.fetch_add(1, Relaxed);
        // Seed 10842, 780 of the 1500 candidates pass the sphere test.
        assert_eq!(event.mesh.vertices().len(), 4680);
        assert!(
            event
                .mesh
                .vertices()
                .iter()
                .all(|p| { (99.9..100.1).contains(&Vec3::from_array(*p).length()) })
        );
        if self.0.edit.load(Relaxed) {
            // The camera at yaw 90 faces -X. Put a single rectangle 100 blocks ahead;
            // upload it in *star* space, not already in its draw pose.
            let from_sky = pose().transpose();
            let world = [
                [-100.0, -10.0, -16.0],
                [-100.0, -10.0, 16.0],
                [-100.0, 10.0, 16.0],
                [-100.0, 10.0, -16.0],
            ];
            let marker = [0, 1, 2, 0, 2, 3].map(|i| {
                from_sky
                    .transform_point3(Vec3::from_array(world[i]))
                    .to_array()
            });
            event.mesh.replace(marker.to_vec()).unwrap();
            assert_eq!(
                event.mesh.replace(vec![[0.0; 3]]),
                Err(StarMeshRefusal::IncompleteTriangle)
            );
            assert_eq!(
                event.mesh.replace(vec![[f32::NAN; 3]; 3]),
                Err(StarMeshRefusal::PositionOutOfRange)
            );
            assert_eq!(
                event.mesh.replace(vec![[0.0; 3]; MAX_STAR_VERTICES + 3]),
                Err(StarMeshRefusal::TooManyVertices)
            );
            assert_eq!(event.mesh.vertices(), &marker);
        }
    }

    fn render_stars(&self, event: &mut RenderStarsEvent) -> Flow {
        self.0.draws.fetch_add(1, Relaxed);
        assert_eq!(event.angle_degrees, ANGLE);
        assert_eq!(event.brightness, BRIGHTNESS);
        assert!(Mat4::from_cols_array_2d(&event.sky_from_stars).abs_diff_eq(pose(), 1e-6));
        if self.0.cancel.load(Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn pose() -> Mat4 {
    Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_2) * Mat4::from_rotation_x(ANGLE.to_radians())
}

fn read_rgb(renderer: &mut Renderer, frame: &Frame<'_>) -> Vec<[u8; 3]> {
    renderer.screenshot = Some(ScreenshotRequest {
        path: "star-stage-unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(frame).unwrap();
    let mut pending = renderer.screenshot_result.take().unwrap().unwrap();
    let image = pending.image_mut();
    assert_eq!([image.width, image.height], SIZE);
    image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn core_stars() -> CoreProgram {
    // The core stars material receives the same positions but paints their coverage green.
    translate_core(
        "#version 330\nin vec3 Position;\n\
         layout(std140) uniform Projection { mat4 ProjMat; };\n\
         layout(std140) uniform DynamicTransforms { mat4 ModelViewMat; };\n\
         void main() { gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0); }",
        "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(0.0, 0.25, 0.0, 1.0); }",
        &[],
        &|_| None,
        &Glue {
            inputs: &[("Position", "vec3")],
            prelude: "layout(location = 0) in vec3 hz_position;",
            setup: "Position = hz_position;",
            finish: "",
            vertex_id: "int(gl_VertexIndex)",
        },
    )
    .unwrap()
}

#[test]
#[ignore = "daily"]
fn star_build_and_cancellation_reach_world_pixels() {
    let name = "game::star_stage_tests::star_build_and_cancellation_reach_world_pixels";
    if run_fresh_cases(name, "HORIZON_STAR_STAGE_CHILD", &["pixels"]) {
        return;
    }
    with_client_game(|game| {
        let mut builder = winit::event_loop::EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11().with_any_thread(true);
        }
        let event_loop = builder.build().unwrap();
        #[allow(deprecated)]
        let window = Arc::new(
            event_loop
                .create_window(
                    winit::window::Window::default_attributes()
                        .with_visible(false)
                        .with_inner_size(winit::dpi::PhysicalSize::new(SIZE[0], SIZE[1])),
                )
                .unwrap(),
        );
        game.options.offscreen = true;
        let hud = HudBatch::new(
            SIZE,
            GuiScale {
                requested: 1,
                unicode: false,
            },
        );
        let sky = Sky {
            star_angle: ANGLE,
            star_brightness: BRIGHTNESS,
            sun_angle: 55.0,
            moon_angle: 125.0,
            ..Sky::default()
        };
        let (celestial, stars) = game.celestial_geometry(&sky, 0.0);
        assert_eq!(celestial.len(), 8, "the sun and full moon loaded");
        let mut frame = Frame {
            title: false,
            skybox: true,
            yaw: 90.0,
            pitch: 0.0,
            eye_rotation: [90.0, 0.0],
            fov_degrees: 100.0,
            sky,
            ..loading_frame(&hud)
        };
        let mut renderer = game.new_renderer(window.clone()).unwrap();
        let blank = read_rgb(&mut renderer, &frame);
        frame.celestial = &celestial;
        let bodies = read_rgb(&mut renderer, &frame);
        let width = SIZE[0] as usize;
        let middle = SIZE[1] as usize / 2 * width;
        let center = middle + width / 2;
        let differences =
            |a: &[[u8; 3]], b: &[[u8; 3]]| a.iter().zip(b).filter(|(a, b)| a != b).count();
        assert!(
            differences(&blank[..middle], &bodies[..middle]) > 100,
            "the sun draws above the centre"
        );
        assert!(
            differences(&blank[middle..], &bodies[middle..]) > 100,
            "the moon draws below the centre"
        );
        frame.stars = stars;
        let stock = read_rgb(&mut renderer, &frame);
        let stock_pixels = differences(&stock, &bodies);
        assert!(
            (1..1000).contains(&stock_pixels),
            "stock stars change {stock_pixels} pixels"
        );
        assert_eq!(
            stock,
            read_rgb(&mut renderer, &frame),
            "unlistened star geometry is stable"
        );
        game.renderer = Some(renderer);

        let probe = Arc::new(Probe::default());
        let mods = vec![ModEntry::new("sky_probe", Box::new(Stars(probe.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut renderer = game.new_renderer(window.clone()).unwrap();
            frame.title = true;
            read_rgb(&mut renderer, &frame);
            frame.title = false;
            frame.skybox = false;
            read_rgb(&mut renderer, &frame);
            frame.skybox = true;
            let night = frame.stars.take();
            assert_eq!(bodies, read_rgb(&mut renderer, &frame));
            frame.stars = night;
            frame.sky_blocked = true;
            read_rgb(&mut renderer, &frame);
            frame.sky_blocked = false;
            assert_eq!(
                (probe.builds.load(Relaxed), probe.draws.load(Relaxed)),
                (0, 0)
            );
            assert_eq!(
                stock,
                read_rgb(&mut renderer, &frame),
                "a build listener preserving the mesh draws stock pixels"
            );
            assert_eq!(stock, read_rgb(&mut renderer, &frame));
            assert_eq!(
                (probe.builds.load(Relaxed), probe.draws.load(Relaxed)),
                (1, 2)
            );

            probe.edit.store(true, Relaxed);
            game.renderer = Some(renderer);
            let mut renderer = game.new_renderer(window).unwrap();
            let edited = read_rgb(&mut renderer, &frame);
            let added = std::array::from_fn::<_, 3, _>(|i| {
                i16::from(edited[center][i]) - i16::from(bodies[center][i])
            });
            assert!(
                added[0] >= 32 && added.iter().all(|v| v.abs_diff(added[0]) <= 1),
                "the replacement adds white star light at the centre: {added:?}"
            );
            let edited_pixels = differences(&edited, &bodies);
            assert!(
                edited_pixels > stock_pixels,
                "one rectangle replaces the small stock points"
            );
            for (i, (edited, original)) in edited.iter().zip(&bodies).enumerate() {
                if !(125..195).contains(&(i % width)) || !(100..140).contains(&(i / width)) {
                    assert_eq!(
                        edited, original,
                        "no stock stars outside the replacement at pixel {i}"
                    );
                }
            }
            probe.cancel.store(true, Relaxed);
            assert_eq!(
                bodies,
                read_rgb(&mut renderer, &frame),
                "cancellation removes only stars"
            );

            renderer.set_sky_shaders(None, Some(&core_stars()), None);
            probe.cancel.store(false, Relaxed);
            let shaded = read_rgb(&mut renderer, &frame);
            assert_eq!(shaded[center][0], bodies[center][0]);
            assert!(shaded[center][1] > bodies[center][1]);
            assert_eq!(shaded[center][2], bodies[center][2]);
            probe.cancel.store(true, Relaxed);
            assert_eq!(
                bodies,
                read_rgb(&mut renderer, &frame),
                "cancellation also skips core/stars"
            );
            assert_eq!(
                (probe.builds.load(Relaxed), probe.draws.load(Relaxed)),
                (2, 6)
            );
            eprintln!(
                "star stage: 4680 stock vertices; {stock_pixels} stock pixels; 6 replacement vertices, {edited_pixels} replacement pixels; centre stock={:?} edited={:?} core={:?} cancelled={:?}; builds=2 draws=6",
                stock[center], edited[center], shaded[center], bodies[center]
            );
        });
    });
}
