//! A shader pack's kept resources (#16011 item 4): the layout entries a pass binds its
//! `[[resource]]`s with, and what a device lacks for them. Nothing makes or binds them yet; the
//! game still refuses such a pack.
use horizon_shader_api::check::CheckedStage;
use horizon_shader_api::kept::{Dispatch, Format, Kept, Shape};
use horizon_shader_api::stages::{KeptAccess, KeptBinding, kept_bindings};
use horizon_shader_api::{Manifest, StageKind};

fn format(format: Format) -> wgpu::TextureFormat {
    match format {
        Format::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        Format::R32Float => wgpu::TextureFormat::R32Float,
        Format::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        Format::Rgba32Uint => wgpu::TextureFormat::Rgba32Uint,
    }
}

/// The layout entries of `bindings` (`kept_bindings`) for `visibility`: a buffer read-only
/// where the pass doesn't write it, a written texture write-only storage, a read one filterable
/// only as `rgba16float` (the 32-bit ones are read with `textureLoad`), `rgba32uint` read as
/// `u32`.
pub(super) fn layout_entries(
    kept: &Kept,
    bindings: &[KeptBinding],
    visibility: wgpu::ShaderStages,
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let entry = |b: &KeptBinding| {
        let ty = match (kept.resources[b.resource].shape, b.access) {
            (Shape::Buffer { .. }, access) => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage {
                    read_only: access != KeptAccess::Write,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            (Shape::Fixed { format: f, .. } | Shape::Scaled { format: f, .. }, access) => {
                match access {
                    KeptAccess::Write => wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: format(f),
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    _ => wgpu::BindingType::Texture {
                        sample_type: match f {
                            Format::Rgba32Uint => wgpu::TextureSampleType::Uint,
                            f => wgpu::TextureSampleType::Float {
                                filterable: f == Format::Rgba16Float,
                            },
                        },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                }
            }
        };
        wgpu::BindGroupLayoutEntry {
            binding: b.binding,
            visibility,
            ty,
            count: None,
        }
    };
    bindings.iter().map(entry).collect()
}

/// What a device of `limits` lacks for the kept resources and compute passes of `manifest`, if
/// anything: `stages` are passes of its `[[post]]` and `[[before_world]]` lists, in order, each
/// binding the group-3 entries `shared` besides its kept ones. A device without compute shaders
/// has no compute limits (wgpu gives them as 0), so those say it.
pub(super) fn refusal(
    limits: &wgpu::Limits,
    manifest: &Manifest,
    stages: &[CheckedStage],
    shared: &[wgpu::BindGroupLayoutEntry],
) -> Option<String> {
    let computes = |s: &CheckedStage| matches!(s.kind, StageKind::Compute | StageKind::BeforeWorld);
    if stages.iter().any(computes) && limits.max_compute_invocations_per_workgroup == 0 {
        return Some("the device runs no compute shaders".into());
    }
    let kept = &manifest.kept;
    let largest = (limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
    let side = limits.max_texture_dimension_2d;
    for r in &kept.resources {
        if let Shape::Fixed { size: [w, h], .. } = r.shape
            && w.max(h) > side
        {
            return Some(format!(
                "{}: {w}×{h} texels, the device takes {side}",
                r.name
            ));
        }
        if let Shape::Buffer { bytes, .. } = r.shape
            && bytes > largest
        {
            return Some(format!(
                "{}: {bytes} bytes, the device takes {largest}",
                r.name
            ));
        }
    }
    // Each kind's checked passes are its manifest entries in order.
    let ordinary: Vec<_> = manifest.post.iter().filter(|s| !s.compute).collect();
    let compute: Vec<_> = manifest.post.iter().filter(|s| s.compute).collect();
    let before: Vec<_> = kept.before_world.iter().collect();
    let mut next = [0; 3];
    for stage in stages {
        let (i, list, visibility) = match stage.kind {
            StageKind::Post => (0, &ordinary, wgpu::ShaderStages::FRAGMENT),
            StageKind::Compute => (1, &compute, wgpu::ShaderStages::COMPUTE),
            StageKind::BeforeWorld => (2, &before, wgpu::ShaderStages::COMPUTE),
            _ => continue,
        };
        let Some(entry) = list.get(next[i]) else {
            continue;
        };
        next[i] += 1;
        let why = |why: String| Some(format!("{}: {why}", stage.file));
        let mut entries: Vec<_> = (shared.iter())
            .map(|e| wgpu::BindGroupLayoutEntry { visibility, ..*e })
            .collect();
        entries.extend(layout_entries(
            kept,
            &kept_bindings(manifest, entry),
            visibility,
        ));
        if let Some(over) = super::trace::over_limits(&entries, &[], limits, visibility) {
            return why(over);
        }
        let Some(workgroup) = entry.workgroup else {
            continue;
        };
        let most = [
            limits.max_compute_workgroup_size_x,
            limits.max_compute_workgroup_size_y,
            limits.max_compute_workgroup_size_z,
        ];
        let invocations = workgroup.iter().product::<u32>();
        if workgroup.iter().zip(most).any(|(w, m)| *w > m)
            || invocations > limits.max_compute_invocations_per_workgroup
        {
            return why(format!(
                "a workgroup of {workgroup:?}, the device takes {most:?} and {} invocations",
                limits.max_compute_invocations_per_workgroup
            ));
        }
        let shared_bytes = workgroup_bytes(stage);
        if shared_bytes > limits.max_compute_workgroup_storage_size {
            return why(format!(
                "{shared_bytes} bytes of var<workgroup>, the device takes {}",
                limits.max_compute_workgroup_storage_size
            ));
        }
        if let Some(Dispatch::Cells(cells)) = entry.dispatch {
            let groups = cells.iter().zip(workgroup).map(|(c, w)| c.div_ceil(w));
            let most = limits.max_compute_workgroups_per_dimension;
            if let Some(n) = groups.max().filter(|n| *n > most) {
                return why(format!(
                    "{n} workgroups along an axis, the device takes {most}"
                ));
            }
        }
    }
    None
}

/// The bytes of the `var<workgroup>`s a compute stage's entry point (its only one) uses, itself
/// or through what it calls, as the validator found: not those the module declares for others.
fn workgroup_bytes(stage: &CheckedStage) -> u32 {
    let (module, uses) = (&stage.module, stage.info.get_entry_point(0));
    let mut layouter = naga::proc::Layouter::default();
    if layouter.update(module.to_ctx()).is_err() {
        return u32::MAX;
    }
    (module.global_variables.iter())
        .filter(|(h, g)| g.space == naga::AddressSpace::WorkGroup && !uses[*h].is_empty())
        .map(|(_, g)| layouter[g.ty].size)
        .sum()
}

#[cfg(test)]
#[path = "native_kept_tests.rs"]
mod tests;
