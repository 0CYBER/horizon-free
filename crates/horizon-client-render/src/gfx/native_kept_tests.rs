use super::*;
use horizon_shader_api::{MANIFEST, Pack, Report, check};
use wgpu::{BufferUsages as B, ShaderStages as S, TextureUsages as T};

const HEAD: &str = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.2\"\n";

/// The checked pack of `manifest` (after `HEAD`) and `files`.
fn report(manifest: &str, files: &[(&str, &str)]) -> Report {
    let mut all: std::collections::BTreeMap<String, Vec<u8>> = (files.iter())
        .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
        .collect();
    all.insert(MANIFEST.into(), format!("{HEAD}{manifest}").into_bytes());
    check(&Pack::from_files(all).unwrap()).unwrap()
}

/// A buffer of each element, then each format at a fixed size and at half of R: more than a
/// pack may declare, so not read from a manifest.
fn every_shape() -> Kept {
    use horizon_shader_api::kept::{Element, Keep, Resource};
    let buffers = Element::ALL.map(|element| Shape::Buffer { bytes: 64, element });
    let textures = Format::ALL.into_iter().flat_map(|format| {
        [
            Shape::Fixed {
                format,
                size: [3, 2],
            },
            Shape::Scaled { format, scale: 0.5 },
        ]
    });
    let shapes = buffers.into_iter().chain(textures).enumerate();
    let resources = shapes.map(|(i, shape)| Resource {
        name: format!("r{i}"),
        shape,
        keep: Keep::View,
    });
    Kept {
        resources: resources.collect(),
        ..Default::default()
    }
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    eprintln!("kept resources on {:?}", adapter.get_info().backend);
    pollster::block_on(adapter.request_device(&Default::default())).ok()
}

fn sizes(store: &KeptStore) -> Vec<(u64, Option<[u32; 2]>)> {
    let size = |t: &wgpu::Texture| [t.width(), t.height()];
    (store.held.iter())
        .map(|h| match h {
            Held::Buffer(b) => {
                assert_eq!(b.usage(), B::STORAGE | B::COPY_DST);
                (b.size(), None)
            }
            Held::Pair([a, b]) => {
                let (a, b) = (a.texture(), b.texture());
                let usage = T::STORAGE_BINDING | T::TEXTURE_BINDING | T::RENDER_ATTACHMENT;
                assert_eq!((a.usage(), b.usage()), (usage, usage));
                assert_eq!((size(a), a.format()), (size(b), b.format()));
                (0, Some(size(a)))
            }
        })
        .collect()
}

#[test]
fn each_shape_is_made_with_its_size_and_usages_and_fit_remakes_the_scaled_ones() {
    let Some((device, _)) = device() else { return };
    let kept = every_shape();
    let mut store = KeptStore::new(&device, &kept, [101, 60]);
    let buffers = [(64, None); 5];
    let textures = |half| [(0, Some([3, 2])), (0, Some(half))].repeat(Format::ALL.len());
    assert_eq!(
        sizes(&store),
        [&buffers[..], &textures([51, 30])[..]].concat()
    );
    let Held::Pair([r32, _]) = &store.held[7] else {
        panic!("a texture")
    };
    assert_eq!(r32.texture().format(), wgpu::TextureFormat::R32Float);
    store.fit(&device, [40, 9]);
    assert_eq!(
        sizes(&store),
        [&buffers[..], &textures([20, 5])[..]].concat()
    );
}

/// Every format and element, made and bound as each pass would, under an error scope: run with
/// `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn every_shape_allocates_and_binds_without_a_validation_error() {
    let Some((device, _)) = device() else { return };
    let kept = every_shape();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let store = KeptStore::new(&device, &kept, [64, 32]);
    for (resource, held) in store.held.iter().enumerate() {
        let access = [KeptAccess::Read, KeptAccess::Write];
        for (access, visibility) in access
            .into_iter()
            .zip([S::FRAGMENT | S::COMPUTE, S::COMPUTE])
        {
            let binding = KeptBinding {
                binding: 0,
                resource,
                access,
            };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &layout_entries(&kept, &[binding], visibility),
            });
            let resource = match held {
                Held::Buffer(b) => b.as_entire_binding(),
                Held::Pair([view, _]) => wgpu::BindingResource::TextureView(view),
            };
            let _group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource,
                }],
            });
        }
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

const BUFFERS: &str = "[[resource]]\nname = \"cells\"\nbuffer = 1024\nelement = \"u32\"\n\
                       keep = \"view\"\n[[resource]]\nname = \"wide\"\nformat = \"r32float\"\n\
                       size = [4096, 1]\nkeep = \"view\"\n";
const SUM: &str = "var<workgroup> sums: array<f32, 4096>;\n\
                   fn compute(input: ComputeInput) {\n    \
                   sums[input.local] = 1.0;\n    workgroupBarrier();\n    \
                   if input.in_bounds { cells[input.local] = u32(sums[0]); }\n}\n";
const LAST: &str =
    "fn post(input: PostInput) -> vec4<f32> { return input.color * f32(cells[0]); }\n";

/// A pack whose compute pass has `workgroup` and `dispatch`, and 16 KiB of `var<workgroup>`.
fn computing(workgroup: &str, dispatch: &str) -> Report {
    let manifest = format!(
        "{BUFFERS}[[post]]\nfile = \"sum.wgsl\"\ncompute = true\ndispatch = {dispatch}\n\
         workgroup = {workgroup}\nwrites = [\"cells\"]\n[[post]]\nfile = \"last.wgsl\"\nreads = [\"cells\"]\n"
    );
    report(&manifest, &[("sum.wgsl", SUM), ("last.wgsl", LAST)])
}

fn refused(report: &Report, limits: wgpu::Limits) -> Option<String> {
    refusal(&limits, &report.manifest, &report.stages, &[])
}

#[test]
fn a_device_short_of_what_the_passes_need_refuses_the_pack_with_the_reason() {
    let pack = computing("[64, 1, 1]", "[65535, 1, 1]");
    let fits = wgpu::Limits::default();
    assert_eq!(refused(&pack, fits.clone()), None);
    let cases = [
        (
            wgpu::Limits::downlevel_webgl2_defaults(),
            "the device runs no compute shaders",
        ),
        (
            wgpu::Limits {
                max_compute_workgroup_storage_size: 8192,
                ..fits.clone()
            },
            "sum.wgsl: 16384 bytes of var<workgroup>, the device takes 8192",
        ),
        (
            wgpu::Limits {
                max_compute_invocations_per_workgroup: 32,
                ..fits.clone()
            },
            "sum.wgsl: a workgroup of [64, 1, 1], the device takes [256, 256, 64] and 32 invocations",
        ),
        (
            wgpu::Limits {
                max_compute_workgroups_per_dimension: 1000,
                ..fits.clone()
            },
            "sum.wgsl: 1024 workgroups along an axis, the device takes 1000",
        ),
        (
            wgpu::Limits {
                max_storage_buffer_binding_size: 512,
                ..fits.clone()
            },
            "cells: 1024 bytes, the device takes 512",
        ),
        (
            wgpu::Limits {
                max_texture_dimension_2d: 2048,
                ..fits.clone()
            },
            "wide: 4096×1 texels, the device takes 2048",
        ),
        (
            wgpu::Limits {
                max_storage_buffers_per_shader_stage: 0,
                ..fits
            },
            "sum.wgsl: 1 storage buffers, the device takes 0",
        ),
    ];
    for (limits, why) in cases {
        assert_eq!(refused(&pack, limits).as_deref(), Some(why));
    }
}

/// Two passes share a file of two 16 KiB arrays and each uses one: each needs 16 KiB, which a
/// 16 KiB device (the default) has.
#[test]
fn a_pass_needs_only_the_workgroup_memory_it_uses() {
    let manifest = "[[post]]\nfile = \"a.wgsl\"\nfiles = [\"w.wgsl\"]\ncompute = true\n\
                dispatch = [64, 1, 1]\nworkgroup = [64, 1, 1]\n\
                [[post]]\nfile = \"b.wgsl\"\nfiles = [\"w.wgsl\"]\ncompute = true\n\
                dispatch = [64, 1, 1]\nworkgroup = [64, 1, 1]\n\
                [[post]]\nfile = \"last.wgsl\"\n";
    let w = "var<workgroup> a: array<f32, 4096>;\nvar<workgroup> b: array<f32, 4096>;\n";
    let uses = |v| format!("fn compute(input: ComputeInput) {{ {v}[input.local] = 1.0; }}\n");
    let last = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    let (a, b) = (uses("a"), uses("b"));
    let files = [
        ("w.wgsl", w),
        ("a.wgsl", &a),
        ("b.wgsl", &b),
        ("last.wgsl", last),
    ];
    let pack = report(manifest, &files);
    assert_eq!(refused(&pack, wgpu::Limits::default()), None);
}

/// Each pass counts its own kept bindings, in its own stage: four storage buffers of a draw
/// pass fit a device that takes four, a fifth doesn't.
#[test]
fn a_pass_s_kept_bindings_count_toward_its_stage_s_limits() {
    let names = ["a", "b", "c", "d", "e"];
    let resources: String = (names.iter())
        .map(|n| {
            format!(
                "[[resource]]\nname = \"{n}\"\nbuffer = 64\nelement = \"u32\"\nkeep = \"view\"\n"
            )
        })
        .collect();
    let pack = |n: usize| {
        let list = |f: fn(&&str) -> String| names[..n].iter().map(f).collect::<Vec<_>>();
        let reads = list(|r| format!("\"{r}\"")).join(", ");
        let sum = list(|r| format!("{r}[0]")).join(" + ");
        let post = format!(
            "fn post(input: PostInput) -> vec4<f32> {{ return input.color * f32({sum}); }}\n"
        );
        let manifest = format!("{resources}[[post]]\nfile = \"p.wgsl\"\nreads = [{reads}]\n");
        report(&manifest, &[("p.wgsl", &post)])
    };
    let four = wgpu::Limits {
        max_storage_buffers_per_shader_stage: 4,
        ..Default::default()
    };
    assert_eq!(refused(&pack(4), four.clone()), None);
    let why = refused(&pack(5), four).unwrap();
    assert_eq!(why, "p.wgsl: 5 storage buffers, the device takes 4");
}

/// A read 32-bit texture isn't filterable (an `rgba32uint` one is `u32`), a written one is
/// write-only storage, and a buffer is read-only where the pass doesn't write it.
#[test]
fn layout_entries_follow_the_access_and_the_format() {
    let kept = every_shape();
    let at = |resource, access| KeptBinding {
        binding: 64,
        resource,
        access,
    };
    let ty = |b| layout_entries(&kept, &[b], S::COMPUTE)[0].ty;
    let texture = |filterable| wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    assert_eq!(ty(at(5, KeptAccess::Previous)), texture(true));
    assert_eq!(ty(at(7, KeptAccess::Read)), texture(false));
    assert_eq!(ty(at(9, KeptAccess::Previous)), texture(false));
    let uint = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Uint,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    assert_eq!(ty(at(11, KeptAccess::Read)), uint);
    assert_eq!(ty(at(12, KeptAccess::Previous)), uint);
    assert_eq!(
        ty(at(11, KeptAccess::Write)),
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba32Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
        }
    );
    assert_eq!(
        ty(at(8, KeptAccess::Write)),
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        }
    );
    let buffer = |read_only| wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    assert_eq!(ty(at(1, KeptAccess::Read)), buffer(true));
    assert_eq!(ty(at(1, KeptAccess::Write)), buffer(false));
}

/// This frame's `[view_epoch, world_epoch, reset_reasons]` as the uploaded bytes hold them.
fn epochs_of(
    native: &mut super::super::NativeFrame,
    frame: &crate::gfx::Frame,
    size: [u32; 2],
    hdr: bool,
) -> [u32; 3] {
    let display = crate::gfx::output::DisplayValues {
        hdr,
        ..crate::gfx::output::DisplayValues::SDR
    };
    let inputs = native.next_inputs(frame, size, display);
    let bytes = bytemuck::bytes_of(&inputs);
    let offsets = &horizon_shader_api::Frame::OFFSETS[horizon_shader_api::Frame::KEPT_FIELDS];
    assert_eq!(offsets, [796, 800, 804]);
    let at = |o: usize| u32::from_ne_bytes(bytes[o..o + 4].try_into().unwrap());
    let got = [0, 1, 2].map(|i| at(offsets[i]));
    assert_eq!(
        got,
        [inputs.view_epoch, inputs.world_epoch, inputs.reset_reasons]
    );
    got
}

/// #16011 item 4: a frame's own events move the epochs they reset. A pause (a gap over 0.5 s),
/// an R change and an HDR toggle move the view's, a teleport past `reach` both, a shorter move
/// neither, a history reset both; the reasons are that frame's only.
#[test]
fn frame_events_move_the_epochs_they_reset() {
    use horizon_shader_api::kept::*;
    let Some((device, _)) = device() else {
        return;
    };
    let hud = crate::hud::HudBatch::scratch([4, 4]);
    let mut frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
    let mut native =
        super::super::NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
    native.epochs = Some(Epochs::new(Some(64)));
    let mut next = |frame: &crate::gfx::Frame, size, hdr| epochs_of(&mut native, frame, size, hdr);
    assert_eq!(next(&frame, [4, 4], false), [1, 1, REASON_LOAD]);
    assert_eq!(next(&frame, [4, 4], false), [1, 1, 0]);
    drop(next);
    native.last.as_mut().unwrap().2 -= std::time::Duration::from_millis(600);
    let mut next = |frame: &crate::gfx::Frame, size, hdr| epochs_of(&mut native, frame, size, hdr);
    assert_eq!(next(&frame, [4, 4], false), [2, 1, REASON_PAUSE]);
    assert_eq!(next(&frame, [8, 4], false), [3, 1, REASON_SIZE]);
    assert_eq!(next(&frame, [8, 4], true), [4, 1, REASON_DISPLAY]);
    frame.camera.x += 100.0;
    assert_eq!(next(&frame, [8, 4], true), [5, 2, REASON_TELEPORT]);
    frame.camera.x += 30.0;
    assert_eq!(next(&frame, [8, 4], true), [5, 2, 0]);
    drop(next);
    native.reset_history();
    assert_eq!(
        epochs_of(&mut native, &frame, [8, 4], true),
        [6, 3, REASON_RESET]
    );
    // A rebuild that carries (`NativePack::carry`) carries the last frame too: a pause or a
    // teleport across it still resets.
    let new = || {
        super::super::NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false)
    };
    let mut rebuilt = new();
    rebuilt.carry(&mut native);
    rebuilt.last.as_mut().unwrap().2 -= std::time::Duration::from_millis(600);
    assert_eq!(
        epochs_of(&mut rebuilt, &frame, [8, 4], true),
        [7, 3, REASON_PAUSE]
    );
    let mut again = new();
    again.carry(&mut rebuilt);
    frame.camera.x += 100.0;
    assert_eq!(
        epochs_of(&mut again, &frame, [8, 4], true),
        [8, 4, REASON_TELEPORT]
    );
}

/// A pack with kept resources, the options `a` (`resets = "view"`), `b` and `w` (`resets =
/// "world"`), and `file` as its last post pass.
fn with_options(file: &str) -> Report {
    let options = ["a\"\nresets = \"view", "b", "w\"\nresets = \"world"].map(|id| {
        format!("[[options]]\nid = \"{id}\"\nname = \"O\"\ntype = \"bool\"\ndefault = true\n")
    });
    let manifest = format!(
        "{}{BUFFERS}[[post]]\nfile = \"sum.wgsl\"\ncompute = true\ndispatch = [64, 1, 1]\n\
         workgroup = [64, 1, 1]\nwrites = [\"cells\"]\n[[post]]\nfile = \"last.wgsl\"\nreads = [\"cells\"]\n",
        options.concat()
    );
    let file = format!("override a: bool;\noverride b: bool;\noverride w: bool;\n{file}");
    report(&manifest, &[("sum.wgsl", SUM), ("last.wgsl", &file)])
}

/// A rebuild of the same declaration and stages carries the store and its epochs: an unmarked
/// option's change resets nothing, a `resets = "view"` one the view's, `"world"` both. A changed
/// stage doesn't carry, and the new pack's epochs start at 1. A world change (`ray_clear`)
/// moves both; an epoch skips 0 when it wraps.
#[test]
fn a_rebuild_carries_the_epochs_and_options_reset_what_they_name() {
    use horizon_shader_api::kept::*;
    let values =
        |[a, b, w]: [f64; 3]| [("a", a), ("b", b), ("w", w)].map(|(k, v)| (k.to_owned(), v));
    let pack = with_options(LAST);
    let signature = |v| Signature::of(&pack, &values(v)).unwrap();
    let mut epochs = Epochs::new(None);
    let next = |e: &mut Epochs| e.advance([4, 4], false, false, 0.0);
    assert_eq!(next(&mut epochs), [1, 1, REASON_LOAD]);
    let was = signature([1.0; 3]);
    assert!(signature([1.0, 0.0, 1.0]).carry(&was, &mut epochs));
    assert_eq!(next(&mut epochs), [1, 1, 0]);
    assert!(signature([0.0, 1.0, 1.0]).carry(&was, &mut epochs));
    assert_eq!(next(&mut epochs), [2, 1, REASON_OPTION]);
    assert!(signature([1.0, 1.0, 0.0]).carry(&was, &mut epochs));
    assert_eq!(next(&mut epochs), [3, 2, REASON_OPTION]);
    let changed = with_options(&LAST.replace("cells[0]", "cells[1]"));
    let changed = Signature::of(&changed, &values([1.0; 3])).unwrap();
    assert!(!changed.carry(&was, &mut epochs));
    assert_eq!(next(&mut epochs), [3, 2, 0]);
    assert_eq!(next(&mut Epochs::new(None)), [1, 1, REASON_LOAD]);
    epochs.note(REASON_WORLD);
    assert_eq!(next(&mut epochs), [4, 3, REASON_WORLD]);
    epochs.view = u32::MAX;
    epochs.note(REASON_PAUSE);
    assert_eq!(next(&mut epochs), [1, 3, REASON_PAUSE]);
    assert!(
        Signature::of(
            &report(
                "[[post]]\nfile = \"last.wgsl\"\n",
                &[(
                    "last.wgsl",
                    "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n"
                )]
            ),
            &[]
        )
        .is_none()
    );
}

/// `begin` starts a real frame (`frame`) once, however often it is called: it swaps the
/// versions then, records its clears of every shape validly (the view's when the view epoch
/// moved, the world's when the world's did), and says when a new R remade textures. Run with
/// `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn begin_starts_a_frame_once_and_clears_what_the_epochs_moved() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut kept = every_shape();
    for r in kept.resources.iter_mut().step_by(2) {
        r.keep = horizon_shader_api::kept::Keep::World;
    }
    let mut store = KeptStore::new(&device, &kept, [8, 8]);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut begin = |store: &mut KeptStore, size| {
        let remade = store.begin(&device, &mut encoder, size);
        (remade, store.parity, store.cleared)
    };
    assert_eq!(begin(&mut store, [8, 8]), (false, 0, [1, 1]));
    store.frame([1, 1], [8, 8]);
    assert_eq!(begin(&mut store, [8, 8]), (false, 1, [1, 1]));
    assert_eq!(begin(&mut store, [8, 8]), (false, 1, [1, 1]));
    store.frame([2, 1], [8, 8]);
    assert_eq!(begin(&mut store, [8, 8]), (false, 0, [2, 1]));
    store.frame([3, 2], [4, 4]);
    assert_eq!(begin(&mut store, [4, 4]), (true, 1, [3, 2]));
    queue.submit([encoder.finish()]);
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// A device, and its instance, which counts the objects made on it (`generate_report`).
fn counted() -> Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).ok()?;
    Some((instance, device, queue))
}

const IDENTITY: &str = "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n";

/// A 0.2 post chain on 4×4 targets, its tone curve the identity: what its last pass writes
/// comes back on the screen as 8-bit values.
struct Chain {
    frame: super::super::NativeFrame,
    post: super::super::NativePost,
    screen: Screen,
}

/// The world's 4×4 targets a chain reads, and the screen it draws onto.
struct Screen {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    screen: wgpu::Texture,
}

impl Screen {
    fn new(device: &wgpu::Device) -> Self {
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let usage = T::RENDER_ATTACHMENT | T::TEXTURE_BINDING;
        let view = |t: wgpu::Texture| t.create_view(&Default::default());
        Self {
            color: view(texture(wgpu::TextureFormat::Rgba8Unorm, usage)),
            depth: view(texture(wgpu::TextureFormat::Depth32Float, usage)),
            screen: texture(
                wgpu::TextureFormat::Rgba8Unorm,
                T::RENDER_ATTACHMENT | T::COPY_SRC,
            ),
        }
    }

    /// Runs `post` at R `size`, with the frame's `surface`; the screen's first pixel.
    fn show(
        &self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        post: &mut NativePost,
        surface: &wgpu::TextureView,
        size: [u32; 2],
    ) -> [u8; 4] {
        let mut encoder = device.create_command_encoder(&Default::default());
        let world = (&self.color, &self.depth, surface);
        let view = self.screen.create_view(&Default::default());
        post.run(device, &mut encoder, world, &view, size, None, None);
        let buffer = crate::gfx::copy_frame(device, &mut encoder, &self.screen, [4, 4]);
        queue.submit([encoder.finish()]);
        (buffer.slice(..)).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixel = buffer.slice(..).get_mapped_range().unwrap()[..4].try_into();
        pixel.unwrap()
    }
}

impl Chain {
    fn frame(device: &wgpu::Device, queue: &wgpu::Queue) -> super::super::NativeFrame {
        let frame = super::super::NativeFrame::new(
            device,
            None,
            Vec::new(),
            Default::default(),
            None,
            false,
        );
        let inputs = horizon_shader_api::Frame {
            headroom: 1.0,
            ..Default::default()
        };
        queue.write_buffer(&frame.buffer, 0, bytemuck::bytes_of(&inputs));
        frame
    }

    /// The post stage of the pack of `manifest` (after `HEAD`, then `[tonemap]`) and `files`,
    /// installed.
    fn post(
        device: &wgpu::Device,
        frame: &super::super::NativeFrame,
        manifest: &str,
        files: &[(&str, &str)],
    ) -> super::super::NativePost {
        let mut post = Self::built(device, frame, manifest, files);
        install(device, Some(&mut post), None, [1, 1]).unwrap();
        post
    }

    /// `post`'s, not installed: its store holds nothing yet.
    fn built(
        device: &wgpu::Device,
        frame: &super::super::NativeFrame,
        manifest: &str,
        files: &[(&str, &str)],
    ) -> super::super::NativePost {
        let manifest = format!("{manifest}[tonemap]\nfile = \"t.wgsl\"\n");
        let report = report(&manifest, &[files, &[("t.wgsl", IDENTITY)]].concat());
        let (passes, others): (Vec<_>, Vec<_>) =
            (report.stages.into_iter()).partition(|s| s.kind != StageKind::Tonemap);
        let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        super::super::NativePost::new(
            device,
            passes,
            &report.manifest,
            &[],
            format,
            frame,
            tonemap,
        )
        .unwrap()
    }

    fn new(device: &wgpu::Device, frame: super::super::NativeFrame, post: NativePost) -> Self {
        let screen = Screen::new(device);
        Self {
            frame,
            post,
            screen,
        }
    }

    /// Starts a frame at R `size` (`frame.resolution` too), its epochs `epochs`, and runs it.
    fn next(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        epochs: [u32; 2],
        size: [u32; 2],
    ) -> [u8; 4] {
        let inputs = horizon_shader_api::Frame {
            headroom: 1.0,
            resolution: size.map(|v| v as f32),
            ..Default::default()
        };
        queue.write_buffer(&self.frame.buffer, 0, bytemuck::bytes_of(&inputs));
        self.post.kept_frame(epochs, size);
        self.run(device, queue, size)
    }

    /// Runs the chain at R `size`, its `[[before_world]]` passes first, as a frame does; the
    /// screen's first pixel.
    fn run(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2]) -> [u8; 4] {
        let mut encoder = device.create_command_encoder(&Default::default());
        self.post.before_world(device, &mut encoder, None, None);
        queue.submit([encoder.finish()]);
        let surface = &self.frame.surface_view;
        (self.screen).show((device, queue), &mut self.post, surface, size)
    }
}

/// A pass writing `hist` (R-relative, adding a quarter to last frame's), one writing the
/// `r32float` `acc`, and the last showing `hist_previous`, `hist`'s alpha (the pass's, not 1)
/// and `acc` as red, green, blue.
const KEPT_CHAIN: &str = "[[resource]]\nname = \"hist\"\nformat = \"rgba16float\"\nscale = 1.0\n\
                          keep = \"view\"\n[[resource]]\nname = \"acc\"\nformat = \"r32float\"\n\
                          size = [4, 4]\nkeep = \"view\"\n\
                          [[post]]\nfile = \"h.wgsl\"\noutput = \"hist\"\nreads = [\"hist_previous\"]\n\
                          [[post]]\nfile = \"a.wgsl\"\noutput = \"acc\"\n\
                          [[post]]\nfile = \"l.wgsl\"\nreads = [\"hist\", \"hist_previous\", \"acc\"]\n";
const KEPT_FILES: [(&str, &str); 3] = [
    (
        "h.wgsl",
        "fn post(input: PostInput) -> vec4<f32> {\n    \
         return textureLoad(hist_previous, vec2<i32>(input.pixel), 0) + 0.25;\n}\n",
    ),
    (
        "a.wgsl",
        "fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(0.6); }\n",
    ),
    (
        "l.wgsl",
        "fn post(input: PostInput) -> vec4<f32> {\n    let p = vec2<i32>(input.pixel);\n    \
         let previous = textureLoad(hist_previous, p, 0).r;\n    \
         return vec4<f32>(previous, textureLoad(hist, p, 0).a, textureLoad(acc, p, 0).r, 1.0);\n}\n",
    ),
];

/// #16011 item 4, P4a: a pass's `output` writes this frame's version of a kept texture, which
/// the next frame reads as `_previous`; a later pass reads it, and an `r32float` one with
/// `textureLoad`. Another run in the same frame swaps nothing. A moved epoch clears both
/// versions, and a new R remakes the R-relative pair as zeros. Run with `WGPU_BACKEND=vulkan`
/// and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_kept_texture_ping_pongs_and_starts_over_on_a_reset_and_a_new_size() {
    let Some((_, device, queue)) = counted() else {
        return;
    };
    let frame = Chain::frame(&device, &queue);
    let post = Chain::post(&device, &frame, KEPT_CHAIN, &KEPT_FILES);
    let mut chain = Chain::new(&device, frame, post);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let next = |chain: &mut Chain, epochs, size| {
        chain.post.kept_frame(epochs, size);
        chain.run(&device, &queue, size)
    };
    // The screen holds light encoded (`horizon_encode`).
    let near = |got: [u8; 4], want: [f32; 3]| {
        let shown = want.map(|v| (v.powf(1.0 / 2.2) * 255.0).round() as u8);
        got.iter().zip(shown).all(|(g, w)| g.abs_diff(w) <= 1)
    };
    for want in [[0.0, 0.25, 0.6], [0.25, 0.5, 0.6], [0.5, 0.75, 0.6]] {
        let got = next(&mut chain, [1, 1], [4, 4]);
        assert!(near(got, want), "{got:?}, {want:?} expected");
    }
    let again = chain.run(&device, &queue, [4, 4]);
    assert!(near(again, [0.5, 0.75, 0.6]), "{again:?} run again");
    // A history reset moves the view epoch (`super::super::NativeFrame::reset_history`).
    let reset = next(&mut chain, [2, 1], [4, 4]);
    assert!(near(reset, [0.0, 0.25, 0.6]), "{reset:?} after the reset");
    let resized = next(&mut chain, [2, 1], [2, 2]);
    assert!(near(resized, [0.0, 0.25, 0.6]), "{resized:?} at a new R");
    // Without the video memory for them at a larger R (`NO_ROOM` stands in for a device out of
    // it: an out-of-memory error raised outside a scope panics the game), the R-relative pair
    // stays at the old R while the passes run at the new one. That R isn't tried every frame,
    // but is after the old R comes back (a window going fullscreen, back, and again).
    let held = |chain: &Chain| {
        let k = chain.post.kept.as_ref().unwrap();
        let Held::Pair(hist) = &k.held[0] else {
            unreachable!("`hist` is a texture")
        };
        let texture = hist[0].texture();
        ([texture.width(), texture.height()], k.size, k.refused)
    };
    NO_ROOM.set(1);
    let kept = next(&mut chain, [2, 1], [4, 4]);
    assert!(near(kept, [0.25, 0.5, 0.6]), "{kept:?} without room");
    assert_eq!(held(&chain), ([2, 2], [2, 2], Some([4, 4])));
    let kept = next(&mut chain, [2, 1], [4, 4]);
    assert!(near(kept, [0.5, 0.75, 0.6]), "{kept:?} at the refused R");
    assert_eq!(held(&chain), ([2, 2], [2, 2], Some([4, 4])));
    // A pack without room for its store is refused at install (`a_reload_never_holds_two_stores`
    // counts what it leaves). The running store, freed first, is made again
    // (`NativePack::reload_kept`) at the R it was last made at, not the frame's: at the refused
    // R the store would have room for neither, and the running pack would stop. That R still
    // isn't tried every frame.
    let mut other = Chain::built(&device, &chain.frame, KEPT_CHAIN, &KEPT_FILES);
    NO_ROOM.set(1);
    let refused = install(&device, Some(&mut other), Some(&mut chain.post), [4, 4]);
    assert!(refused.is_err());
    reinstall(&device, &mut chain.post).unwrap();
    assert_eq!(held(&chain), ([2, 2], [2, 2], Some([4, 4])));
    let fresh = next(&mut chain, [3, 2], [4, 4]);
    assert!(near(fresh, [0.0, 0.25, 0.6]), "{fresh:?} made again");
    assert_eq!(held(&chain), ([2, 2], [2, 2], Some([4, 4])));
    next(&mut chain, [3, 2], [2, 2]);
    assert_eq!(held(&chain), ([2, 2], [2, 2], None));
    let grown = next(&mut chain, [2, 1], [4, 4]);
    assert!(
        near(grown, [0.0, 0.25, 0.6]),
        "{grown:?} at the R refused before"
    );
    assert_eq!(held(&chain), ([4, 4], [4, 4], None));
    // Refused where the store was last made at the frame's R: its groups would be kept but bind
    // the freed textures.
    NO_ROOM.set(1);
    let refused = install(&device, Some(&mut other), Some(&mut chain.post), [4, 4]);
    assert!(refused.is_err());
    reinstall(&device, &mut chain.post).unwrap();
    assert_eq!(held(&chain), ([4, 4], [4, 4], None));
    let fresh = next(&mut chain, [4, 3], [4, 4]);
    assert!(near(fresh, [0.0, 0.25, 0.6]), "{fresh:?} made again");
    // With room, a store declared before any frame (at 1×1) is made at the frame's R.
    install(&device, Some(&mut other), Some(&mut chain.post), [4, 4]).unwrap();
    chain.post = other;
    assert_eq!(held(&chain), ([4, 4], [4, 4], None));
    let first = next(&mut chain, [5, 4], [4, 4]);
    assert!(
        near(first, [0.0, 0.25, 0.6]),
        "{first:?} the other pack's first frame"
    );
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// An `rgba32uint` kept texture binds to a post pass as `texture_2d<u32>`, both versions read
/// with `textureLoad`: zeros, until compute passes write it, across a swap and a clear. Run with
/// `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_post_pass_loads_an_rgba32uint_texture_and_its_previous() {
    let Some((device, queue)) = device() else {
        return;
    };
    let manifest = "[[resource]]\nname = \"ints\"\nformat = \"rgba32uint\"\nsize = [4, 4]\n\
                    keep = \"view\"\n[[post]]\nfile = \"l.wgsl\"\nreads = [\"ints\", \"ints_previous\"]\n";
    let read = "fn post(input: PostInput) -> vec4<f32> {\n    let p = vec2<i32>(input.pixel);\n    \
                let n = textureLoad(ints, p, 0) + textureLoad(ints_previous, p, 0);\n    \
                return vec4<f32>(0.5 + f32(n.x + n.w), 0.25, 0.0, 1.0);\n}\n";
    let frame = Chain::frame(&device, &queue);
    let post = Chain::post(&device, &frame, manifest, &[("l.wgsl", read)]);
    let mut chain = Chain::new(&device, frame, post);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // 0.5 and 0.25, encoded (`horizon_encode`).
    let shown = [0.5f32, 0.25, 0.0].map(|v| (v.powf(1.0 / 2.2) * 255.0).round() as u8);
    for epochs in [[1, 1], [1, 1], [2, 1]] {
        chain.post.kept_frame(epochs, [4, 4]);
        let got = chain.run(&device, &queue, [4, 4]);
        let near = got.iter().zip(shown).all(|(g, w)| g.abs_diff(w) <= 1);
        assert!(near, "{got:?}, {shown:?} expected");
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// `want` as the screen shows it (`horizon_encode`), within 1 of `got`.
fn shows(got: [u8; 4], want: [f32; 3]) -> bool {
    let shown = want.map(|v| (v.powf(1.0 / 2.2) * 255.0).round() as u8);
    got.iter().zip(shown).all(|(g, w)| g.abs_diff(w) <= 1)
}

/// The chain of `manifest` and `files` over `frames` (epochs, R, the colour expected), with no
/// validation error; a second run in the last frame dispatches nothing and shows the same.
fn computes(manifest: &str, files: &[(&str, &str)], frames: &[([u32; 2], [u32; 2], [f32; 3])]) {
    let Some((device, queue)) = device() else {
        return;
    };
    let frame = Chain::frame(&device, &queue);
    let post = Chain::post(&device, &frame, manifest, files);
    let mut chain = Chain::new(&device, frame, post);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    for (n, &(epochs, size, want)) in frames.iter().enumerate() {
        let got = chain.next(&device, &queue, epochs, size);
        assert!(shows(got, want), "frame {n}: {got:?}, {want:?} expected");
    }
    let (_, size, want) = frames[frames.len() - 1];
    let again = chain.run(&device, &queue, size);
    assert!(
        shows(again, want),
        "run again: {again:?}, {want:?} expected"
    );
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// #16011 item 4, P4b: a compute pass writes this frame's version of an R-relative kept texture
/// (adding to `_previous`), and a later pass shows it; the compute pass doesn't advance
/// `previous`, so that pass's colour is the first pass's blue. A reset clears it. Run with
/// `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_compute_pass_writes_a_kept_texture_a_later_pass_shows() {
    let manifest = "[[resource]]\nname = \"img\"\nformat = \"rgba16float\"\nscale = 1.0\n\
                    keep = \"view\"\n[[post]]\nfile = \"b.wgsl\"\n\
                    [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = \"pixels\"\n\
                    workgroup = [8, 8, 1]\nreads = [\"img_previous\"]\nwrites = [\"img\"]\n\
                    [[post]]\nfile = \"l.wgsl\"\nreads = [\"img\"]\n";
    let files = [
        (
            "b.wgsl",
            "fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.6, 1.0); }\n",
        ),
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) {\n    if input.in_bounds {\n        \
             let last = textureLoad(img_previous, input.cell.xy, 0);\n        \
             textureStore(img, input.cell.xy, last + vec4<f32>(0.25, 0.125, 0.0, 0.0));\n    }\n}\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             let kept = textureLoad(img, vec2<i32>(input.pixel), 0);\n    \
             return vec4<f32>(kept.rg, input.color.b, 1.0);\n}\n",
        ),
    ];
    let frames = [
        ([1, 1], [4, 4], [0.25, 0.125, 0.6]),
        ([1, 1], [4, 4], [0.5, 0.25, 0.6]),
        ([2, 1], [4, 4], [0.25, 0.125, 0.6]),
    ];
    computes(manifest, &files, &frames);
}

/// A 1024×1024 fixed-size kept texture (16 MiB with both versions, past the old 1 MiB budget)
/// builds, installs and is written as a pass's `output` at its own size, adding to `_previous`;
/// the last pass reads it at the screen's pixel and at its far corner, and a reset clears it.
/// Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_large_fixed_size_kept_texture_is_written_read_and_cleared() {
    let manifest = "[[resource]]\nname = \"trail\"\nformat = \"rgba16float\"\nsize = [1024, 1024]\n\
                    keep = \"view\"\n[[post]]\nfile = \"w.wgsl\"\noutput = \"trail\"\n\
                    reads = [\"trail_previous\"]\n[[post]]\nfile = \"l.wgsl\"\nreads = [\"trail\"]\n";
    let files = [
        (
            "w.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             let last = textureLoad(trail_previous, vec2<i32>(input.pixel), 0);\n    \
             return last + vec4<f32>(0.25, 0.125, 0.0, 0.0);\n}\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             let here = textureLoad(trail, vec2<i32>(input.pixel), 0);\n    \
             let corner = textureLoad(trail, vec2<i32>(1023, 1023), 0);\n    \
             return vec4<f32>(here.r, corner.g, 0.0, 1.0);\n}\n",
        ),
    ];
    let frames = [
        ([1, 1], [4, 4], [0.25, 0.125, 0.0]),
        ([1, 1], [4, 4], [0.5, 0.25, 0.0]),
        ([2, 1], [4, 4], [0.25, 0.125, 0.0]),
    ];
    computes(manifest, &files, &frames);
}

/// A compute pass adding one to the kept buffer `sum` each frame, and the last pass showing it
/// ×0.25 in red.
const SUMMING: (&str, [(&str, &str); 2]) = (
    "[[resource]]\nname = \"sum\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
     [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [1, 1, 1]\n\
     workgroup = [1, 1, 1]\nwrites = [\"sum\"]\n[[post]]\nfile = \"l.wgsl\"\nreads = [\"sum\"]\n",
    [
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) { sum[0] = sum[0] + 1u; }\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             return vec4<f32>(f32(sum[0]) * 0.25, 0.0, 0.0, 1.0);\n}\n",
        ),
    ],
);

/// A compute pass adds one to a kept buffer each frame, which the last pass shows; a history
/// reset (the view epoch) starts it over. Run with `WGPU_BACKEND=vulkan` and
/// `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_compute_pass_accumulates_in_a_buffer_and_starts_over_on_a_reset() {
    let (manifest, files) = SUMMING;
    let frames = [
        ([1, 1], [4, 4], [0.25, 0.0, 0.0]),
        ([1, 1], [4, 4], [0.5, 0.0, 0.0]),
        ([1, 1], [4, 4], [0.75, 0.0, 0.0]),
        ([2, 1], [4, 4], [0.25, 0.0, 0.0]),
    ];
    computes(manifest, &files, &frames);
}

/// A compute pass writes an `rgba32uint` kept texture (adding to `_previous`), which a post pass
/// loads. Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_compute_pass_writes_an_rgba32uint_texture_a_post_pass_loads() {
    let manifest = "[[resource]]\nname = \"ints\"\nformat = \"rgba32uint\"\nsize = [4, 4]\n\
                    keep = \"view\"\n[[post]]\nfile = \"c.wgsl\"\ncompute = true\n\
                    dispatch = [4, 4, 1]\nworkgroup = [4, 4, 1]\nreads = [\"ints_previous\"]\n\
                    writes = [\"ints\"]\n[[post]]\nfile = \"l.wgsl\"\nreads = [\"ints\"]\n";
    let files = [
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) {\n    \
             let last = textureLoad(ints_previous, input.cell.xy, 0);\n    \
             textureStore(ints, input.cell.xy, last + vec4<u32>(1u, 2u, 0u, 0u));\n}\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             let n = textureLoad(ints, vec2<i32>(input.pixel), 0);\n    \
             return vec4<f32>(vec2<f32>(n.xy) * 0.125, 0.0, 1.0);\n}\n",
        ),
    ];
    let frames = [
        ([1, 1], [4, 4], [0.125, 0.25, 0.0]),
        ([1, 1], [4, 4], [0.25, 0.5, 0.0]),
    ];
    computes(manifest, &files, &frames);
}

/// A compute pass writes an `r32uint` kept texture (adding 3 to `_previous` in every channel),
/// which a post pass loads: red the count / 8, green the second channel, which `r32uint` doesn't
/// keep (it reads 0). A reset clears it. Run with `WGPU_BACKEND=vulkan` and
/// `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_compute_pass_writes_an_r32uint_texture_a_post_pass_loads() {
    let manifest = "[[resource]]\nname = \"ints\"\nformat = \"r32uint\"\nsize = [4, 4]\n\
                    keep = \"view\"\n[[post]]\nfile = \"c.wgsl\"\ncompute = true\n\
                    dispatch = [4, 4, 1]\nworkgroup = [4, 4, 1]\nreads = [\"ints_previous\"]\n\
                    writes = [\"ints\"]\n[[post]]\nfile = \"l.wgsl\"\nreads = [\"ints\"]\n";
    let files = [
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) {\n    \
             let last = textureLoad(ints_previous, input.cell.xy, 0);\n    \
             textureStore(ints, input.cell.xy, last + vec4<u32>(3u));\n}\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             let n = textureLoad(ints, vec2<i32>(input.pixel), 0);\n    \
             return vec4<f32>(vec2<f32>(n.xy) * 0.125, 0.0, 1.0);\n}\n",
        ),
    ];
    let frames = [
        ([1, 1], [4, 4], [0.375, 0.0, 0.0]),
        ([1, 1], [4, 4], [0.75, 0.0, 0.0]),
        ([2, 1], [4, 4], [0.375, 0.0, 0.0]),
    ];
    computes(manifest, &files, &frames);
}

/// The pack of `manifest` (with the identity tone curve) and `files` through the renderer's own
/// path: built by `NativePack::new` and installed, each frame's inputs written by
/// `NativePack::write`, which hands the kept store its frame (`kept_frame`), its
/// `[[before_world]]` passes recorded by `NativePack::before_world`, then its chain; the
/// screen's first pixel each frame, red `wants`.
fn through_pack(manifest: &str, files: &[(&str, &str)], wants: &[f32]) {
    through_pack_frames(manifest, files, wants, |_, _| {}, true);
}

/// `through_pack`, with `set` changing the renderer's frame before each (by its index), and
/// with `refusal` the store's refusal on a device without room for it after them, which only a
/// pack with `[[resource]]` has.
fn through_pack_frames(
    manifest: &str,
    files: &[(&str, &str)],
    wants: &[f32],
    set: impl Fn(&mut crate::gfx::Frame, usize),
    refusal: bool,
) {
    let Some((device, queue)) = device() else {
        return;
    };
    let (globals, empty) = crate::gfx::native_pack::tests::terrain_layouts(&device);
    let terrain = crate::gfx::native_pack::tests::terrain_shape(&globals, &empty);
    let manifest = format!("{manifest}[tonemap]\nfile = \"t.wgsl\"\n");
    let report = report(&manifest, &[files, &[("t.wgsl", IDENTITY)]].concat());
    let built = crate::gfx::native_pack::NativePack::new(
        &device,
        &queue,
        &terrain,
        report,
        &[],
        Default::default(),
    );
    let (mut pack, _) = built.unwrap();
    install(&device, Some(&mut pack.post), None, [1, 1]).unwrap();
    let screen = Screen::new(&device);
    let hud = crate::hud::HudBatch::scratch([4, 4]);
    let mut frame = crate::gfx::pack_uniforms::initial_history_tests::frame(&hud);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    for (n, &want) in (1..).zip(wants) {
        set(&mut frame, n as usize - 1);
        // A scripted clock: a slow frame on a loaded machine is no pause that resets the store.
        let timing = crate::gfx::pack_uniforms::FrameTiming::nth(n, 50.0);
        frame.pack.frame_timing = Some(timing);
        let display = crate::gfx::output::DisplayValues::SDR;
        pack.write(&queue, &frame, [4, 4], display, None);
        let mut encoder = device.create_command_encoder(&Default::default());
        pack.before_world(&device, &mut encoder, None);
        queue.submit([encoder.finish()]);
        let post = &mut pack.post;
        let got = screen.show((&device, &queue), post, &pack.frame.surface_view, [4, 4]);
        assert!(shows(got, [want, 0.0, 0.0]), "{got:?}, {want} expected");
    }
    if !refusal {
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        return;
    }
    // The same pack again, on a device without the video memory for its store (`NO_ROOM`):
    // refused (`install_kept`), and this one keeps running, its store made again and its
    // frames counting from a load.
    let again = self::report(&manifest, &[files, &[("t.wgsl", IDENTITY)]].concat());
    let built = crate::gfx::native_pack::NativePack::new(
        &device,
        &queue,
        &terrain,
        again,
        &[],
        Default::default(),
    );
    let (mut other, _) = built.unwrap();
    NO_ROOM.set(1);
    let install = crate::gfx::native_pack::install_kept;
    let refused = install(&device, Some(&mut other), Some(&mut pack), [4, 4]);
    assert!(matches!(refused, Err((_, true))), "{refused:?}");
    let epochs = pack.frame.epochs.as_ref().unwrap();
    assert_ne!(epochs.reasons & horizon_shader_api::kept::REASON_LOAD, 0);
    let timing = crate::gfx::pack_uniforms::FrameTiming::nth(wants.len() as u32 + 1, 50.0);
    frame.pack.frame_timing = Some(timing);
    pack.write(
        &queue,
        &frame,
        [4, 4],
        crate::gfx::output::DisplayValues::SDR,
        None,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    pack.before_world(&device, &mut encoder, None);
    queue.submit([encoder.finish()]);
    let got = screen.show(
        (&device, &queue),
        &mut pack.post,
        &pack.frame.surface_view,
        [4, 4],
    );
    assert!(
        shows(got, [wants[0], 0.0, 0.0]),
        "{got:?} after the refusal"
    );
    // Without room for either store, the running pack can't keep running.
    NO_ROOM.set(2);
    let refused = install(&device, Some(&mut other), Some(&mut pack), [4, 4]);
    assert!(matches!(refused, Err((_, false))), "{refused:?}");
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// A pack's own frames run its compute pass (`through_pack`): without `NativePack::write`
/// handing the store its frame, the store never starts one and the pass never runs. Run with
/// `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_pack_s_own_frames_run_its_compute_pass() {
    let (manifest, files) = SUMMING;
    through_pack(manifest, &files, &[0.25, 0.5, 0.75]);
}

/// A pack whose files name `moon_phase` reads the phase the renderer's frame carries
/// (`Sky::moon_phase`), through `NativePack::write`, where its prelude declares the field: each
/// frame shows its own phase. Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each
/// backend.
#[test]
fn a_pack_reads_the_moon_phase_the_renderer_draws() {
    let files = [(
        "p.wgsl",
        "fn post(input: PostInput) -> vec4<f32> {\n    \
         return vec4<f32>(f32(frame.moon_phase) * 0.125, 0.0, 0.0, 1.0);\n}\n",
    )];
    let phases = [2, 5, 7];
    let wants = phases.map(|p| p as f32 * 0.125);
    let manifest = "[[post]]\nfile = \"p.wgsl\"\n";
    let set = |frame: &mut crate::gfx::Frame, n: usize| frame.sky.moon_phase = phases[n];
    through_pack_frames(manifest, &files, &wants, set, false);
}

/// `SUMMING` with its compute pass before the world (`[[before_world]]`) instead.
const BEFORE_SUMMING: (&str, [(&str, &str); 2]) = (
    "[[resource]]\nname = \"sum\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
     [[before_world]]\nfile = \"c.wgsl\"\ndispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\n\
     writes = [\"sum\"]\n[[post]]\nfile = \"l.wgsl\"\nreads = [\"sum\"]\n",
    SUMMING.1,
);

/// #16011 item 4, P5: a `[[before_world]]` pass adds one to a kept buffer once a frame, which
/// the last pass shows; a history reset starts it over, and running the frame's passes again
/// dispatches nothing. Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_before_world_pass_accumulates_in_a_buffer_and_starts_over_on_a_reset() {
    let (manifest, files) = BEFORE_SUMMING;
    let frames = [
        ([1, 1], [4, 4], [0.25, 0.0, 0.0]),
        ([1, 1], [4, 4], [0.5, 0.0, 0.0]),
        ([2, 1], [4, 4], [0.25, 0.0, 0.0]),
    ];
    computes(manifest, &files, &frames);
}

/// A pack's own frames run its `[[before_world]]` pass (`through_pack`): without
/// `NativePack::before_world`, the call the renderer makes after the shadow passes, it never
/// runs. Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_pack_s_own_frames_run_its_before_world_pass() {
    let (manifest, files) = BEFORE_SUMMING;
    through_pack(manifest, &files, &[0.25, 0.5, 0.75]);
}

/// A pack without the video memory for its kept resources (`NO_ROOM`) is refused through the
/// hook the renderer installs every pipeline with (`ExternalPipeline::take_over`, from
/// `Renderer::replace_executor`), which says why: the pack that ran keeps running, its store
/// made again, unless there is no room for that either.
#[test]
fn a_pack_without_room_is_refused_through_the_pipeline_hook() {
    use crate::gfx::external::{ExternalPipeline, Refused};
    use crate::gfx::native_pack::{NativePack, tests};
    let Some((device, queue)) = device() else {
        return;
    };
    let (globals, empty) = tests::terrain_layouts(&device);
    let terrain = tests::terrain_shape(&globals, &empty);
    // Two packs that keep the same buffer, its pass at another stage: no rebuild of the other,
    // so neither carries the other's store.
    let pack = |(manifest, files): (&str, [(&str, &str); 2])| {
        let manifest = format!("{manifest}[tonemap]\nfile = \"t.wgsl\"\n");
        let report = report(&manifest, &[&files[..], &[("t.wgsl", IDENTITY)]].concat());
        let built = NativePack::new(&device, &queue, &terrain, report, &[], Default::default());
        Box::new(built.unwrap().0)
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut running: Box<dyn ExternalPipeline> = pack(SUMMING);
    running.take_over(&device, None, [4, 4]).unwrap();
    let held = |p: &mut Box<dyn ExternalPipeline>| {
        let p = (p.as_mut() as &mut dyn std::any::Any).downcast_mut::<NativePack>();
        p.and_then(|p| p.post.kept.as_ref()).map(|k| k.held.len())
    };
    assert_eq!(held(&mut running), Some(1));
    let mut other: Box<dyn ExternalPipeline> = pack(BEFORE_SUMMING);
    NO_ROOM.set(1);
    let refused = other.take_over(&device, Some(running.as_mut()), [4, 4]);
    let why = "its kept resources (1 MiB) don't fit in video memory (no room (test))";
    let still = Refused {
        why: why.into(),
        old_runs: true,
    };
    assert_eq!(refused, Err(still));
    assert_eq!(held(&mut running), Some(1));
    assert_eq!(held(&mut other), Some(0));
    // Without room for either store, the running pack can't keep running.
    NO_ROOM.set(2);
    let refused = other.take_over(&device, Some(running.as_mut()), [4, 4]);
    assert!(
        matches!(
            refused,
            Err(Refused {
                old_runs: false,
                ..
            })
        ),
        "{refused:?}"
    );
    // With room, it takes over.
    other
        .take_over(&device, Some(running.as_mut()), [4, 4])
        .unwrap();
    assert_eq!(held(&mut other), Some(1));
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
}

/// A `[[before_world]]` pass writes this frame's version of an R-relative kept texture (adding
/// to `_previous`), after the frame's swap, and the post pass shows it in the same frame. Its
/// `"pixels"` dispatch and `input.size` are R (blue: R's width / 16), and it reads the frame:
/// `frame.headroom` (1) in green. It uses every other binding the stage has (the samplers and
/// the shadow maps), adding nothing, so a group without one is refused. Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for
/// each backend.
#[test]
fn a_before_world_pass_writes_a_kept_texture_a_post_pass_shows() {
    let manifest = "[[resource]]\nname = \"img\"\nformat = \"rgba16float\"\nscale = 1.0\n\
                    keep = \"view\"\n[[before_world]]\nfile = \"w.wgsl\"\ndispatch = \"pixels\"\n\
                    workgroup = [8, 8, 1]\nreads = [\"img_previous\"]\nwrites = [\"img\"]\n\
                    [[post]]\nfile = \"l.wgsl\"\nreads = [\"img\"]\n";
    let files = [
        (
            "w.wgsl",
            "fn compute(input: ComputeInput) {\n    if input.in_bounds {\n        \
             let last = textureLoad(img_previous, input.cell.xy, 0).r;\n        \
             let wide = f32(input.size.x) / 16.0;\n        \
             let p = vec2<f32>(0.5);\n        \
             let maps = textureDimensions(shadow_map).x + textureDimensions(shadow_map_translucent).x;\n        \
             let lit = textureSampleCompareLevel(shadow_map, shadow_sampler, p, 0.5);\n        \
             let tints = textureSampleLevel(shadow_color, linear_sampler, p, 0.0).r\n            \
             + textureSampleLevel(shadow_color, nearest_sampler, p, 0.0).r;\n        \
             let none = (f32(maps) + lit + tints) * 0.0;\n        \
             let here = vec4<f32>(last + 0.25, frame.headroom * 0.5, wide + none, 1.0);\n        \
             textureStore(img, input.cell.xy, here);\n    }\n}\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             return vec4<f32>(textureLoad(img, vec2<i32>(input.pixel), 0).rgb, 1.0);\n}\n",
        ),
    ];
    let frames = [
        ([1, 1], [4, 4], [0.25, 0.5, 0.25]),
        ([1, 1], [4, 4], [0.5, 0.5, 0.25]),
        ([2, 1], [4, 4], [0.25, 0.5, 0.25]),
    ];
    computes(manifest, &files, &frames);
}

/// A `[[before_world]]` pass and a compute `[[post]]` pass in one pack, each adding one to its
/// own element of a kept buffer, shown in red and green: the frame starts before the world, and
/// the chain's compute pass still runs once in it. Run with `WGPU_BACKEND=vulkan` and
/// `WGPU_BACKEND=dx12` for each backend.
#[test]
fn before_world_and_compute_post_passes_both_run_once_a_frame() {
    let manifest = "[[resource]]\nname = \"sum\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
                    [[before_world]]\nfile = \"w.wgsl\"\ndispatch = [1, 1, 1]\n\
                    workgroup = [1, 1, 1]\nwrites = [\"sum\"]\n\
                    [[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [1, 1, 1]\n\
                    workgroup = [1, 1, 1]\nwrites = [\"sum\"]\n\
                    [[post]]\nfile = \"l.wgsl\"\nreads = [\"sum\"]\n";
    let files = [
        (
            "w.wgsl",
            "fn compute(input: ComputeInput) { sum[0] = sum[0] + 1u; }\n",
        ),
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) { sum[1] = sum[1] + 1u; }\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    \
             return vec4<f32>(f32(sum[0]) * 0.25, f32(sum[1]) * 0.125, 0.0, 1.0);\n}\n",
        ),
    ];
    let frames = [
        ([1, 1], [4, 4], [0.25, 0.125, 0.0]),
        ([1, 1], [4, 4], [0.5, 0.25, 0.0]),
    ];
    computes(manifest, &files, &frames);
}

/// A pack with kept resources and a `[[before_world]]` pass but no `[[post]]` pass (a 0.2 pack
/// still has its decode and tone map) builds, and its frames run it without a validation
/// error, the screen the world's picture (black). Run with `WGPU_BACKEND=vulkan` and
/// `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_before_world_pass_runs_without_a_post_chain() {
    let manifest = "[[resource]]\nname = \"sum\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
                    [[before_world]]\nfile = \"w.wgsl\"\ndispatch = [1, 1, 1]\n\
                    workgroup = [1, 1, 1]\nwrites = [\"sum\"]\n";
    let files = [(
        "w.wgsl",
        "fn compute(input: ComputeInput) { sum[0] = sum[0] + 1u; }\n",
    )];
    computes(manifest, &files, &[([1, 1], [4, 4], [0.0, 0.0, 0.0])]);
}

/// The cap on compute work a frame is the API's, checked when the pack loads
/// (`horizon_shader_api::kept::MAX_FRAME_INVOCATIONS`, over `[[before_world]]` and compute
/// `[[post]]` passes together): a pack at it builds and runs a frame with nothing refused at
/// run time; a workgroup more is refused at load. Run with `WGPU_BACKEND=vulkan` and
/// `WGPU_BACKEND=dx12` for each backend.
#[test]
fn the_frame_s_compute_cap_is_the_api_s() {
    use horizon_shader_api::kept::MAX_FRAME_INVOCATIONS;
    let half = "dispatch = [8192, 4096, 1]\nworkgroup = [256, 1, 1]\nwrites = [\"sum\"]\n";
    let manifest = |extra: &str| {
        format!(
            "[[resource]]\nname = \"sum\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
             [[before_world]]\nfile = \"c.wgsl\"\n{half}\
             [[post]]\nfile = \"c.wgsl\"\ncompute = true\n{half}{extra}\
             [[post]]\nfile = \"l.wgsl\"\nreads = [\"sum\"]\n"
        )
    };
    let compute = "fn compute(input: ComputeInput) { if input.cell.x == 0u { sum[0] = 1u; } }\n";
    let files = [("c.wgsl", compute), ("l.wgsl", SUMMING.1[1].1)];
    assert_eq!(2 * 8192 * 4096, MAX_FRAME_INVOCATIONS);
    computes(&manifest(""), &files, &[([1, 1], [4, 4], [0.25, 0.0, 0.0])]);
    let over = "[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [1, 1, 1]\n\
                workgroup = [1, 1, 1]\nwrites = [\"sum\"]\n";
    let mut all: std::collections::BTreeMap<String, Vec<u8>> = (files.iter())
        .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
        .collect();
    all.insert(
        MANIFEST.into(),
        format!("{HEAD}{}", manifest(over)).into_bytes(),
    );
    let refused = Pack::from_files(all).err().unwrap_or_default();
    assert!(refused.contains("a pack may run 67108864"), "{refused}");
}

/// `"pixels"` dispatches R in whole workgroups: at an R of 3×5 with 2×2 workgroups, 4×6
/// invocations write an 8×8 texture, the 15 in bounds 1 and the rest 0.5, which the last pass
/// adds up. Run with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_pixels_dispatch_covers_a_render_size_off_the_workgroup() {
    let manifest = "[[resource]]\nname = \"hit\"\nformat = \"r32float\"\nsize = [8, 8]\n\
                    keep = \"view\"\n[[post]]\nfile = \"c.wgsl\"\ncompute = true\n\
                    dispatch = \"pixels\"\nworkgroup = [2, 2, 1]\nwrites = [\"hit\"]\n\
                    [[post]]\nfile = \"l.wgsl\"\nreads = [\"hit\"]\n";
    let files = [
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) {\n    \
             textureStore(hit, input.cell.xy, vec4<f32>(select(0.5, 1.0, input.in_bounds)));\n}\n",
        ),
        (
            "l.wgsl",
            "fn post(input: PostInput) -> vec4<f32> {\n    var n = 0.0;\n    \
             for (var i = 0; i < 64; i++) { n += textureLoad(hit, vec2<i32>(i % 8, i / 8), 0).r; }\n    \
             return vec4<f32>(n / 32.0, 0.0, 0.0, 1.0);\n}\n",
        ),
    ];
    computes(
        manifest,
        &files,
        &[([1, 1], [3, 5], [19.5 / 32.0, 0.0, 0.0])],
    );
}

/// Kept resources cost a pack without them nothing: its chain makes one group layout, and on
/// its first frame a bind group a pass and one for the tonemap, as before them; none on the
/// next frame. A pack with them makes a group layout a pass besides, and its groups once for
/// each parity, none after; one with a compute or `[[before_world]]` pass, its pipeline too,
/// while it builds.
#[test]
fn a_pack_without_kept_resources_makes_the_layouts_and_groups_it_made_before() {
    let Some((instance, device, queue)) = counted() else {
        return;
    };
    let count = || {
        let hub = instance.generate_report()?.hub;
        let counts = [
            hub.bind_group_layouts,
            hub.bind_groups,
            hub.compute_pipelines,
        ];
        Some(counts.map(|c| c.num_allocated))
    };
    let pass = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    let plain =
        "[[post]]\nfile = \"a.wgsl\"\n[[post]]\nfile = \"b.wgsl\"\n[[post]]\nfile = \"l.wgsl\"\n";
    let plain_files = [("a.wgsl", pass), ("b.wgsl", pass), ("l.wgsl", pass)];
    let made = |manifest, files: &[(&str, &str)]| {
        let frame = Chain::frame(&device, &queue);
        let before = count()?;
        let post = Chain::post(&device, &frame, manifest, files);
        let built = count()?;
        let mut chain = Chain::new(&device, frame, post);
        chain.post.kept_frame([1, 1], [4, 4]);
        chain.run(&device, &queue, [4, 4]);
        let first = count()?;
        chain.post.kept_frame([1, 1], [4, 4]);
        chain.run(&device, &queue, [4, 4]);
        let second = count()?;
        let delta = |a: [usize; 3], b: [usize; 3]| [0, 1, 2].map(|i| b[i] - a[i]);
        Some([
            delta(before, built),
            delta(built, first),
            delta(first, second),
        ])
    };
    // Without a report (a backend outside wgpu-core) there is nothing to count.
    let Some(plain) = made(plain, &plain_files) else {
        return;
    };
    assert_eq!(plain, [[1, 0, 0], [0, 4, 0], [0, 0, 0]]);
    let kept = made(KEPT_CHAIN, &KEPT_FILES).unwrap();
    assert_eq!(kept, [[4, 0, 0], [0, 8, 0], [0, 0, 0]]);
    let compute = "[[post]]\nfile = \"c.wgsl\"\ncompute = true\ndispatch = [1, 1, 1]\n\
                   workgroup = [1, 1, 1]\n[[post]]\nfile = \"l.wgsl\"\n";
    let files = [
        ("c.wgsl", "fn compute(input: ComputeInput) {}\n"),
        ("l.wgsl", pass),
    ];
    // Without `[[resource]]` the compute pass has nothing to write: nothing is made for it.
    let idle = made(compute, &files).unwrap();
    assert_eq!(idle, [[1, 0, 0], [0, 2, 0], [0, 0, 0]]);
    let (manifest, files) = SUMMING;
    let computing = made(manifest, &files).unwrap();
    assert_eq!(computing, [[3, 0, 1], [0, 6, 0], [0, 0, 0]]);
    // A `[[before_world]]` pass: its layout and pipeline while the pack builds, its groups (one a
    // parity) on the first frame besides the chain's.
    let (manifest, files) = BEFORE_SUMMING;
    let before = made(manifest, &files).unwrap();
    assert_eq!(before, [[3, 0, 1], [0, 6, 0], [0, 0, 0]]);
}

/// #16011 item 4: a pack's kept resources are made at install (`replace_executor`), not while it
/// builds, so one that fails to build makes none; a reload destroys the old store before it
/// makes the new one, and a carried store stays; a pack whose kept formats the device can't make
/// is refused while it builds, and the running store stays. Counted with
/// `Instance::generate_report`.
#[test]
fn a_reload_never_holds_two_stores() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let Ok((device, _)) = pollster::block_on(adapter.request_device(&Default::default())) else {
        return;
    };
    // Live buffers and textures.
    let count = || {
        let hub = instance.generate_report()?.hub;
        Some([hub.buffers.num_allocated, hub.textures.num_allocated])
    };
    if count().is_none() {
        return;
    }
    let frame =
        super::super::NativeFrame::new(&device, None, Vec::new(), Default::default(), None, false);
    let build = |resources: &str, colour: &str| {
        let pass = format!("fn post(input: PostInput) -> vec4<f32> {{ return {colour}; }}\n");
        let report = report(
            &format!("{resources}[[post]]\nfile = \"a.wgsl\"\n"),
            &[("a.wgsl", &pass)],
        );
        let stages = (report.stages.into_iter())
            .filter(|s| s.kind == StageKind::Post)
            .collect();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        super::super::NativePost::new(&device, stages, &report.manifest, &[], format, &frame, None)
    };
    let post = |resources: &str, colour: &str| build(resources, colour).unwrap();
    let delta = |from: [usize; 2]| count().map(|c| [c[0] - from[0], c[1] - from[1]]);
    // Building one makes what building one without resources makes: nothing kept.
    let before = count().unwrap();
    let bare = post("", "input.color");
    let made = delta(before).unwrap();
    drop(bare);
    let before = count().unwrap();
    let mut first = post(BUFFERS, "input.color");
    assert_eq!(delta(before), Some(made));
    // Installed: the buffer and the texture's two versions.
    let built = count().unwrap();
    install(&device, Some(&mut first), None, [1, 1]).unwrap();
    assert_eq!(delta(built), Some([1, 2]));
    // A reload with another file: the first store goes, then the second is made.
    let mut second = post(BUFFERS, "input.color * 0.5");
    let built = count().unwrap();
    install(&device, Some(&mut second), Some(&mut first), [1, 1]).unwrap();
    assert_eq!(delta(built), Some([0, 0]));
    assert!(first.kept.as_ref().is_some_and(|k| k.held.is_empty()));
    // A carried store stays: nothing destroyed, nothing made.
    let mut third = post(BUFFERS, "input.color * 0.5");
    third.carry(&mut second);
    let built = count().unwrap();
    install(&device, Some(&mut third), Some(&mut second), [1, 1]).unwrap();
    assert_eq!(delta(built), Some([0, 0]));
    assert_eq!(third.kept.as_ref().map(|k| k.held.len()), Some(2));
    // A device that can't make `wide`'s r32float so (`PROBE_EXTRA` stands in for one): refused
    // in the build, its error caught there, nothing left made, the running store whole.
    PROBE_EXTRA.set(wgpu::TextureUsages::STORAGE_ATOMIC);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let built = count().unwrap();
    let refused = build(BUFFERS, "input.color").err().unwrap();
    PROBE_EXTRA.set(wgpu::TextureUsages::empty());
    assert!(
        refused.starts_with("wide: the device can't make an r32float texture to keep"),
        "{refused}"
    );
    assert_eq!(delta(built), Some([0, 0]));
    assert!(pollster::block_on(scope.pop()).is_none());
    assert_eq!(third.kept.as_ref().map(|k| k.held.len()), Some(2));
    // A device without the video memory for a pack's store (`NO_ROOM` stands in for one):
    // refused at install with what it asks for, nothing of it left made, and the running store,
    // freed first, made again (`NativePack::reload_kept`).
    let mut fourth = post(BUFFERS, "input.color * 0.25");
    let built = count().unwrap();
    NO_ROOM.set(1);
    let refused = install(&device, Some(&mut fourth), Some(&mut third), [4, 4]);
    let why = "its kept resources (1 MiB) don't fit in video memory (no room (test))";
    assert_eq!(refused, Err(why.into()));
    assert!(fourth.kept.as_ref().is_some_and(|k| k.held.is_empty()));
    let freed = count().map(|c| [built[0] - c[0], built[1] - c[1]]);
    assert_eq!(freed, Some([1, 2]));
    install(&device, Some(&mut third), None, [4, 4]).unwrap();
    assert_eq!(delta(built), Some([0, 0]));
}

/// The video memory the refusal names: buffers as declared, textures both versions, an
/// R-relative one at R.
#[test]
fn a_store_s_size_counts_both_versions_of_a_texture() {
    use horizon_shader_api::kept::Element;
    let buffer = Shape::Buffer {
        bytes: 1024,
        element: Element::U32,
    };
    let fixed = Shape::Fixed {
        format: Format::R32Float,
        size: [64, 32],
    };
    let scaled = Shape::Scaled {
        format: Format::Rgba32Float,
        scale: 1.0,
    };
    assert_eq!(bytes(buffer, [3840, 2160]), 1024);
    assert_eq!(bytes(fixed, [3840, 2160]), 2 * 64 * 32 * 4);
    assert_eq!(bytes(scaled, [3840, 2160]), 2 * 3840 * 2160 * 16);
}

/// A timed frame times each compute pass under its `name`, a `[[before_world]]` one and a
/// compute `[[post]]` one: before, neither had timestamps, so a profile couldn't see them. Run
/// with `WGPU_BACKEND=vulkan` and `WGPU_BACKEND=dx12` for each backend.
#[test]
fn a_timed_frame_times_each_compute_pass_under_its_name() {
    use crate::gfx::pack_timing::PassTiming;
    let _gpu = crate::gfx::test_gpu::lock();
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let wanted = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        eprintln!("no timestamps on {:?}: skipped", adapter.get_info().backend);
        return;
    }
    let desc = wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&desc)).unwrap();
    let manifest = "[[resource]]\nname = \"sum\"\nbuffer = 16\nelement = \"u32\"\nkeep = \"view\"\n\
                    [[before_world]]\nfile = \"w.wgsl\"\nname = \"World sum\"\n\
                    dispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\nwrites = [\"sum\"]\n\
                    [[post]]\nfile = \"c.wgsl\"\nname = \"Chain sum\"\ncompute = true\n\
                    dispatch = [1, 1, 1]\nworkgroup = [1, 1, 1]\nwrites = [\"sum\"]\n\
                    [[post]]\nfile = \"l.wgsl\"\nreads = [\"sum\"]\n";
    let files = [
        (
            "w.wgsl",
            "fn compute(input: ComputeInput) { sum[0] = sum[0] + 1u; }\n",
        ),
        (
            "c.wgsl",
            "fn compute(input: ComputeInput) { sum[1] = sum[1] + 1u; }\n",
        ),
        ("l.wgsl", SUMMING.1[1].1),
    ];
    let frame = Chain::frame(&device, &queue);
    let mut post = Chain::post(&device, &frame, manifest, &files);
    let screen = Screen::new(&device);
    let mut timing = PassTiming::new(&device, &queue).unwrap();
    for _ in 0..6 {
        timing.begin(&device, &queue);
        post.kept_frame([1, 1], [4, 4]);
        let mut encoder = device.create_command_encoder(&Default::default());
        post.before_world(&device, &mut encoder, None, Some(&mut timing));
        let world = (&screen.color, &screen.depth, &frame.surface_view);
        let view = screen.screen.create_view(&Default::default());
        post.run(
            &device,
            &mut encoder,
            world,
            &view,
            [4, 4],
            Some(&mut timing),
            None,
        );
        queue.submit([encoder.finish()]);
        timing.submitted(&queue);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    let names: Vec<_> = timing.medians().into_iter().map(|(n, _)| n).collect();
    for name in ["World sum", "Chain sum"] {
        assert!(names.iter().any(|n| n == name), "{name} not in {names:?}");
    }
}
