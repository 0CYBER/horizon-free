//! World views (`horizon_client_render::render_slots::world_views`, the hook notes' "World views") through the real
//! `Renderer`, offscreen: a white wall the player's camera faces and a green one behind it that
//! a view from the other side faces, an entity quad between them, drawn without views and with
//! two. The frame's own picture and draws stay as they were; each view comes back at its size
//! with what its own camera sees, and the mod is asked once a frame. Behind the player's camera,
//! a blue wall facing it: mirrored views draw its front and cull its back as an unmirrored view
//! does, mirrored left to right; a clip plane leaves the green wall out of a view behind it, and
//! a clipped view's entities keep their depth order against terrain. Views seen in views draw
//! deepest first, so a child's picture is this frame's in its parent's `world_view_drawn`, and
//! eight views draw with the frame unchanged. Run with throwaway
//! HORIZON_HOME, HORIZON_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME, through production
//! source staging: `python3 tools/run_world_views.py <output-dir>`; `--bench` (an optimized
//! build): instead, frames of rolling land 49 by 49 chunks around the camera at 1920 by 1080
//! without a view, with one 640 by 360 view and with four: their CPU time, visibility spans,
//! section draws and GPU time.
use client::{
    gfx::{Frame, Renderer, ScreenshotRequest},
    graphics::GraphicsApi,
    hud::HudBatch,
    mesher::{SectionMesh, TerrainVertex, Vertex},
    render_slots::{
        self, Slots, WorldView, WorldViewDrawn, WorldViewRefusal, WorldViewRefusalReason,
        WorldViews,
    },
};
use glam::DVec3;
use horizon_client_render as client;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

/// The HUD's scale: the window's own, no forced Unicode font.
const GUI: client::hud::GuiScale = client::hud::GuiScale {
    requested: 0,
    unicode: false,
};

/// The views the mod below asks for each frame, and how many times it was asked.
static VIEWS: Mutex<Vec<WorldView>> = Mutex::new(Vec::new());
static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Each drawn view's id, size and a copy of its picture on its way back.
static DRAWN: Mutex<Vec<(u64, [u32; 2], wgpu::TextureFormat, wgpu::Buffer)>> =
    Mutex::new(Vec::new());
static DEVICE: Mutex<Option<wgpu::Device>> = Mutex::new(None);
/// What each push of the last frame returned.
type Pushed = Result<(), (u64, WorldViewRefusalReason)>;
static PUSHED: Mutex<Vec<Pushed>> = Mutex::new(Vec::new());
/// Each view drawn this frame: its id, parent and nesting depth, in the order drawn, and its
/// target.
static ORDER: Mutex<Vec<(u64, Option<u64>, u32)>> = Mutex::new(Vec::new());
static TARGETS: Mutex<Vec<(u64, wgpu::Texture)>> = Mutex::new(Vec::new());
/// In a parent's `world_view_drawn`: the parent, its child and a copy of the child's picture.
type Copied = (u64, u64, [u32; 2], wgpu::TextureFormat, wgpu::Buffer);
static IN_PARENT: Mutex<Vec<Copied>> = Mutex::new(Vec::new());
fn ask_views(views: &mut WorldViews) {
    ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ORDER.lock().unwrap().clear();
    TARGETS.lock().unwrap().clear();
    let named = |r: WorldViewRefusal| (r.id, r.reason);
    *PUSHED.lock().unwrap() = VIEWS
        .lock()
        .unwrap()
        .iter()
        .map(|v| views.push(*v).map_err(named))
        .collect();
}
/// A buffer `texture`'s picture is copied into, in `encoder`'s order.
fn copy_out(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> wgpu::Buffer {
    let row = (texture.width() * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    buffer
}
fn view_drawn(view: &mut WorldViewDrawn<'_>) {
    assert_eq!([view.color.width(), view.color.height()], view.size);
    // the views seen in this one, drawn before it this frame
    let children: Vec<(u64, Option<u64>)> = VIEWS
        .lock()
        .unwrap()
        .iter()
        .map(|v| (v.id, v.parent))
        .collect();
    for (child, texture) in TARGETS.lock().unwrap().iter() {
        if children.contains(&(*child, Some(view.id))) {
            let buffer = copy_out(view.device, view.encoder, texture);
            let size = [texture.width(), texture.height()];
            IN_PARENT
                .lock()
                .unwrap()
                .push((view.id, *child, size, view.color_format, buffer));
        }
    }
    let buffer = copy_out(view.device, view.encoder, view.color);
    *DEVICE.lock().unwrap() = Some(view.device.clone());
    ORDER
        .lock()
        .unwrap()
        .push((view.id, view.parent, view.nesting_depth));
    TARGETS.lock().unwrap().push((view.id, view.color.clone()));
    DRAWN
        .lock()
        .unwrap()
        .push((view.id, view.size, view.color_format, buffer));
}

fn install_mod() {
    render_slots::install(Slots {
        world_views: ask_views,
        world_views_hooked: || true,
        world_view_drawn: view_drawn,
        world_drawn: |_| {},
        hand_drawn: |_| {},
        render_needs: |_| {},
        world_drawn_hooked: || false,
    });
}

/// The views drawn in the last frame, by id: their size and RGBA pixels.
fn drawn_views() -> Vec<(u64, [u32; 2], Vec<[u8; 4]>)> {
    let drawn: Vec<_> = DRAWN.lock().unwrap().drain(..).collect();
    drawn
        .into_iter()
        .map(|(id, size, format, buffer)| (id, size, read_back(size, format, buffer)))
        .collect()
}

/// The RGBA pixels of a `copy_out` buffer once the GPU is done with it.
fn read_back(
    [width, height]: [u32; 2],
    format: wgpu::TextureFormat,
    buffer: wgpu::Buffer,
) -> Vec<[u8; 4]> {
    let device = DEVICE.lock().unwrap().clone();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device
        .as_ref()
        .unwrap()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = slice.get_mapped_range().unwrap();
    let row = bytes.len() / height as usize;
    let bgra = matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    assert!(
        format.block_copy_size(None) == Some(4),
        "an 8-bit frame: {format:?}"
    );
    let pixels: Vec<[u8; 4]> = (0..height as usize)
        .flat_map(|y| {
            bytes[y * row..y * row + width as usize * 4]
                .as_chunks::<4>()
                .0
        })
        .map(|p| {
            if bgra {
                [p[2], p[1], p[0], p[3]]
            } else {
                [p[0], p[1], p[2], p[3]]
            }
        })
        .collect();
    drop(bytes);
    buffer.unmap();
    pixels
}

fn read(path: &Path) -> Vec<[u8; 4]> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    bytes.truncate(info.buffer_size());
    bytes.as_chunks::<4>().0.to_vec()
}

/// Pixels like `want` (within 40 a channel), of an image.
fn count(pixels: &[[u8; 4]], want: [u8; 3]) -> usize {
    pixels
        .iter()
        .filter(|p| (0..3).all(|i| p[i].abs_diff(want[i]) < 40))
        .count()
}

/// A wall across x 0..16, y 0..16 at z = 12 of a section, in `color`, with each of `windings`:
/// `[0, 1, 2, 3]` faces north (-z), `[3, 2, 1, 0]` south.
fn wall(color: u32, windings: &[[usize; 4]]) -> SectionMesh {
    let mut mesh = SectionMesh::default();
    let corners = [
        [0., 0., 12.],
        [0., 16., 12.],
        [16., 16., 12.],
        [16., 0., 12.],
    ];
    let uvs = [[0., 1.], [0., 0.], [1., 0.], [1., 1.]];
    for order in windings {
        for &i in order {
            mesh.solid
                .push(TerrainVertex::new(corners[i], uvs[i], 0, color, 0xf0f0));
        }
    }
    mesh.sort_solid();
    mesh
}

/// `mesh` with a quad through `corners` (section-relative), both windings, in `color`.
fn with_quad(mut mesh: SectionMesh, corners: [[f32; 3]; 4], color: u32) -> SectionMesh {
    let uvs = [[0., 1.], [0., 0.], [1., 0.], [1., 1.]];
    for order in [[0, 1, 2, 3], [3, 2, 1, 0]] {
        for i in order {
            mesh.solid
                .push(TerrainVertex::new(corners[i], uvs[i], 0, color, 0xf0f0));
        }
    }
    mesh.sort_solid();
    mesh
}

fn fixture(window: Arc<Window>) -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).ok_or("output directory required")?;
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;
    install_mod();
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        true,
        &white,
        16,
        16,
        1,
        &[],
        &[],
        (&white, 16, 16),
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        None,
        GraphicsApi::Vulkan,
        false,
        true,
        &client::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))?;
    if std::env::args().any(|a| a == "--bench") {
        return bench(&mut renderer, out);
    }
    renderer.resize(320, 180);
    renderer.fullbright = true;
    // (keys count sections from the bottom of the world, y -64: 10 is y 96 to 112)
    // white at z 12, which the player's camera at z 0.5 faces; green at z 28 behind it
    let both = [[0, 1, 2, 3], [3, 2, 1, 0]];
    renderer.upload((0, 10, 0), &wall(u32::MAX, &both));
    // In front of the green wall, magenta terrain at x 9..11 a clipped view's plane at z 30 cuts:
    // a floor quad across it (z 28..32) and a wall quad just in front of it (z 29.995). Their
    // entity twins stand mirrored about x 8.
    let magenta = 0xffff_40ff;
    let green = wall(0xff00_ff00, &both);
    let green = with_quad(
        green,
        [[9., 6., 12.], [9., 6., 16.], [10., 6., 16.], [10., 6., 12.]],
        magenta,
    );
    let green = with_quad(
        green,
        [
            [9., 3., 13.995],
            [9., 5., 13.995],
            [11., 5., 13.995],
            [11., 3., 13.995],
        ],
        magenta,
    );
    renderer.upload((0, 10, 1), &green);
    // blue at z -4 behind the player's camera, facing south (toward it) only
    renderer.upload((0, 10, -1), &wall(0xffff_0000, &[[3, 2, 1, 0]]));
    // entity quads: red between the player's camera and the white wall, at z 6; yellow 0.1
    // behind the green wall (z 27.9) and cyan on its plane (z 28), seen from past it
    let quad = |corners: [[f32; 3]; 4], color: u32| {
        corners
            .into_iter()
            .zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]])
            .map(move |([x, y, z], uv)| Vertex {
                pos: [x - 8.0, y - 104.0, z - 0.5],
                uv,
                layer: 0,
                color,
                light: 0xf0,
                normal: [0, 0, 0, 0],
                glint_uv: [0.0; 2],
            })
    };
    let upright =
        |[x0, y0, x1, y1]: [f32; 4], z: f32| [[x0, y0, z], [x0, y1, z], [x1, y1, z], [x1, y0, z]];
    let entity: Vec<Vertex> = quad(upright([7.5, 103.5, 8.5, 104.5], 6.0), 0xff40_40ff)
        .chain(quad(upright([4.0, 100.0, 8.0, 104.0], 27.9), 0xff40_ffff))
        .chain(quad(upright([9.0, 105.0, 13.0, 109.0], 28.0), 0xffff_ff40))
        .chain(quad(
            [
                [6., 102., 28.],
                [6., 102., 32.],
                [7., 102., 32.],
                [7., 102., 28.],
            ],
            magenta,
        ))
        .chain(quad(upright([5.0, 99.0, 7.0, 101.0], 29.995), magenta))
        .collect();
    let hud = HudBatch::new([320, 180], GUI);
    // The game's frame: the entities culled with `camera_frustum`, which asks for the views,
    // then the frame drawn with them.
    let camera = DVec3::new(8.0, 104.0, 0.5);
    let entity_frusta = |renderer: &mut Renderer| {
        renderer.camera_frustum(camera, 85.0, 0.0, 0.0, glam::Mat4::IDENTITY, 1000.0)
    };
    let shot = |renderer: &mut Renderer, name: &str, views: Vec<WorldView>| {
        *VIEWS.lock().unwrap() = views;
        let mut frame = fixture_frame(&hud);
        frame.entities = &entity;
        let path = out.join(format!("{name}.png"));
        renderer.screenshot = Some(ScreenshotRequest {
            path: path.clone(),
            size: None,
            scaled: false,
            background: false,
        });
        let asked = ASKED.load(std::sync::atomic::Ordering::Relaxed);
        for _ in 0..2 {
            entity_frusta(renderer);
            renderer.render(&frame).unwrap();
        }
        let asked = ASKED.load(std::sync::atomic::Ordering::Relaxed) - asked;
        assert_eq!(asked, 2, "the mod is asked once a frame");
        let draws = [renderer.drawn_sections, renderer.section_draws];
        // the frames are submitted on the renderer's own thread: their copies first
        renderer.wait_submissions();
        renderer
            .screenshot_result
            .take()
            .expect("frame readback")
            .unwrap()
            .save()
            .unwrap();
        // the second frame's views
        let mut drawn = drawn_views();
        drawn.drain(..drawn.len() / 2);
        (read(&path), drawn, draws)
    };
    let (plain, none, plain_draws) = shot(&mut renderer, "plain", Vec::new());
    assert!(none.is_empty(), "no view asked for, none drawn");
    let wall_px = count(&plain, [255, 255, 255]);
    let entity_px = count(&plain, [255, 64, 64]);
    assert!(
        wall_px > 1000,
        "the player's camera sees the white wall: {wall_px}"
    );
    assert!(entity_px > 50, "and the entity: {entity_px}");
    assert_eq!(
        count(&plain, [0, 255, 0]),
        0,
        "not the green wall behind it"
    );

    // a camera past the green wall looking back (north), and one just behind the entity
    let behind = WorldView::new(
        7,
        DVec3::new(8.0, 104.0, 40.0),
        180.0,
        0.0,
        70.0,
        [200, 120],
    );
    let close = WorldView::new(9, DVec3::new(8.0, 104.0, 9.0), 180.0, 0.0, 70.0, [64, 64]);
    let (with_views, drawn, view_draws) = shot(&mut renderer, "views", vec![behind, close]);
    assert!(
        with_views == plain,
        "the frame's picture is the same with views drawn"
    );
    // the white and green walls ahead, each face toward the camera alone; not the blue one
    assert_eq!(plain_draws, [2, 2], "the frame's sections and draws");
    assert_eq!(
        view_draws, plain_draws,
        "views leave the frame's culling as it was"
    );
    let views_drew = renderer.world_view_draws();
    let ids: Vec<u64> = drawn.iter().map(|(id, ..)| *id).collect();
    assert_eq!(ids, [7, 9], "each view drawn once, in order");
    let (_, size, far) = &drawn[0];
    write_png(&out.join("view_7.png"), *size, far)?;
    write_png(&out.join("view_9.png"), drawn[1].1, &drawn[1].2)?;
    assert_eq!(*size, [200, 120]);
    assert_eq!(far.len(), 200 * 120);
    let green = count(far, [0, 255, 0]);
    assert!(
        green > far.len() / 4,
        "the view past the green wall sees it: {green}"
    );
    assert_eq!(
        count(far, [255, 255, 255]),
        0,
        "and not the white one behind it"
    );
    let (_, size, near) = &drawn[1];
    assert_eq!(*size, [64, 64]);
    // a block 3 blocks off at 70 degrees: about 17 by 17 of its 64 by 64 pixels
    let red = count(near, [255, 64, 64]);
    assert!(red > 200, "the view behind the entity sees it: {red}");

    // The game culls entities with the camera's frustum joined to the views': behind the
    // camera, one ahead of view 7 is kept for it, one 80 blocks east of it is in no frustum.
    let seen = |renderer: &mut Renderer, x: f32| {
        let (min, max) = (
            glam::Vec3::new(x - 1.0, -1.0, -12.0),
            glam::Vec3::new(x + 1.0, 1.0, -10.0),
        );
        entity_frusta(renderer).intersects(min, max)
    };
    assert!(seen(&mut renderer, 0.0), "an entity a view sees is kept");
    assert!(!seen(&mut renderer, 80.0), "one no frustum sees is culled");
    *VIEWS.lock().unwrap() = Vec::new();
    assert!(
        !seen(&mut renderer, 0.0),
        "and the first too, without views"
    );

    // Views seen in views, fresh ids no earlier frame drew, one frame: pushed parent first,
    // drawn deepest first at their depths, and in each parent's `world_view_drawn` its child's
    // picture is this frame's. A view whose parent was not pushed is refused by name.
    let parent = WorldView::new(
        50,
        DVec3::new(8.0, 104.0, 40.0),
        180.0,
        0.0,
        70.0,
        [200, 120],
    );
    let child = WorldView::new(51, DVec3::new(8.0, 104.0, 9.0), 180.0, 0.0, 70.0, [64, 64]);
    let grandchild = WorldView::new(52, DVec3::new(8.0, 104.0, 40.0), 180.0, 0.0, 70.0, [48, 32]);
    *VIEWS.lock().unwrap() = vec![
        parent,
        child.with_parent(50),
        grandchild.with_parent(51),
        WorldView::new(53, camera, 0.0, 0.0, 70.0, [8, 8]).with_parent(99),
    ];
    let mut frame = fixture_frame(&hud);
    frame.entities = &entity;
    entity_frusta(&mut renderer);
    renderer.render(&frame).unwrap();
    renderer.wait_submissions();
    assert_eq!(
        *PUSHED.lock().unwrap(),
        [
            Ok(()),
            Ok(()),
            Ok(()),
            Err((53, WorldViewRefusalReason::MissingParent(99)))
        ]
    );
    assert_eq!(
        *ORDER.lock().unwrap(),
        [(52, Some(51), 3), (51, Some(50), 2), (50, None, 1)]
    );
    let nested = drawn_views();
    let copies: Vec<_> = IN_PARENT.lock().unwrap().drain(..).collect();
    let copies: Vec<(u64, u64, Vec<[u8; 4]>)> = copies
        .into_iter()
        .map(|(parent, child, size, format, buffer)| {
            (parent, child, read_back(size, format, buffer))
        })
        .collect();
    let pairs: Vec<(u64, u64)> = copies.iter().map(|(p, c, _)| (*p, *c)).collect();
    assert_eq!(
        pairs,
        [(51, 52), (50, 51)],
        "each child in its parent's hook"
    );
    let own = |id| &nested.iter().find(|v| v.0 == id).unwrap().2;
    assert!(
        copies[0].2 == *own(52) && copies[1].2 == *own(51),
        "a child's picture in its parent's hook is the one it drew this frame"
    );
    let nested_green = count(&copies[0].2, [0, 255, 0]);
    let nested_red = count(&copies[1].2, [255, 64, 64]);
    assert!(
        nested_green > 48 * 32 / 4 && nested_red > 200,
        "and holds what it sees: {nested_green} green, {nested_red} red"
    );
    *VIEWS.lock().unwrap() = Vec::new();

    // Eight views draw, and the frame's picture and draws stay as they were.
    let eight: Vec<WorldView> = (60..68)
        .map(|id| WorldView::new(id, DVec3::new(8.0, 104.0, 40.0), 180.0, 0.0, 70.0, [64, 64]))
        .collect();
    let (with_eight, drawn, eight_draws) = shot(&mut renderer, "eight", eight);
    let ids: Vec<u64> = drawn.iter().map(|(id, ..)| *id).collect();
    assert_eq!(ids, [60, 61, 62, 63, 64, 65, 66, 67], "eight views drawn");
    assert!(
        with_eight == plain && eight_draws == plain_draws,
        "the frame's picture is the same with eight views drawn"
    );
    let eight_green: Vec<usize> = drawn.iter().map(|v| count(&v.2, [0, 255, 0])).collect();
    assert!(
        eight_green.iter().all(|&g| g > 64 * 64 / 4),
        "each sees the green wall: {eight_green:?}"
    );

    // Mirrored views (64 by 64, x 12): looking north at the blue wall's front 4.5 blocks off, it
    // fills the picture; looking south from z -10 at its back, it is culled, mirrored or not, and
    // the mirrored picture is the unmirrored one left to right (the white wall and the entity
    // are to the right of the middle). And with a clip plane at z 20, the view past the green
    // wall sees the white one instead.
    let at = |id, z, yaw| WorldView::new(id, DVec3::new(12.0, 104.0, z), yaw, 0.0, 70.0, [64, 64]);
    let clipped = behind.with_clip_plane([0.0, 0.0, -1.0, 20.0]);
    let mirrors = vec![
        at(11, 0.5, 180.0).with_mirror(),
        at(12, -10.0, 0.0).with_mirror(),
        at(13, -10.0, 0.0),
        clipped,
    ];
    let (_, drawn, _) = shot(&mut renderer, "mirrors", mirrors);
    let ids: Vec<u64> = drawn.iter().map(|(id, ..)| *id).collect();
    assert_eq!(ids, [11, 12, 13, 7]);
    for (id, size, pixels) in &drawn {
        write_png(&out.join(format!("mirror_view_{id}.png")), *size, pixels)?;
    }
    let blue = [0, 0, 255];
    let front_blue = count(&drawn[0].2, blue);
    assert_eq!(
        front_blue,
        64 * 64,
        "a mirrored view draws the faces toward it"
    );
    let back_blue = [count(&drawn[1].2, blue), count(&drawn[2].2, blue)];
    assert_eq!(back_blue, [0, 0], "and culls the faces away, as unmirrored");
    let (mirrored, plain_back) = (&drawn[1].2, &drawn[2].2);
    let unlike = (0..64 * 64)
        .filter(|i| {
            let (a, b) = (mirrored[*i], plain_back[i / 64 * 64 + 63 - i % 64]);
            (0..3).any(|c| a[c].abs_diff(b[c]) >= 40)
        })
        .count();
    let asymmetric = (0..64 * 64)
        .filter(|i| {
            let (a, b) = (plain_back[*i], plain_back[i / 64 * 64 + 63 - i % 64]);
            (0..3).any(|c| a[c].abs_diff(b[c]) >= 40)
        })
        .count();
    assert!(
        asymmetric > 200,
        "the unmirrored picture isn't symmetric: {asymmetric}"
    );
    assert!(
        unlike < 64,
        "the mirrored picture is it left to right: {unlike} unlike"
    );
    let clip = &drawn[3].2;
    let clip_white = count(clip, [255, 255, 255]);
    assert_eq!(
        count(clip, [0, 255, 0]),
        0,
        "the clip plane leaves the green wall out"
    );
    assert!(
        clip_white > 2000,
        "and the view sees the white wall: {clip_white}"
    );

    // Clipped views' entities: seen from z 76 through a plane 0.05 in front of the
    // camera, where depth changes little with distance, the yellow quad 0.1 behind the green wall
    // stays hidden; the cyan one on the wall's plane shows as an unclipped view shows it, clip and
    // mirror together too.
    let (yellow, cyan) = ([255, 255, 64], [64, 255, 255]);
    let north = |id, z, size| WorldView::new(id, DVec3::new(8.0, 104.0, z), 180.0, 0.0, 70.0, size);
    let entity_views = vec![
        north(21, 76.0, [256, 256]).with_clip_plane([0.0, 0.0, -1.0, 75.95]),
        north(22, 40.0, [64, 64]).with_clip_plane([0.0, 0.0, -1.0, 36.0]),
        north(23, 40.0, [64, 64]),
        north(24, 40.0, [64, 64])
            .with_clip_plane([0.0, 0.0, -1.0, 36.0])
            .with_mirror(),
    ];
    let (_, drawn, _) = shot(&mut renderer, "clipped_entities", entity_views);
    for (id, size, pixels) in &drawn {
        write_png(&out.join(format!("entity_view_{id}.png")), *size, pixels)?;
    }
    let behind_wall = count(&drawn[0].2, yellow);
    assert!(
        count(&drawn[0].2, [0, 255, 0]) > 2000,
        "the far view sees the green wall"
    );
    assert_eq!(
        behind_wall, 0,
        "an entity behind a wall stays hidden in a clipped view"
    );
    let coplanar = drawn[1..]
        .iter()
        .map(|v| count(&v.2, cyan))
        .collect::<Vec<_>>();
    assert!(
        coplanar[1] > 150,
        "the unclipped view sees the cyan quad: {coplanar:?}"
    );
    assert!(
        coplanar.iter().all(|&c| c.abs_diff(coplanar[1]) < 8),
        "and the clipped ones, mirrored too, as much of it: {coplanar:?}"
    );
    let (clipped, both) = (&drawn[1].2, &drawn[3].2);
    let clip_mirror_unlike = (0..64 * 64)
        .filter(|i| {
            let (a, b) = (both[*i], clipped[i / 64 * 64 + 63 - i % 64]);
            (0..3).any(|c| a[c].abs_diff(b[c]) >= 40)
        })
        .count();
    assert!(
        clip_mirror_unlike < 64,
        "clipped and mirrored is the clipped picture left to right: {clip_mirror_unlike}"
    );

    // A clipped view 10 behind its plane (z 30) cuts the magenta entities where it cuts their
    // terrain twins: the floor across the plane, and the wall 0.005 in front of it kept whole.
    // The unclipped view sees more of the floor.
    let edge = vec![
        north(26, 40.0, [128, 128]).with_clip_plane([0.0, 0.0, -1.0, 30.0]),
        north(27, 40.0, [128, 128]),
    ];
    let (_, drawn, _) = shot(&mut renderer, "plane_edge", edge);
    for (id, size, pixels) in &drawn {
        write_png(&out.join(format!("edge_view_{id}.png")), *size, pixels)?;
    }
    let halves = |pixels: &[[u8; 4]]| {
        let left = (0..128 * 128)
            .filter(|i| i % 128 < 64 && count(&pixels[*i..*i + 1], [255, 64, 255]) == 1)
            .count();
        (left, count(pixels, [255, 64, 255]) - left)
    };
    let (edge_entity, edge_terrain) = halves(&drawn[0].2);
    let (whole_entity, _) = halves(&drawn[1].2);
    assert!(
        edge_terrain > 200 && edge_entity.abs_diff(edge_terrain) <= 4 + edge_terrain / 50,
        "the plane cuts entities as it cuts terrain: {edge_entity} vs {edge_terrain}"
    );
    assert!(whole_entity > edge_entity, "{whole_entity} unclipped");

    // a view asked for again at another size and roll comes back at that size
    let turned = behind.with_roll(30.0);
    let mut resized = turned;
    resized.size = [96, 160];
    let (_, drawn, _) = shot(&mut renderer, "resized", vec![resized]);
    assert_eq!(drawn.len(), 1);
    assert_eq!((drawn[0].1, drawn[0].2.len()), ([96, 160], 96 * 160));

    let (after, drawn, _) = shot(&mut renderer, "after", Vec::new());
    assert!(
        drawn.is_empty() && after == plain,
        "with the views gone the frame is as before"
    );
    std::fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "wall_pixels": wall_px, "entity_pixels": entity_px, "view_green_pixels": green,
            "view_entity_pixels": red, "frame_unchanged_with_views": true,
            "nested_child_pixels": [nested_green, nested_red], "eight_views_green": eight_green,
            "frame_sections_and_draws": plain_draws, "view_draws": views_drew,
            "mirrored_front_pixels": front_blue, "mirrored_back_pixels": back_blue[0],
            "mirrored_unlike_pixels": unlike, "clipped_white_pixels": clip_white,
            "clipped_behind_wall_pixels": behind_wall, "coplanar_pixels": coplanar,
            "clipped_mirrored_unlike_pixels": clip_mirror_unlike,
            "plane_edge_pixels": [edge_entity, edge_terrain, whole_entity],
            "graphics_adapter": client::crash::system_details("fixture").into_iter()
                .find(|(name, _)| name == "Graphics Adapter").map(|(_, detail)| detail),
        }))?,
    )?;
    println!(
        "world view check passed: {}",
        out.join("result.json").display()
    );
    Ok(())
}

fn write_png(path: &Path, [width, height]: [u32; 2], pixels: &[[u8; 4]]) -> std::io::Result<()> {
    let mut encoder = png::Encoder::new(std::fs::File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels.as_flattened())?;
    Ok(())
}
/// Rolling land: a column of a section's top faces and the sides they show, at y 64 to 80.
fn hills(cx: i32, cz: i32) -> SectionMesh {
    let height = |x: i32, z: i32| {
        let (x, z) = (f64::from(cx * 16 + x), f64::from(cz * 16 + z));
        (8.0 + 4.0 * (x * 0.09).sin() + 3.0 * (z * 0.07 + x * 0.02).cos()).floor() as i32
    };
    let mut mesh = SectionMesh::default();
    let mut quad = |c: [[f32; 3]; 4]| {
        for (p, uv) in c.into_iter().zip([[0., 1.], [0., 0.], [1., 0.], [1., 1.]]) {
            mesh.solid
                .push(TerrainVertex::new(p, uv, 0, 0xff60_a060, 0xf0));
        }
    };
    for x in 0..16 {
        for z in 0..16 {
            let h = height(x, z);
            let (fx, fz, fh) = (x as f32, z as f32, h as f32);
            quad([
                [fx, fh, fz],
                [fx, fh, fz + 1.],
                [fx + 1., fh, fz + 1.],
                [fx + 1., fh, fz],
            ]);
            // the step down to a lower neighbour, toward +x and +z
            for (dx, dz) in [(1, 0), (0, 1)] {
                let n = height(x + dx, z + dz);
                if n < h {
                    let (lo, hi) = (n as f32, fh);
                    if dx == 1 {
                        quad([
                            [fx + 1., lo, fz],
                            [fx + 1., hi, fz],
                            [fx + 1., hi, fz + 1.],
                            [fx + 1., lo, fz + 1.],
                        ]);
                    } else {
                        quad([
                            [fx, lo, fz + 1.],
                            [fx + 1., lo, fz + 1.],
                            [fx + 1., hi, fz + 1.],
                            [fx, hi, fz + 1.],
                        ]);
                    }
                }
            }
        }
    }
    mesh.sort_solid();
    mesh
}

/// `--bench`: a frame without a view, with one and with four, each 640 by 360: its CPU time (the
/// render call, milliseconds), its visibility spans (the frame's walk and the views'), its
/// section draws and the views', and its GPU time.
fn bench(renderer: &mut Renderer, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    const R: i32 = 24;
    renderer.resize(1920, 1080);
    let mut sections = 0;
    for cx in -R..=R {
        for cz in -R..=R {
            // (keys count sections from y -64: 8 is y 64 to 80)
            renderer.upload((cx, 8, cz), &hills(cx, cz));
            sections += 1;
        }
    }
    client::spans::enable(true);
    let hud = HudBatch::new([1920, 1080], GUI);
    let mut time = |views: Vec<WorldView>| {
        *VIEWS.lock().unwrap() = views;
        let mut frame = fixture_frame(&hud);
        frame.camera = DVec3::new(8.0, 90.0, 8.0);
        frame.eye = frame.camera;
        frame.pitch = 15.0;
        frame.eye_rotation = [0.0, 15.0];
        frame.distance = 16.0 * R as f32;
        frame.detail_distance = frame.distance;
        frame.depth_far = 2000.0;
        frame.cave_culling = true;
        for _ in 0..30 {
            renderer.render(&frame).unwrap();
            DRAWN.lock().unwrap().clear();
        }
        client::spans::take();
        renderer.gpu_times.clear();
        const FRAMES: u32 = 240;
        let (mut cpu, mut walk, mut views_walk) = (0.0, 0.0, 0.0);
        for _ in 0..FRAMES {
            let started = std::time::Instant::now();
            renderer.render(&frame).unwrap();
            cpu += started.elapsed().as_secs_f64() * 1000.0;
            for span in client::spans::take() {
                match span.name {
                    "visibility" => walk += f64::from(span.ms),
                    "world view visibility" => views_walk += f64::from(span.ms),
                    _ => {}
                }
            }
            DRAWN.lock().unwrap().clear();
        }
        renderer.wait_submissions();
        let frames = f64::from(FRAMES);
        let gpu = &renderer.gpu_times;
        let gpu_ms = (!gpu.is_empty())
            .then(|| gpu.iter().map(|&t| f64::from(t)).sum::<f64>() / gpu.len() as f64);
        serde_json::json!({
            "frame_cpu_ms": cpu / frames, "visibility_ms": walk / frames,
            "view_visibility_ms": views_walk / frames, "frame_sections": renderer.drawn_sections,
            "frame_section_draws": renderer.section_draws,
            "frame_draw_calls": renderer.section_draw_calls,
            "view_section_draws": renderer.world_view_draws(), "gpu_ms": gpu_ms,
        })
    };
    // cameras 60 blocks away, looking back across the land from four sides
    let view = |id, x: f64, z: f64, yaw| {
        WorldView::new(
            id,
            DVec3::new(8.0 + x, 95.0, 8.0 + z),
            yaw,
            10.0,
            70.0,
            [640, 360],
        )
    };
    let four = vec![
        view(1, 60.0, 30.0, 120.0),
        view(2, -60.0, 30.0, -120.0),
        view(3, 30.0, -60.0, 30.0),
        view(4, -30.0, -60.0, -30.0),
    ];
    let none = time(Vec::new());
    let one = time(four[..1].to_vec());
    let all = time(four);
    let again = time(Vec::new());
    let result = serde_json::json!({
        "sections": sections, "no_view": none, "one_view": one, "four_views": all,
        "no_view_after": again,
        "graphics_adapter": client::crash::system_details("bench").into_iter()
            .find(|(name, _)| name == "Graphics Adapter").map(|(_, detail)| detail),
    });
    println!("{result}");
    std::fs::write(out.join("bench.json"), serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

struct App;
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        if let Err(error) = fixture(window) {
            eprintln!("world view check failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
fn main() {
    for key in [
        "HORIZON_HOME",
        "HORIZON_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        assert!(
            std::env::var_os(key).is_some(),
            "set throwaway {key} (AGENTS.md)"
        );
    }
    EventLoop::new().unwrap().run_app(&mut App).unwrap();
}

fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(8.0, 104.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_degrees: 85.0,
        eye: glam::DVec3::new(8.0, 104.0, 0.5),
        eye_rotation: [0.0, 0.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 128.0,
        detail_distance: 128.0,
        depth_far: 1000.0,
        underwater: false,
        water_plane: None,
        cave_culling: false,
        title: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        boss_world_fog: false,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        pvp: Default::default(),
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: Default::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        colored_lights: &[],
        emissive_layers: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        shadow_only: &[],
        celestial: &[],
        stars: None,
        hand: &[],
        hand_items: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        chunk_fade: 0.0,
        post_effects: client::post_chain::Request::none(),
        menu_blur: None,
    }
}
