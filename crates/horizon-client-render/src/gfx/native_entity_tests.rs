use super::{EntityKind, EntityRuns, kind_bits, shader_source};
use crate::mesher::Vertex;

/// Vanilla's own as a pack writes it (docs/api/shaders-reference-0.2.md): the flash over the
/// surface in vanilla's gamma, then the light.
pub const VANILLA: &str = "
fn entity(input: EntityInput) -> Surface {
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    let flashed = pow(mix(pow(base, vec3(1.0 / 2.2)), pow(input.overlay.rgb, vec3(1.0 / 2.2)),
        input.overlay.a), vec3(2.2));
    let light = min(input.lightmap + input.colored_light, vec3(1.0));
    return Surface(vec4(flashed * light, input.albedo.a * input.tint.a));
}
";

/// A pack's entity shader whose stage is `entity`.
pub fn shader(entity: &str) -> String {
    use horizon_shader_api::{MANIFEST, Pack, check};
    let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\n\
        [entity]\nfile = \"entity.wgsl\"\n";
    let files = [(MANIFEST, manifest), ("entity.wgsl", entity)]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
    let report = check(&Pack::from_files(files).unwrap()).unwrap();
    shader_source(&report.stages[0].source)
}

/// Every name a pack's code could take that Horizon's entity shaders or the entries around
/// the stage have had, in any case: a pack's own are checked only against lowercase `horizon_`.
const OWN_NAMES: &str = "const globals: f32 = 1.0;
fn entity_color(c: f32) -> f32 { return c; }
struct Out { a: f32 }
struct HorizonEntityOut { a: f32 }
const HORIZON_ENTITY_KIND_SHIFT: u32 = 1u;
const HORIZON_ENTITY_BLOCK_ENTITY: u32 = 2u;
";

/// The module-level names of `module`: its types, constants, overrides, variables, functions
/// and entry points.
fn names(module: &naga::Module) -> std::collections::BTreeSet<String> {
    let types = module.types.iter().filter_map(|(_, t)| t.name.clone());
    let constants = module.constants.iter().filter_map(|(_, c)| c.name.clone());
    let overrides = module.overrides.iter().filter_map(|(_, o)| o.name.clone());
    let variables = module
        .global_variables
        .iter()
        .filter_map(|(_, v)| v.name.clone());
    let functions = module.functions.iter().filter_map(|(_, f)| f.name.clone());
    let entries = module.entry_points.iter().map(|e| e.name.clone());
    types
        .chain(constants)
        .chain(overrides)
        .chain(variables)
        .chain(functions)
        .chain(entries)
        .collect()
}

/// What the renderer adds to a pack's stage is all in the names a pack's files may not
/// declare (`horizon_`, lowercase: horizon-shader-api's check), so no pack that passes the
/// check can meet a duplicate declaration here.
#[test]
fn the_renderers_names_are_reserved() {
    let parse = |source: &str| naga::front::wgsl::parse_str(source).unwrap();
    let source = shader(VANILLA);
    let stage = names(&parse(
        &source[..source.find("// Horizon's entities").unwrap()],
    ));
    let added: Vec<_> = names(&parse(&source))
        .into_iter()
        .filter(|name| !stage.contains(name) && !name.starts_with("horizon_"))
        .collect();
    assert_eq!(added, [] as [&str; 0]);
}

/// The shader the renderer builds around a pack's stage is valid WGSL with both of its
/// pipelines' entries, whatever names the pack's own code takes: Horizon's are prefixed.
#[test]
fn the_packs_entity_shader_validates() {
    for entity in [VANILLA.to_owned(), OWN_NAMES.to_owned() + VANILLA] {
        let source = shader(&entity);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let entries: Vec<_> = module.entry_points.iter().map(|e| &*e.name).collect();
        for entry in [
            "horizon_entity_vs",
            "horizon_entity_vs_static",
            "horizon_entity_fs",
        ] {
            assert!(entries.contains(&entry), "{entry} in {entries:?}");
        }
    }
}

/// The kind is written above every bit the entity shaders read of a layer (the overlay, the
/// HDR tier, `OIT_ENTITY`), so a quad drawn by Horizon's own pipelines reads as before.
#[test]
fn a_kind_keeps_clear_of_the_layers_other_bits() {
    let kinds = [EntityKind::Entity, EntityKind::BlockEntity];
    for kind in kinds {
        let bits = kind_bits(kind);
        assert_ne!(bits, 0);
        assert_eq!(bits & (super::super::entity_batch::OIT_ENTITY * 2 - 1), 0);
        assert_eq!((bits >> super::KIND_SHIFT) & 7, kind as u32 + 1);
    }
}

fn quad(layer: u32) -> [Vertex; 4] {
    [Vertex {
        layer,
        ..bytemuck::Zeroable::zeroed()
    }; 4]
}

/// The kinds follow the game's runs through the sorted batch: an ordinary quad inside a run
/// takes its kind; one before, between or after the runs takes none, whatever was drawn
/// before it (an arrow after the last mob, a dragon after everything), and neither do the
/// other materials' quads, in a run or not (a translucent item in a mob's hand). The frame
/// after has none unless the game names its runs again.
#[test]
fn quads_take_the_kind_of_their_run() {
    // Quads 0, 1, 3, 4 and 5 are ordinary; quad 2 is another material's (overlay 32, a
    // translucent item's), sorted out of them though it lies in the entity's run.
    let vertices: Vec<Vertex> = [0, 5, 32, 0, 7, 0].into_iter().flat_map(quad).collect();
    let mut batch = super::super::entity_batch::EntityBatch::default();
    batch.kinds.push(4..12, EntityKind::Entity);
    batch.kinds.push(12..16, EntityKind::BlockEntity);
    batch.kinds.push(20..24, EntityKind::Entity);
    let kinds = |batch: &mut super::super::entity_batch::EntityBatch| -> Vec<u32> {
        batch.update(&vertices, &[], &[], &[], false);
        let quads = batch.combined.chunks(4);
        quads.map(|q| q[3].layer >> super::KIND_SHIFT & 7).collect()
    };
    // The ordinary quads lead the batch: 0, 1, 3, 4 and 5, then quad 2. Quad 4 follows the
    // block entity's run and is in none.
    assert_eq!(kinds(&mut batch), [0, 1, 2, 0, 1, 0]);
    assert_eq!(batch.combined[4].layer & 0xffff, 5);
    assert_eq!(batch.combined[20].layer, 32);
    assert_eq!(kinds(&mut batch), [0; 6]);
}

/// The ray scene's entity layer takes the quads the runs name, of the ordinary material and of
/// improved transparency's, wherever the batch sorts them, and nothing else: not a particle
/// drawn between two mobs, nor an additive quad, nor another material's quad in a run. Its two
/// ranges are where the batch put each material.
#[test]
fn the_entity_layer_s_quads_are_the_runs_through_the_sorting() {
    use super::super::entity_batch::{EntityBatch, OIT_ENTITY};
    use horizon_shader_api::stages::RAY_ENTITY_QUAD;
    // A mob (quads 0 and 1, one of improved transparency's), a particle (2), a translucent
    // item the mob holds (3), a block entity (4, improved transparency's), a particle (5).
    let layers = [0, OIT_ENTITY, 0, 32, OIT_ENTITY, 0];
    let vertices: Vec<Vertex> = layers.into_iter().flat_map(quad).collect();
    let additive: Vec<Vertex> = quad(2 | OIT_ENTITY).into();
    let mut batch = EntityBatch::default();
    batch.kinds.push(0..8, EntityKind::Entity);
    batch.kinds.push(12..20, EntityKind::Entity);
    batch.kinds.push(16..20, EntityKind::BlockEntity);
    batch.update(&vertices, &additive, &[], &[], true);
    // Sorted: the ordinary quads 0, 2, 5, the item 3, improved transparency's 1, 4 and the
    // additive quad it took.
    let marked: Vec<bool> = (batch.combined.chunks(4))
        .map(|q| q.iter().all(|v| v.layer & RAY_ENTITY_QUAD != 0))
        .collect();
    assert_eq!(marked, [true, false, false, false, true, true, false]);
    assert_eq!(batch.combined[12].layer & 0xffff, 32);
    assert_eq!(batch.ray_ranges, [(0, 3), (4, 3)]);
    // The bit keeps clear of the kind and of every bit the entity shaders read.
    let kind = batch.combined[20].layer >> super::KIND_SHIFT & 7;
    assert_eq!(kind, EntityKind::BlockEntity as u32 + 1);
    assert_eq!(
        RAY_ENTITY_QUAD & (7 << super::KIND_SHIFT | (OIT_ENTITY * 2 - 1)),
        0
    );
    // Without runs nothing is marked.
    batch.update(&vertices, &additive, &[], &[], true);
    assert!(
        batch
            .combined
            .iter()
            .all(|v| v.layer & RAY_ENTITY_QUAD == 0)
    );
}

/// A run has its end, and one named where geometry was cut back replaces what was named
/// there: no vertex is ever in a run its caller didn't name it in.
#[test]
fn runs_end_and_give_way_where_the_geometry_was_cut_back() {
    let bits = |runs: &EntityRuns| -> Vec<u32> {
        let quads = runs.bits((0..6).map(|quad| quad * 4));
        quads.map(|bits| bits >> super::KIND_SHIFT).collect()
    };
    let mut runs = EntityRuns::default();
    // Nothing drawn names nothing.
    runs.push(0..0, EntityKind::Entity);
    assert_eq!(runs, EntityRuns::default());
    runs.push(4..8, EntityKind::Entity);
    runs.push(8..12, EntityKind::Entity);
    runs.push(12..20, EntityKind::BlockEntity);
    assert_eq!(bits(&runs), [0, 1, 1, 2, 2, 0]);
    // The geometry was cut back to vertex 8 and a block entity drawn there: the entity's
    // run ends at it, and the old run from 12 is gone with its vertices.
    runs.push(8..12, EntityKind::BlockEntity);
    assert_eq!(bits(&runs), [0, 1, 2, 0, 0, 0]);
    // Cut back to before every run, with nothing named after.
    runs.push(2..2, EntityKind::Entity);
    assert_eq!(bits(&runs), [0; 6]);
}
