//! Frame generation on the float frame with HDR output (#12989): the made frame keeps the
//! light above paper white, lands halfway, and HDR's last pass shows it as a real frame.

use super::optical_flow::tests as of;
use super::{Camera, FrameGen};
use crate::gfx::{hdr, hud_coverage, world_post::Post};
use glam::Mat4;

const SIZE: [u32; 2] = [512, 288];
const FLOAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The display's peak over paper white: 1000 nits over 203.
const PEAK: f32 = 1000.0 / 203.0;

/// 8 times paper white, as the float frame stores light (vanilla's gamma).
fn bright() -> f32 {
    8f32.powf(1.0 / 2.2)
}

/// The float frame's light: cells of one primary each (`of::pattern`), and every fifth cell
/// across and down at 8 times paper white, all moved by `shift` pixels.
fn light(shift: [i32; 2]) -> Vec<[f32; 4]> {
    let primaries = of::pattern(SIZE, 100, shift);
    (0..(SIZE[0] * SIZE[1]) as usize)
        .map(|i| {
            let (x, y) = ((i as u32 % SIZE[0]) as i32, (i as u32 / SIZE[0]) as i32);
            let cell = [(x - shift[0]).div_euclid(4), (y - shift[1]).div_euclid(4)];
            if cell.iter().all(|c| c.rem_euclid(5) == 0) {
                [bright(), bright(), bright(), 1.0]
            } else {
                let [r, g, b] = [0, 1, 2].map(|c| f32::from(primaries[i * 4 + c]) / 255.0);
                [r, g, b, 1.0]
            }
        })
        .collect()
}

fn texture(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, pixels: &[[f32; 4]]) -> wgpu::TextureView {
    let t = texture(device, FLOAT);
    let halves: Vec<u16> = pixels
        .iter()
        .flat_map(|p| p.map(|v| half::f16::from_f32(v).to_bits()))
        .collect();
    queue.write_texture(
        t.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE[0] * 8),
            rows_per_image: None,
        },
        t.size(),
    );
    t.create_view(&Default::default())
}

fn read_float(device: &wgpu::Device, queue: &wgpu::Queue, t: &wgpu::Texture) -> Vec<[f32; 4]> {
    of::read(device, queue, t, 8)
        .chunks(8)
        .map(|c| {
            std::array::from_fn(|i| half::f16::from_le_bytes([c[2 * i], c[2 * i + 1]]).to_f32())
        })
        .collect()
}

/// Frame generation as the renderer runs it on the float frame, the camera still.
struct Run {
    device: wgpu::Device,
    queue: wgpu::Queue,
    post: Post,
    fg: FrameGen,
    camera: Camera,
    config: wgpu::SurfaceConfiguration,
}

impl Run {
    fn new() -> Option<Self> {
        let (device, queue) = of::device()?;
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE[0],
                height: SIZE[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = depth.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.01),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        queue.submit([encoder.finish()]);
        let mut fg = FrameGen::new();
        let atlas = of::upload(&device, &queue, [1, 1], &[0; 4]);
        fg.track_entities(&device, &atlas, &[], &[], glam::DVec3::ZERO);
        Some(Self {
            post: Post::new(&device, FLOAT),
            fg,
            camera: Camera {
                jittered: Mat4::IDENTITY,
                view_proj: Mat4::IDENTITY,
                depth,
                render_size: SIZE,
                jitter: [0.0; 2],
                reproject: None,
                near: 0.05,
                far: 512.0,
                fov_y: 1.2,
            },
            config: wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
                format: wgpu::TextureFormat::Rgb10a2Unorm,
                width: SIZE[0],
                height: SIZE[1],
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                color_space: wgpu::SurfaceColorSpace::default(),
            },
            device,
            queue,
        })
    }

    /// One frame: `pixels` drawn onto the world at the screen's size, and the made frame's
    /// world onto `made`. Whether there is a made frame.
    fn frame(&mut self, pixels: &[[f32; 4]], made: &wgpu::TextureView) -> bool {
        let (device, queue) = (&self.device, &self.queue);
        let world = self.fg.world(device, &self.post, &self.config, FLOAT);
        let mut encoder = device.create_command_encoder(&Default::default());
        let source = upload(device, queue, pixels);
        self.post
            .finish_with(&mut encoder, &world, &self.post.bind(device, &source), None);
        let made = self.fg.generate(
            device,
            queue,
            &mut encoder,
            &self.post,
            made,
            &self.camera,
            Some(PEAK),
        );
        queue.submit([encoder.finish()]);
        self.camera.reproject = Some(Mat4::IDENTITY);
        made
    }
}

/// A highlight at 8 times paper white among primaries, moving 4 pixels: the made frame has
/// it halfway, as bright, and holds no negative or non-finite value. Light past 1 would
/// spill out of the optical flow's 8-bit luma without the float frame's.
#[test]
fn a_made_frame_of_the_float_frame_keeps_its_light_and_lands_halfway() {
    let Some(mut run) = Run::new() else {
        return;
    };
    let made = texture(&run.device, FLOAT);
    let made_view = made.create_view(&Default::default());
    // FSR uses the game's vectors only for 10 frames after a reset; the camera is still.
    let still = light([0, 0]);
    for _ in 0..20 {
        run.frame(&still, &made_view);
    }
    let moved = light([4, 0]);
    assert!(run.frame(&moved, &made_view));
    let got = read_float(&run.device, &run.queue, &made);
    assert!(
        got.iter().flatten().all(|v| v.is_finite() && *v >= 0.0),
        "a negative or non-finite value"
    );
    let truth = light([2, 0]);
    let inside = |i: usize| {
        let (x, y) = (i % SIZE[0] as usize, i / SIZE[0] as usize);
        (32..SIZE[0] as usize - 32).contains(&x) && (32..SIZE[1] as usize - 32).contains(&y)
    };
    let error = |frame: &[[f32; 4]]| {
        let (mut sum, mut count) = (0.0, 0.0);
        for (i, (p, t)) in frame.iter().zip(&truth).enumerate() {
            if inside(i) {
                sum += (0..3).map(|c| (p[c] - t[c]).abs()).sum::<f32>();
                count += 3.0;
            }
        }
        sum / count
    };
    let (made_error, before, after) = (error(&got), error(&still), error(&moved));
    assert!(
        made_error * 3.0 < before.min(after),
        "{made_error} against {before} and {after}"
    );
    // The highlight's pixels where it is halfway keep its light: none clipped at paper
    // white (1) or past 8 times it. FSR mixes in a little of the still camera's estimate
    // where the two frames' colours differ (white against a primary), so they sit a few
    // percent under it.
    let mut highlight: Vec<f32> = (0..truth.len())
        .filter(|&i| inside(i) && truth[i][0] == bright())
        .map(|i| got[i][0])
        .collect();
    highlight.sort_by(f32::total_cmp);
    let n = highlight.len();
    let (tenth, median, top) = (highlight[n / 10], highlight[n / 2], highlight[n - 1]);
    assert!(
        tenth > 2.0 && median > bright() * 0.95 && top <= bright() * 1.001,
        "highlight {tenth} {median} {top} against {}",
        bright()
    );
}

/// HDR output's last pass on a made frame gives the same nits as on a real frame with the
/// same content: the made frame takes the frame and goes through the same passes.
#[test]
fn hdr_output_shows_a_made_frame_as_the_real_one() {
    let Some(mut run) = Run::new() else {
        return;
    };
    let mut hdr = hdr::Hdr::new(&run.device, &run.queue, FLOAT, hdr::Output::Pq, true);
    hdr.paper_white = 203.0;
    hdr.peak = 1000.0;
    let pixels = light([0, 0]);
    // Through HDR's passes from the tone curve on: what the frame holds, onto `target`.
    let show = |run: &Run, hdr: &mut hdr::Hdr, made: bool| {
        let frame = hdr.frame(&run.device, SIZE);
        let view = frame.create_view(&Default::default());
        let mut encoder = run.device.create_command_encoder(&Default::default());
        if !made {
            let source = upload(&run.device, &run.queue, &pixels);
            let group = run.post.bind(&run.device, &source);
            run.post.finish_with(&mut encoder, &view, &group, None);
        }
        hdr.begin(&run.queue, true);
        hdr.tonemap(&mut encoder, None);
        hdr.hud_backdrop(&mut encoder, None);
        hud_coverage::clear(&mut encoder, &hdr.coverage_view().unwrap());
        let target = texture(&run.device, hdr::Output::Pq.format());
        hdr.present(&run.queue, &mut encoder, &target, None);
        run.queue.submit([encoder.finish()]);
        of::read(&run.device, &run.queue, &target, 4)
    };
    let real = show(&run, &mut hdr, false);
    let frame = hdr
        .frame(&run.device, SIZE)
        .create_view(&Default::default());
    for _ in 0..3 {
        run.frame(&pixels, &frame);
    }
    let made = show(&run, &mut hdr, true);
    let codes = |bytes: &[u8]| -> Vec<u32> {
        bytes
            .chunks(4)
            .flat_map(|c| {
                let pixel = u32::from_le_bytes(c.try_into().unwrap());
                [0, 10, 20].map(|shift| (pixel >> shift) & 0x3ff)
            })
            .collect()
    };
    let (real, made) = (codes(&real), codes(&made));
    // The highlight shows above paper white (203 nits is PQ code 592 of 1023).
    assert!(real.iter().any(|&c| c > 700), "no light above paper white");
    // The interpolation samples the two frames bilinearly in float, a hair off a pixel's
    // centre at most: a channel may round to the next half float, one PQ code at most.
    let off = real.iter().zip(&made).filter(|(a, b)| a != b).count();
    let most = real.iter().zip(&made).map(|(a, b)| a.abs_diff(*b)).max();
    assert!(
        most <= Some(1) && off * 100 <= real.len(),
        "{off} of {} channels differ, by {most:?} PQ codes at most",
        real.len()
    );
}
