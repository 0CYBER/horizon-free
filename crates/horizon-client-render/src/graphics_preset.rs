//! The graphics presets: each one a fixed table of the options it sets, vanilla's and
//! Horizon's own, so that picking one gives the same settings whatever was set before.
//! Every preset turns on the optimizations that leave the picture as it is; Full
//! Performance adds the ones that change it. What no table names keeps the player's
//! choice: the display preferences (`graphics_ui`'s `touched`), Far Detail and Leaf
//! Culling, which follow the machine and the look the player picked, and Translucent
//! Batching, on unless the player turned it off.
use super::{EntityCullingMode, GpuCullingMode, ParticleCullingMode, Preset, Upscaling};
use crate::settings::Settings;

/// What differs between the presets. Every one of them also turns on entity and particle
/// culling on a worker, the block-entity caches and GPU Culling's Auto (which tries it only
/// where it makes frames faster), keeps cave culling, the sky, the fog, the weather and the
/// animated textures on, uploads meshes unthrottled and draws at a fixed resolution without
/// TAA.
struct Table {
    render_distance: i32,
    simulation_distance: i32,
    /// `options.txt`'s `renderClouds`: `false`, `fast` or `true`.
    clouds: &'static str,
    /// 0 all, 1 decreased, 2 minimal.
    particles: u8,
    entity_shadows: bool,
    biome_blend: i32,
    mipmap_levels: i32,
    entity_distance: f32,
    cutout_leaves: bool,
    weather_radius: i32,
    /// Far terrain's range in chunks; 0 is off.
    lod_distance: u32,
    opaque_ice: bool,
    resolution_scale: u32,
    fxaa: bool,
    dither_cutout_mips: bool,
}

fn table(preset: Preset) -> Option<Table> {
    Some(match preset {
        Preset::FullPerformance => Table {
            render_distance: 6,
            simulation_distance: 5,
            clouds: "false",
            particles: 2,
            entity_shadows: false,
            biome_blend: 0,
            mipmap_levels: 0,
            entity_distance: 0.5,
            cutout_leaves: false,
            weather_radius: 3,
            lod_distance: 0,
            opaque_ice: true,
            resolution_scale: 75,
            fxaa: false,
            dither_cutout_mips: false,
        },
        Preset::Performance => Table {
            render_distance: 8,
            simulation_distance: 6,
            clouds: "false",
            particles: 2,
            entity_shadows: false,
            biome_blend: 0,
            mipmap_levels: 0,
            entity_distance: 0.5,
            cutout_leaves: false,
            weather_radius: 3,
            lod_distance: 0,
            opaque_ice: false,
            resolution_scale: 100,
            fxaa: false,
            dither_cutout_mips: false,
        },
        Preset::Balanced => Table {
            render_distance: 12,
            simulation_distance: 8,
            clouds: "fast",
            particles: 1,
            entity_shadows: true,
            biome_blend: 1,
            mipmap_levels: 2,
            entity_distance: 0.75,
            cutout_leaves: true,
            weather_radius: 5,
            lod_distance: 0,
            opaque_ice: false,
            resolution_scale: 100,
            fxaa: false,
            dither_cutout_mips: false,
        },
        Preset::Quality => Table {
            render_distance: 16,
            simulation_distance: 12,
            clouds: "true",
            particles: 0,
            entity_shadows: true,
            biome_blend: 2,
            mipmap_levels: 4,
            entity_distance: 1.0,
            cutout_leaves: true,
            weather_radius: 10,
            lod_distance: 64,
            opaque_ice: false,
            resolution_scale: 100,
            fxaa: true,
            dither_cutout_mips: false,
        },
        Preset::Ultra => Table {
            render_distance: 24,
            simulation_distance: 12,
            clouds: "true",
            particles: 0,
            entity_shadows: true,
            biome_blend: 3,
            mipmap_levels: 4,
            entity_distance: 1.5,
            cutout_leaves: true,
            weather_radius: 10,
            lod_distance: 128,
            opaque_ice: false,
            resolution_scale: 100,
            fxaa: true,
            dither_cutout_mips: true,
        },
        Preset::Custom => return None,
    })
}

/// Sets what `preset`'s table names, in vanilla's options and Horizon's own.
pub fn apply_preset(preset: Preset, s: &mut Settings) {
    let Some(t) = table(preset) else { return };
    s.render_distance = t.render_distance;
    s.simulation_distance = t.simulation_distance;
    s.clouds = t.clouds.into();
    s.particles = t.particles;
    s.entity_shadows = t.entity_shadows;
    s.biome_blend = t.biome_blend;
    s.mipmap_levels = t.mipmap_levels;
    s.entity_distance = t.entity_distance;
    s.cutout_leaves = t.cutout_leaves;
    s.weather_radius = t.weather_radius;
    s.graphics_preset = "custom".into();
    let g = &mut s.horizon;
    g.preset = preset;
    g.lod_distance = t.lod_distance;
    g.cave_culling = true;
    g.gpu_culling = GpuCullingMode::Auto;
    g.entity_culling = EntityCullingMode::Asynchronous;
    g.particle_culling = ParticleCullingMode::Asynchronous;
    g.opaque_ice = t.opaque_ice;
    g.cache_shulker_boxes = true;
    g.cache_signs = true;
    g.cache_decorated_pots = true;
    g.cache_bells = true;
    g.cache_chests = true;
    g.cache_banners = true;
    g.cache_copper_golem_statues = true;
    g.chunk_updates = 0;
    g.upload_budget_kib = 0;
    g.sky = true;
    g.fog = true;
    g.weather = true;
    g.animated_textures = true;
    g.dither_cutout_mips = t.dither_cutout_mips;
    g.resolution_scale = t.resolution_scale;
    g.dynamic_resolution = false;
    g.fxaa = t.fxaa;
    g.taa = false;
    g.upscaling = Upscaling::Native;
}
