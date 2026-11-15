//! Section geometry native mods register (`add_section_geometry`, NeoForge's
//! `AddSectionGeometryEvent`): each renderer is asked on the meshing workers every time a section
//! meshes, and its quads go into that section's own mesh, so they follow its rebuilds and unload
//! with it.
use super::{Mesher, SectionMesh, TerrainSource, TerrainVertex, Tints, mod_tint::Region};
use crate::connected::TileLayer;
use mod_api::{BlockAndTintGetter, SectionGeometry, SectionGeometryContext, SectionQuad};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// One mod renderer, and whether it has panicked (then it is asked no more) or had its quads
/// refused (logged once).
pub(super) struct Renderer {
    renderer: Arc<dyn SectionGeometry>,
    failed: AtomicBool,
    refused: AtomicBool,
}

impl Renderer {
    pub(super) fn new(renderer: Arc<dyn SectionGeometry>) -> Self {
        Self {
            renderer,
            failed: AtomicBool::new(false),
            refused: AtomicBool::new(false),
        }
    }
    /// Whether it panicked, so that it is asked no more.
    #[cfg(test)]
    pub(super) fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// The renderers, in registration order: set once at start, read by every table build.
static REGISTERED: OnceLock<Arc<[Renderer]>> = OnceLock::new();

/// Sets the native mods' section geometry renderers, once, before the resources first load; each
/// later [`super::MeshTables::new`] hands them to the meshing workers. False when already set.
pub fn register_section_geometry(renderers: Vec<Arc<dyn SectionGeometry>>) -> bool {
    REGISTERED
        .set(renderers.into_iter().map(Renderer::new).collect())
        .is_ok()
}

/// Whether any mod renderer is registered, so the game prepares the block values its `level`
/// reads.
pub fn has_section_geometry() -> bool {
    REGISTERED.get().is_some_and(|r| !r.is_empty())
}

/// What [`register_section_geometry`] set; none before.
pub(super) fn registered() -> Arc<[Renderer]> {
    REGISTERED.get().cloned().unwrap_or_else(|| Arc::new([]))
}

/// The most quads one section's mesh may hold, its own blocks' and every renderer's together
/// (the section-meshing design's per-section ceiling).
pub(super) const MAX_QUADS: usize = 262_144;
/// The most mesh data one section may hold, in bytes (the design's encoded section data ceiling).
pub(super) const MAX_SECTION_BYTES: usize = 32 << 20;
/// The most added quads all meshing workers may hold at once before they are in their meshes, in
/// the mesh bytes they will take (the design's in-flight ceiling).
pub(super) const MAX_IN_FLIGHT_BYTES: usize = 128 << 20;
/// The mesh bytes one added quad takes: its four vertices and its tint.
pub(super) const QUAD_BYTES: usize = 4 * size_of::<TerrainVertex>() + size_of::<u32>();

/// The mesh bytes of added quads the meshing workers hold now, out of [`MAX_IN_FLIGHT_BYTES`].
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// What is left of one section's budget, shared by its own blocks and every renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Budget {
    pub(super) quads: usize,
    pub(super) bytes: usize,
}

impl Budget {
    /// What a section whose mesh is `out` so far has left.
    pub(super) fn left_after(out: &SectionMesh) -> Self {
        let vertices = out.solid.len() + out.translucent.len();
        let sources = out.solid_sources.len() + out.translucent_sources.len();
        let bytes = vertices * size_of::<TerrainVertex>()
            + out.solid_tints.len() * size_of::<u32>()
            + sources * size_of::<TerrainSource>();
        Self {
            quads: MAX_QUADS.saturating_sub(vertices / 4),
            bytes: MAX_SECTION_BYTES.saturating_sub(bytes),
        }
    }

    /// How many more added quads fit.
    fn quads_left(self) -> usize {
        self.quads.min(self.bytes / QUAD_BYTES)
    }
}

/// A quad that passed its checks, waiting for its renderer to finish.
pub(super) struct Staged {
    pub(super) quad: SectionQuad,
    pub(super) layer: u16,
}

/// One renderer's quads for one section, behind the [`SectionGeometryContext`] it is handed.
/// Every quad is checked,
/// and counted against the section's shared budget and the workers' in-flight one, before it is
/// kept; one that fails refuses the whole batch, which gives back all it held.
pub(super) struct Batch<'a> {
    sprites: &'a BTreeMap<String, u32>,
    budget: &'a mut Budget,
    pub(super) staged: Vec<Staged>,
    /// The workers' shared in-flight bytes: [`IN_FLIGHT`] outside tests.
    shared: &'a AtomicUsize,
    /// The bytes this batch holds in `shared`, given back when it drops.
    pub(super) in_flight: usize,
    pub(super) refused: Option<String>,
}

impl<'a> Batch<'a> {
    pub(super) fn new(
        sprites: &'a BTreeMap<String, u32>,
        budget: &'a mut Budget,
        shared: &'a AtomicUsize,
    ) -> Self {
        Self {
            sprites,
            budget,
            staged: Vec::new(),
            shared,
            in_flight: 0,
            refused: None,
        }
    }

    /// `quad`'s atlas layer, or why it refuses the batch.
    fn check(&self, sprite: &str, quad: &SectionQuad) -> Result<u16, String> {
        // Inside its own section, so its bounds, culling and unload stay that section's.
        let corner = |c: &f32| c.is_finite() && (0.0..=16.0).contains(c);
        let uv = |c: &f32| c.is_finite() && (0.0..=1.0).contains(c);
        if !quad.positions.iter().flatten().all(corner) {
            return Err(format!(
                "a corner outside the section: {:?}",
                quad.positions
            ));
        }
        if !quad.uvs.iter().flatten().all(uv) {
            return Err(format!("a UV outside 0 to 1: {:?}", quad.uvs));
        }
        let Some(&layer) = self.sprites.get(sprite) else {
            // Bound only the diagnostic, not accepted atlas identifiers. A character
            // boundary keeps arbitrary Unicode sprite names safe to report.
            let end = sprite.floor_char_boundary(256);
            let suffix = if end < sprite.len() { "…" } else { "" };
            return Err(format!(
                "sprite {}{suffix}, which the block atlas lacks",
                &sprite[..end]
            ));
        };
        if self.budget.quads_left() == 0 {
            return Err(format!(
                "past the section's {MAX_QUADS} quads or {MAX_SECTION_BYTES} bytes"
            ));
        }
        Ok(layer as u16)
    }

    /// Takes [`QUAD_BYTES`] of the shared in-flight bytes, unless that would pass
    /// [`MAX_IN_FLIGHT_BYTES`].
    fn reserve(&mut self) -> bool {
        let taken = self
            .shared
            .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                held.checked_add(QUAD_BYTES)
                    .filter(|&after| after <= MAX_IN_FLIGHT_BYTES)
            });
        if taken.is_ok() {
            self.in_flight += QUAD_BYTES;
        }
        taken.is_ok()
    }

    /// Refuses the batch: drops every quad it held and gives their budget back.
    pub(super) fn refuse(&mut self, why: String) {
        self.budget.quads += self.staged.len();
        self.budget.bytes += self.staged.len() * QUAD_BYTES;
        self.staged = Vec::new();
        self.release();
        self.refused = Some(why);
    }

    fn release(&mut self) {
        self.shared
            .fetch_sub(std::mem::take(&mut self.in_flight), Ordering::AcqRel);
    }
}

impl Batch<'_> {
    /// Keeps `quad`, drawn with atlas sprite `sprite`, unless it refuses the batch.
    pub(super) fn add(&mut self, sprite: &str, quad: SectionQuad) -> bool {
        if self.refused.is_some() {
            return false;
        }
        let layer = match self.check(sprite, &quad) {
            Ok(layer) => layer,
            Err(why) => {
                self.refuse(why);
                return false;
            }
        };
        if !self.reserve() {
            self.refuse(format!(
                "the meshing workers already hold {MAX_IN_FLIGHT_BYTES} bytes of added quads"
            ));
            return false;
        }
        self.budget.quads -= 1;
        self.budget.bytes -= QUAD_BYTES;
        self.staged.push(Staged { quad, layer });
        true
    }

    /// How many more quads the batch takes.
    pub(super) fn remaining(&self) -> u32 {
        if self.refused.is_some() {
            return 0;
        }
        u32::try_from(self.budget.quads_left()).unwrap_or(u32::MAX)
    }
}

/// What a renderer is handed for one section: the section's level and its batch.
struct Context<'a> {
    level: &'a dyn BlockAndTintGetter,
    batch: Batch<'a>,
}

impl SectionGeometryContext for Context<'_> {
    fn level(&self) -> &dyn BlockAndTintGetter {
        self.level
    }

    fn add(&mut self, sprite: &str, quad: SectionQuad) -> bool {
        self.batch.add(sprite, quad)
    }

    fn remaining(&self) -> u32 {
        self.batch.remaining()
    }
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

/// Asks every renderer to add the quads of the section `tints` is set up for, and adds them to
/// `out` after the section's own blocks. A renderer's quads are taken whole or refused whole.
pub(super) fn add(mesher: &Mesher<'_>, tints: &mut Tints<'_>, out: &mut SectionMesh) {
    let [cx, cz, base_y] = tints.origin;
    let origin = [cx * 16, base_y, cz * 16];
    let region = Region {
        tables: mesher.tables,
        tints: RefCell::new(tints),
        nether: mesher.nether_shading,
    };
    let mut budget = Budget::left_after(out);
    for renderer in mesher.tables.section_geometry.iter() {
        if renderer.failed.load(Ordering::Relaxed) {
            continue;
        }
        let mut context = Context {
            level: &region,
            batch: Batch::new(&mesher.render.sprite_layers, &mut budget, &IN_FLIGHT),
        };
        let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.renderer.add(origin, &mut context)
        }));
        let mut batch = context.batch;
        if asked.is_err() {
            batch.refuse(String::new());
            if !renderer.failed.swap(true, Ordering::Relaxed) {
                log::warn!("Horizon: a section geometry renderer panicked; it is asked no more");
            }
            continue;
        }
        match &batch.refused {
            // The batch keeps its in-flight bytes until its quads are in the mesh.
            None => {
                for staged in &batch.staged {
                    emit(mesher, &staged.quad, staged.layer, out);
                }
            }
            Some(why) => {
                if !renderer.refused.swap(true, Ordering::Relaxed) {
                    log::warn!(
                        "Horizon: section geometry for the section at {origin:?} was refused: {why}"
                    );
                }
            }
        }
    }
}

/// `quad` as four section vertices in its pass, drawn with atlas `layer`.
fn emit(mesher: &Mesher<'_>, quad: &SectionQuad, layer: u16, out: &mut SectionMesh) {
    let [a, r, g, b] = quad.color.to_be_bytes().map(u32::from);
    let color = r | g << 8 | b << 16 | a << 24;
    let [sky, block] = quad.light.map(|l| u16::from(l.min(15)) * 16);
    let light = sky | block << 8;
    let vertices = std::array::from_fn(|i| {
        TerrainVertex::new(quad.positions[i], quad.uvs[i], 0, color, light)
    });
    let tile = TileLayer::Layer {
        layer,
        translucent: quad.translucent,
        opaque: false,
    };
    mesher.quad_as(
        out,
        false,
        tile,
        vertices,
        false,
        Some(quad.color & 0xFF_FFFF),
    );
}

#[cfg(test)]
#[path = "mod_geometry_diagnostic_tests.rs"]
mod diagnostic_tests;
