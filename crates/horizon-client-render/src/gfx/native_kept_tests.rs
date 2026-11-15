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
