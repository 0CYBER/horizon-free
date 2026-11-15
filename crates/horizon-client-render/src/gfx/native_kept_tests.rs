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
    assert_eq!(r32.format(), wgpu::TextureFormat::R32Float);
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
            let view;
            let resource = match held {
                Held::Buffer(b) => b.as_entire_binding(),
                Held::Pair([t, _]) => {
                    view = t.create_view(&Default::default());
                    wgpu::BindingResource::TextureView(&view)
                }
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
    let hud = crate::hud::HudBatch::new([4, 4]);
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

/// #16011 item 4: a pack's kept resources are made at install (`replace_native`), not while it
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
    install(&device, Some(&mut first), None);
    assert_eq!(delta(built), Some([1, 2]));
    // A reload with another file: the first store goes, then the second is made.
    let mut second = post(BUFFERS, "input.color * 0.5");
    let built = count().unwrap();
    install(&device, Some(&mut second), Some(&mut first));
    assert_eq!(delta(built), Some([0, 0]));
    assert!(first.kept.as_ref().is_some_and(|k| k.held.is_empty()));
    // A carried store stays: nothing destroyed, nothing made.
    let mut third = post(BUFFERS, "input.color * 0.5");
    third.carry(&mut second);
    let built = count().unwrap();
    install(&device, Some(&mut third), Some(&mut second));
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
}
