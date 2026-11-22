//! Image formats: a storage image's format qualifier as wgpu's, which textures it binds, and the
//! format a module's storage images declare.

/// Makes each storage image of `module` in group 0 declare the format its layout entry binds,
/// the texture's when it differs from the image's qualifier (`binds_as`, `bindings`): wgpu
/// refuses a pipeline whose shader declares another storage format than its layout.
pub(super) fn storage_formats_of(
    module: &mut naga::Module,
    entries: &[wgpu::BindGroupLayoutEntry],
) {
    let globals: Vec<_> = module.global_variables.iter().map(|(h, _)| h).collect();
    for handle in globals {
        let var = &module.global_variables[handle];
        let Some(naga::ResourceBinding { group: 0, binding }) = var.binding else {
            continue;
        };
        let naga::TypeInner::Image {
            dim,
            arrayed,
            class: naga::ImageClass::Storage { format, access },
        } = module.types[var.ty].inner
        else {
            continue;
        };
        let bound = entries.iter().find_map(|e| match e.ty {
            wgpu::BindingType::StorageTexture { format, .. } if e.binding == binding => {
                Some(format)
            }
            _ => None,
        });
        let Some(to) = bound.and_then(storage_format).filter(|&to| to != format) else {
            continue;
        };
        // A new type: another image may share the old one.
        let ty = module.types.insert(
            naga::Type {
                name: module.types[var.ty].name.clone(),
                inner: naga::TypeInner::Image {
                    dim,
                    arrayed,
                    class: naga::ImageClass::Storage { format: to, access },
                },
            },
            naga::Span::UNDEFINED,
        );
        module.global_variables[handle].ty = ty;
    }
}

/// Whether a storage image declared with the qualifier of `declared` binds a texture of `actual`,
/// as `actual`. The same format does. So does a qualifier with the texture's channels and bits per
/// channel whose values are floats too (normalized or float), for a storage format: IterationRP's
/// bloom declares `layout(rgba16)` over colortex8, which the pack makes `RGBA16F`, and its passes
/// were refused, leaving the frame white (#14889). GLSL's image loads and stores read and write a
/// `vec4` for either, so the pass runs as written; GL leaves a qualifier that differs from the
/// image unit's format undefined. An integer qualifier over a float texture (or the reverse),
/// another channel count, or another width (`rgba16f` over an `RGBA8` texture, `rgb10_a2` over
/// `RGBA8`) stays refused. And an `R11F_G11F_B10F` image or buffer this device makes `RGBA16F` (a
/// float stand-in that neither clamps nor drops NaN) takes `layout(r11f_g11f_b10f)`: Iris 1.11.7
/// stored into and read back such an image with a probe pack of ours (#10490).
pub(super) fn binds_as(declared: wgpu::TextureFormat, actual: wgpu::TextureFormat) -> bool {
    use wgpu::TextureFormat as T;
    let float = |f: T| {
        matches!(
            f.sample_type(None, None),
            Some(wgpu::TextureSampleType::Float { .. })
        )
    };
    // Channels of different widths in one texel: the same texel size isn't the same channels.
    let packed = |f: T| matches!(f, T::Rgb10a2Unorm | T::Rg11b10Ufloat);
    declared == actual
        || (declared == T::Rg11b10Ufloat && actual == T::Rgba16Float)
        || (declared.components() == actual.components()
            && declared.block_copy_size(None) == actual.block_copy_size(None)
            && !packed(declared)
            && !packed(actual)
            && float(declared)
            && float(actual)
            && storage_format(actual).is_some())
}

/// naga's storage format for a texture format, `None` for one no storage image can have.
fn storage_format(format: wgpu::TextureFormat) -> Option<naga::StorageFormat> {
    use naga::StorageFormat as S;
    use wgpu::TextureFormat as T;
    Some(match format {
        T::R8Unorm => S::R8Unorm,
        T::R8Snorm => S::R8Snorm,
        T::R8Uint => S::R8Uint,
        T::R8Sint => S::R8Sint,
        T::R16Unorm => S::R16Unorm,
        T::R16Snorm => S::R16Snorm,
        T::R16Uint => S::R16Uint,
        T::R16Sint => S::R16Sint,
        T::R16Float => S::R16Float,
        T::Rg8Unorm => S::Rg8Unorm,
        T::Rg8Snorm => S::Rg8Snorm,
        T::Rg8Uint => S::Rg8Uint,
        T::Rg8Sint => S::Rg8Sint,
        T::R32Uint => S::R32Uint,
        T::R32Sint => S::R32Sint,
        T::R32Float => S::R32Float,
        T::Rg16Unorm => S::Rg16Unorm,
        T::Rg16Snorm => S::Rg16Snorm,
        T::Rg16Uint => S::Rg16Uint,
        T::Rg16Sint => S::Rg16Sint,
        T::Rg16Float => S::Rg16Float,
        T::Rgba8Unorm => S::Rgba8Unorm,
        T::Rgba8Snorm => S::Rgba8Snorm,
        T::Rgba8Uint => S::Rgba8Uint,
        T::Rgba8Sint => S::Rgba8Sint,
        T::Bgra8Unorm => S::Bgra8Unorm,
        T::Rgb10a2Uint => S::Rgb10a2Uint,
        T::Rgb10a2Unorm => S::Rgb10a2Unorm,
        T::Rg11b10Ufloat => S::Rg11b10Ufloat,
        T::R64Uint => S::R64Uint,
        T::Rg32Uint => S::Rg32Uint,
        T::Rg32Sint => S::Rg32Sint,
        T::Rg32Float => S::Rg32Float,
        T::Rgba16Unorm => S::Rgba16Unorm,
        T::Rgba16Snorm => S::Rgba16Snorm,
        T::Rgba16Uint => S::Rgba16Uint,
        T::Rgba16Sint => S::Rgba16Sint,
        T::Rgba16Float => S::Rgba16Float,
        T::Rgba32Uint => S::Rgba32Uint,
        T::Rgba32Sint => S::Rgba32Sint,
        T::Rgba32Float => S::Rgba32Float,
        _ => return None,
    })
}

/// The wgpu format of an image's format qualifier.
pub(super) fn image_format(qualifier: &str) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as T;
    Some(match qualifier {
        "rgba32f" => T::Rgba32Float,
        "rgba16f" => T::Rgba16Float,
        "rg32f" => T::Rg32Float,
        "rg16f" => T::Rg16Float,
        "r11f_g11f_b10f" => T::Rg11b10Ufloat,
        "r32f" => T::R32Float,
        "r16f" => T::R16Float,
        "rgba16" => T::Rgba16Unorm,
        "rgb10_a2" => T::Rgb10a2Unorm,
        "rgba8" => T::Rgba8Unorm,
        "rg16" => T::Rg16Unorm,
        "rg8" => T::Rg8Unorm,
        "r16" => T::R16Unorm,
        "r8" => T::R8Unorm,
        "rgba16_snorm" => T::Rgba16Snorm,
        "rgba8_snorm" => T::Rgba8Snorm,
        "rg16_snorm" => T::Rg16Snorm,
        "rg8_snorm" => T::Rg8Snorm,
        "r16_snorm" => T::R16Snorm,
        "r8_snorm" => T::R8Snorm,
        "rgba32i" => T::Rgba32Sint,
        "rgba16i" => T::Rgba16Sint,
        "rgba8i" => T::Rgba8Sint,
        "rg32i" => T::Rg32Sint,
        "rg16i" => T::Rg16Sint,
        "rg8i" => T::Rg8Sint,
        "r32i" => T::R32Sint,
        "r16i" => T::R16Sint,
        "r8i" => T::R8Sint,
        "rgba32ui" => T::Rgba32Uint,
        "rgba16ui" => T::Rgba16Uint,
        "rgb10_a2ui" => T::Rgb10a2Uint,
        "rgba8ui" => T::Rgba8Uint,
        "rg32ui" => T::Rg32Uint,
        "rg16ui" => T::Rg16Uint,
        "rg8ui" => T::Rg8Uint,
        "r32ui" => T::R32Uint,
        "r16ui" => T::R16Uint,
        "r8ui" => T::R8Uint,
        _ => return None,
    })
}
