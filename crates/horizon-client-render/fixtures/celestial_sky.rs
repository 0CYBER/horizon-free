//! Asset-free, main-world pixel fixture. Run `python3 tools/run_world_views.py OUT --celestial`.
use super::{GUI, fixture_frame, read, wall};
use glam::{DVec3, Vec3};
use horizon_client_render::{
    gfx::{Frame, Renderer, ScreenshotRequest},
    hud::HudBatch,
    render_slots::celestial_sky::{self, CelestialDrawError, CelestialSky, CelestialVertex},
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

const SIZE: [u32; 2] = [320, 180];
const OFF: usize = 0;
const EMPTY: usize = 1;
const DRAW: usize = 2;
static MODE: AtomicUsize = AtomicUsize::new(EMPTY);
static CALLED: AtomicUsize = AtomicUsize::new(0);

/// A white sprite with a cut-out strip and a translucent side; the terrain atlas stays white.
pub(super) fn atlas() -> [u8; 16 * 16 * 4] {
    let mut pixels = [255; 16 * 16 * 4];
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        pixel[3] = match index % 16 {
            6..10 => 0,
            10.. => 128,
            _ => 255,
        };
    }
    pixels
}

pub(super) fn install() {
    celestial_sky::install(|| match MODE.load(Relaxed) {
        OFF => None,
        EMPTY => Some(|_| {
            CALLED.fetch_add(1, Relaxed);
        }),
        _ => Some(draw),
    });
}

fn draw(sky: &mut CelestialSky<'_>) {
    CALLED.fetch_add(1, Relaxed);
    let rotation = sky.star_rotation();
    // At angle zero the panel sits 80 blocks south, beyond this frame's terrain far plane.
    // The star-space X axis faces south: increasing its angle rolls the panel in the picture.
    let corners = [[-62.0, 40.0], [62.0, 40.0], [62.0, -40.0], [-62.0, -40.0]];
    let uvs = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let color = [[0.1, 1.0, 0.2, 1.0]; 2];
    let vertices = [0, 1, 2, 0, 2, 3].map(|i| {
        let [x, y] = corners[i];
        CelestialVertex {
            position: rotation
                .transform_vector3(Vec3::new(80.0, y, -x))
                .to_array(),
            uv: uvs[i],
            color: if i < 2 {
                color[i]
            } else {
                [1.0, 0.1, 0.15, 1.0]
            },
        }
    });
    let sprite = mod_api::AtlasRegion::new([0.0, 0.0, 1.0, 1.0], [16; 2]);
    assert_eq!(
        sky.draw_triangles(sprite, &vertices[..5]),
        Err(CelestialDrawError::IncompleteTriangle),
    );
    let mut invalid = vertices;
    invalid[5].color[0] = f32::NAN;
    assert_eq!(
        sky.draw_triangles(sprite, &invalid),
        Err(CelestialDrawError::InvalidVertex)
    );
    sky.draw_triangles(sprite, &vertices).unwrap();
}

pub(super) fn check(renderer: &mut Renderer, out: &Path) {
    renderer.resize(SIZE[0], SIZE[1]);
    renderer.fullbright = true;
    renderer.set_world_post(1.0, false, false, 1.0, 0.0);
    let hud = HudBatch::new(SIZE, GUI);
    let mut frame = fixture_frame(&hud);
    frame.depth_far = 64.0;
    frame.skybox = true;
    frame.sky.sky_color = [0.1, 0.2, 0.3];
    frame.sky.fog_color = frame.sky.sky_color;
    let shot = |renderer: &mut Renderer, frame: &Frame<'_>, mode, name: &str| {
        MODE.store(mode, Relaxed);
        let path = out.join(format!("celestial_{name}.png"));
        renderer.screenshot = Some(ScreenshotRequest {
            path: path.clone(),
            size: None,
            scaled: false,
            background: false,
        });
        assert!(renderer.render(frame).unwrap());
        renderer.wait_submissions();
        renderer
            .screenshot_result
            .take()
            .expect("pixel readback")
            .unwrap()
            .save()
            .unwrap();
        read(&path)
    };
    let pixel = |image: &[[u8; 4]], x, y| image[y * SIZE[0] as usize + x];
    let changed =
        |from: &[[u8; 4]], to: &[[u8; 4]]| from.iter().zip(to).filter(|(a, b)| a != b).count();

    let stock = shot(renderer, &frame, OFF, "stock");
    assert_eq!(CALLED.load(Relaxed), 0);
    let empty = shot(renderer, &frame, EMPTY, "empty");
    assert_eq!(CALLED.load(Relaxed), 1);
    assert_eq!(changed(&stock, &empty), 0);
    let nebula = shot(renderer, &frame, DRAW, "nebula");
    assert_eq!(CALLED.load(Relaxed), 2);
    let top = pixel(&nebula, 110, 53);
    let bottom = pixel(&nebula, 110, 127);
    let transparent = pixel(&nebula, 160, 90);
    let translucent = pixel(&nebula, 209, 53);
    let difference = changed(&stock, &nebula);
    assert!(
        difference > 7_000,
        "celestial picture changed only {difference} pixels"
    );
    assert!(
        top[1] > 200 && top[0] < 90,
        "green end of the gradient: {top:?}"
    );
    assert!(
        bottom[0] > 200 && bottom[1] < 90,
        "red end of the gradient: {bottom:?}"
    );
    assert_eq!(
        transparent,
        pixel(&stock, 160, 90),
        "transparent texels retain the sky"
    );
    assert!(
        translucent[1] > 80 && translucent[1] < 180,
        "texture alpha: {translucent:?}"
    );

    let initial_camera = frame.camera;
    frame.camera = DVec3::new(2_090_111.25, 203.0, -4_983_841.0);
    frame.eye = frame.camera;
    let translated = shot(renderer, &frame, DRAW, "translated");
    assert_eq!(changed(&nebula, &translated), 0);
    frame.camera = initial_camera;
    frame.eye = initial_camera;
    frame.yaw = 180.0;
    let away_stock = shot(renderer, &frame, OFF, "look_away_stock");
    let away = shot(renderer, &frame, DRAW, "look_away");
    assert_eq!(
        changed(&away_stock, &away),
        0,
        "camera rotation leaves the layer behind the view"
    );
    frame.yaw = 0.0;
    frame.sky.star_angle = 90.0;
    let turned = shot(renderer, &frame, DRAW, "star_angle");
    let turned_difference = changed(&nebula, &turned);
    assert!(
        turned_difference > 7_000,
        "star pose changed only {turned_difference} pixels"
    );
    frame.sky.star_angle = 0.0;

    let calls = CALLED.load(Relaxed);
    frame.sky_blocked = true;
    shot(renderer, &frame, DRAW, "blocked");
    frame.sky_blocked = false;
    frame.skybox = false;
    shot(renderer, &frame, DRAW, "no_overworld_sky");
    frame.end_sky = true;
    shot(renderer, &frame, DRAW, "end");
    frame.end_sky = false;
    frame.skybox = true;
    frame.title = true;
    shot(renderer, &frame, DRAW, "title");
    frame.title = false;
    assert_eq!(
        CALLED.load(Relaxed),
        calls,
        "these routes do not ask for a celestial layer"
    );

    let report = horizon_shader_api::check(&horizon_shader_api::Pack::from_files(
        [
            (horizon_shader_api::MANIFEST, "schema=1\nid='test_sky'\nname='Test sky'\nversion='1'\napi='0.2'\n[sky]\nfile='sky.wgsl'\n"),
            ("sky.wgsl", "fn sky(input: SkyInput) -> vec4<f32> { return vec4(0.0, 0.0, 0.5, 1.0); }"),
        ]
        .map(|(name, source)| (name.to_owned(), source.as_bytes().to_vec()))
        .into(),
    ).unwrap()).unwrap();
    renderer
        .set_native_shader_pack(Some((report, vec![], vec![], vec![], Default::default())))
        .unwrap();
    let pack = shot(renderer, &frame, OFF, "pack_stock");
    let pack_listened = shot(renderer, &frame, DRAW, "pack_listened");
    assert_eq!(CALLED.load(Relaxed), calls, "the pack owns this sky");
    assert_eq!(changed(&pack, &pack_listened), 0);
    renderer.set_native_shader_pack(None).unwrap();

    renderer.upload((0, 10, 0), &wall(u32::MAX, &[[0, 1, 2, 3], [3, 2, 1, 0]]));
    let terrain = shot(renderer, &frame, OFF, "terrain_stock");
    let occluded = shot(renderer, &frame, DRAW, "terrain_occlusion");
    assert_eq!(pixel(&occluded, 110, 53), pixel(&terrain, 110, 53));
    assert_ne!(
        pixel(&terrain, 110, 53),
        top,
        "opaque terrain replaces the nebula"
    );
    assert!(
        changed(&terrain, &occluded) < difference,
        "the uploaded terrain hides the layer"
    );
    assert!(
        changed(&terrain, &occluded) > 0,
        "the layer still shows beside terrain"
    );
    println!(
        "celestial stock={:?}, textured gradient={top:?}/{bottom:?}, transparent={transparent:?}, translucent={translucent:?}, terrain={:?}; changed pixels={difference}, translation=0, star angle={turned_difference}",
        pixel(&stock, 110, 53),
        pixel(&terrain, 110, 53)
    );
}
