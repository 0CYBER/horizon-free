//! Fixed-capacity output for a view's custom geometry; the GPU consumer owns drawing.
use crate::gfx::mod_pipelines::{self, PipelineRun, PipelineVertex};
use crate::gfx::world_primitives::{Batch, DEFAULT_CAPACITY as CAPACITY, STAGES};
use crate::{gizmos, mesher::Vertex};
use glam::{DVec3, Mat4, Vec3, Vec4};
use mod_api::{
    DrawError, InstanceError, ModelInstance, PipelineDrawError, PipelineQuad, RenderLevelStage,
    WorldDepth, WorldDrawUsage, WorldLine, WorldQuad,
};

/// Copies of resolved models accepted for one view. Reserved once, never grown in a frame.
pub(crate) const INSTANCE_CAPACITY: u32 = 16_384;

/// One accepted copy, already checked. `light` and `overlay` are the renderer's vanilla packing.
#[derive(Clone, Copy)]
pub struct ModelCopy {
    pub slot: u32,
    pub generation: u64,
    pub origin: [f64; 3],
    pub transform: [f32; 12],
    pub tint: u32,
    pub light: u32,
    pub overlay: u32,
}

/// The model a batch draws. Resolved once, then applied to every copy.
#[derive(Clone, Copy)]
pub struct ModelBatch {
    pub slot: u32,
    pub generation: u64,
    pub translucent: bool,
    pub bounds: [[f32; 3]; 2],
}

/// Reused between views, including frames with no subscribers or output.
pub struct WorldGeometry {
    camera: [f64; 3],
    view: Mat4,
    white: [f32; 2],
    lines: [Vec<[f32; 8]>; 2],
    quads: [Vec<Vertex>; 2],
    /// Quads drawn by mod pipelines, 4 vertices each, and the runs that share a pipeline and
    /// a stage. They count against the quad capacity.
    pipeline_quads: Vec<PipelineVertex>,
    pipeline_runs: Vec<PipelineRun>,
    /// `PipelineFrame.time` for this view.
    time: f32,
    copies: Vec<ModelCopy>,
    groups: std::collections::HashSet<mod_api::InstanceGroupRef>,
    accepted_models: u32,
    model_cut: u32,
    /// The stage lines and quads go to now (`RenderLevelStage::ALL`'s index).
    stage: usize,
    /// Where each stage before `stage` ended (`Batch::stage_ends`).
    stage_ends: [[u32; 4]; STAGES - 1],
}
impl Default for WorldGeometry {
    fn default() -> Self {
        Self {
            camera: [0.0; 3],
            view: Mat4::IDENTITY,
            white: [0.0; 2],
            lines: std::array::from_fn(|_| Vec::with_capacity(CAPACITY as usize)),
            quads: std::array::from_fn(|_| Vec::with_capacity(CAPACITY as usize * 4)),
            pipeline_quads: Vec::with_capacity(CAPACITY as usize * 4),
            pipeline_runs: Vec::with_capacity(CAPACITY as usize),
            time: 0.0,
            copies: Vec::with_capacity(INSTANCE_CAPACITY as usize),
            groups: std::collections::HashSet::with_capacity(INSTANCE_CAPACITY as usize),
            accepted_models: 0,
            model_cut: 0,
            stage: 0,
            stage_ends: [[0; 4]; STAGES - 1],
        }
    }
}
impl WorldGeometry {
    /// Start a view even if nothing will be submitted; previous output disappears.
    pub fn reset(&mut self, camera: [f64; 3], view: Mat4, white: [f32; 2]) {
        self.camera = camera;
        self.view = view;
        self.white = white;
        self.lines.iter_mut().for_each(Vec::clear);
        self.quads.iter_mut().for_each(Vec::clear);
        self.pipeline_quads.clear();
        self.pipeline_runs.clear();
        self.clear_copies();
        self.model_cut = 0;
        self.groups.clear();
        self.stage = 0;
        self.stage_ends = [[0; 4]; STAGES - 1];
    }
    /// From now on, lines and quads draw at `stage`: the stages before it end here. Stages
    /// come in frame order, so a stage before the current one is refused and keeps the current.
    pub fn begin_stage(&mut self, stage: RenderLevelStage) {
        let index = RenderLevelStage::ALL
            .iter()
            .position(|s| *s == stage)
            .unwrap_or(STAGES - 1);
        let counts = self.counts();
        for ended in self.stage..index.min(STAGES - 1) {
            self.stage_ends[ended] = counts;
        }
        self.stage = self.stage.max(index);
    }
    /// The game time this view's pipeline shaders read, in seconds (`PipelineFrame.time`).
    pub fn set_time(&mut self, seconds: f32) {
        self.time = seconds;
    }
    /// Entity-stream cut used by directly staged instances in this view.
    pub fn set_model_cut(&mut self, cut: u32) {
        self.model_cut = cut;
    }
    pub fn model_cut(&self) -> u32 {
        self.model_cut
    }
    pub fn camera(&self) -> [f64; 3] {
        self.camera
    }
    /// The current counts and setup-time limits across both depth modes.
    pub fn usage(&self) -> WorldDrawUsage {
        WorldDrawUsage::new(
            self.lines.iter().map(|v| v.len() as u32).sum(),
            CAPACITY,
            self.quads.iter().map(|v| v.len() as u32 / 4).sum::<u32>()
                + self.pipeline_quads.len() as u32 / 4,
            CAPACITY,
            self.accepted_models,
            INSTANCE_CAPACITY,
        )
    }
    /// Accept every visible copy, or leave the output unchanged.
    ///
    /// Culled copies are omitted and do not count. A fault in any input, including one
    /// that would have been culled, refuses the whole batch. `stage` returns true
    /// for copies it consumes directly; its caller rolls back that output on error.
    pub fn submit_model(
        &mut self,
        batch: ModelBatch,
        instances: &[ModelInstance],
        cull: Option<&impl crate::gfx::Cull>,
        stage: impl FnMut(&ModelCopy) -> bool,
    ) -> Result<(), InstanceError> {
        if instances.len() > INSTANCE_CAPACITY as usize {
            return Err(InstanceError::CapacityExceeded);
        }
        validate_instances(instances)?;
        let mut stage = stage;
        self.submit_validated_model(batch, instances, cull, |_, copy| stage(copy))
    }
    /// Submit copies validated when written into persistent storage.
    pub fn submit_validated_model(
        &mut self,
        batch: ModelBatch,
        instances: &[ModelInstance],
        cull: Option<&impl crate::gfx::Cull>,
        mut stage: impl FnMut(usize, &ModelCopy) -> bool,
    ) -> Result<(), InstanceError> {
        if instances.is_empty() {
            return Ok(());
        }
        if instances.len() > INSTANCE_CAPACITY as usize {
            return Err(InstanceError::CapacityExceeded);
        }
        if batch.translucent {
            return Err(InstanceError::Translucent);
        }
        let start = self.copies.len();
        let accepted = self.accepted_models;
        for (index, instance) in instances.iter().enumerate() {
            if !copy_visible(
                batch.bounds,
                instance.transform,
                instance.origin,
                self.camera,
                cull,
            ) {
                continue;
            }
            // Validation above has already checked even the culled inputs. Roll back
            // only this submission on overflow; cull each copy just once.
            if self.accepted_models == INSTANCE_CAPACITY {
                self.copies.truncate(start);
                self.accepted_models = accepted;
                return Err(InstanceError::CapacityExceeded);
            }
            let copy = ModelCopy {
                slot: batch.slot,
                generation: batch.generation,
                origin: instance.origin,
                transform: instance.transform,
                tint: instance.tint,
                light: pack_light(instance.light),
                overlay: pack_overlay(instance.overlay),
            };
            if !stage(index, &copy) {
                self.copies.push(copy);
            }
            self.accepted_models += 1;
        }
        Ok(())
    }
    pub fn group_room(&self) -> bool {
        self.groups.len() < INSTANCE_CAPACITY as usize
    }
    pub fn has_group(&self, group: mod_api::InstanceGroupRef) -> bool {
        self.groups.contains(&group)
    }
    pub fn mark_group(&mut self, group: mod_api::InstanceGroupRef) {
        self.groups.insert(group);
    }
    pub fn copy_count(&self) -> usize {
        self.copies.len()
    }
    pub fn copy(&self, index: usize) -> ModelCopy {
        self.copies[index]
    }
    pub fn clear_copies(&mut self) {
        self.copies.clear();
        self.accepted_models = 0;
    }
    /// Validate once at the submission boundary, before any output is appended.
    fn validate_positions(&self, points: impl Iterator<Item = [f64; 3]>) -> Result<(), DrawError> {
        if points
            .flatten()
            .enumerate()
            .any(|(i, p)| !p.is_finite() || !((p - self.camera[i % 3]) as f32).is_finite())
        {
            return Err(DrawError::NonFinite);
        }
        Ok(())
    }
    /// Append an entire line batch or leave the output unchanged.
    pub fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        if lines.len() > (CAPACITY - self.usage().lines) as usize {
            return Err(DrawError::CapacityExceeded);
        }
        self.validate_positions(lines.iter().flat_map(|l| [l.from, l.to]))?;
        for l in lines {
            if !l.width.is_finite() {
                return Err(DrawError::NonFinite);
            }
            if l.width <= 0.0 {
                return Err(DrawError::InvalidWidth);
            }
        }
        for l in lines {
            let line = gizmos::Line {
                start: l.from,
                end: l.to,
                color: l.color,
                width: l.width,
            };
            let out = &mut self.lines[usize::from(l.depth == WorldDepth::ThroughWalls)];
            gizmos::camera_relative_to(&[line], self.camera, self.view, |line| out.push(line));
        }
        Ok(())
    }
    /// Append an entire quad batch or leave the output unchanged.
    pub fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        if quads.len() > (CAPACITY - self.usage().quads) as usize {
            return Err(DrawError::CapacityExceeded);
        }
        self.validate_positions(quads.iter().flat_map(|q| q.corners))?;
        for q in quads {
            let color = crate::world_primitives::vertex_color(q.color, 255);
            self.quads[usize::from(q.depth == WorldDepth::ThroughWalls)].extend(q.corners.map(
                |p| Vertex {
                    pos: std::array::from_fn(|i| (p[i] - self.camera[i]) as f32),
                    uv: self.white,
                    color,
                    layer: 0,
                    light: 0,
                    normal: [0; 4],
                    glint_uv: [0.0; 2],
                },
            ));
        }
        Ok(())
    }
    /// Append an entire batch of quads drawn by the mod pipeline in `slot`, or leave the
    /// output unchanged. The caller has resolved the handle; quads that follow the last ones
    /// with the same pipeline and stage join their run.
    pub fn pipeline_quads(
        &mut self,
        slot: u32,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        if quads.len() > (CAPACITY - self.usage().quads) as usize {
            return Err(PipelineDrawError::CapacityExceeded);
        }
        self.validate_positions(quads.iter().flat_map(|q| q.corners))
            .map_err(|_| PipelineDrawError::NonFinite)?;
        if quads
            .iter()
            .flat_map(|q| q.uv)
            .flatten()
            .any(|c| !c.is_finite())
        {
            return Err(PipelineDrawError::NonFinite);
        }
        if quads.is_empty() {
            return Ok(());
        }
        for q in quads {
            let color = crate::world_primitives::vertex_color(q.color, 255);
            self.pipeline_quads
                .extend((0..4).map(|corner| PipelineVertex {
                    pos: std::array::from_fn(|i| (q.corners[corner][i] - self.camera[i]) as f32),
                    uv: q.uv[corner],
                    color,
                }));
        }
        let (stage, count) = (self.stage as u32, quads.len() as u32);
        match self.pipeline_runs.last_mut() {
            Some(run) if run.slot == slot && run.stage == stage => run.quads += count,
            _ => self.pipeline_runs.push(PipelineRun {
                slot,
                stage,
                quads: count,
            }),
        }
        Ok(())
    }
    /// Borrow the existing renderer's batch, with no copying or frame allocation. The frame's
    /// builder names the pipeline set the queued slots belong to.
    pub fn batch(&self) -> Batch<'_> {
        Batch {
            lines: &self.lines[0],
            lines_through: &self.lines[1],
            quads: &self.quads[0],
            quads_through: &self.quads[1],
            stage_ends: self.stage_ends(),
            pipelines: mod_pipelines::Batch {
                set: None,
                vertices: &self.pipeline_quads,
                runs: &self.pipeline_runs,
                time: self.time,
            },
        }
    }
    /// [`Self::batch`] for a frame drawn with the resources that built `pipelines`, the set
    /// the queued pipeline quads' slots were resolved in.
    pub fn batch_with<'a>(&'a self, pipelines: &'a crate::mod_pipelines::Pipelines) -> Batch<'a> {
        let mut batch = self.batch();
        batch.pipelines.set = Some(pipelines);
        batch
    }
    /// Quads, quads through walls, lines and lines through walls queued so far.
    fn counts(&self) -> [u32; 4] {
        [
            self.quads[0].len() as u32 / 4,
            self.quads[1].len() as u32 / 4,
            self.lines[0].len() as u32,
            self.lines[1].len() as u32,
        ]
    }
    /// The ends of the stages before the current one; those not reached yet end at the counts
    /// so far, so what follows draws at the current stage.
    fn stage_ends(&self) -> [[u32; 4]; STAGES - 1] {
        let counts = self.counts();
        std::array::from_fn(|stage| {
            if stage < self.stage {
                self.stage_ends[stage]
            } else {
                counts
            }
        })
    }
}

/// Row-major 3×4 to a column-major matrix. The last row is the affine 0,0,0,1.
#[inline]
pub fn row_major_3x4(m: [f32; 12]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m[0], m[4], m[8], 0.0),
        Vec4::new(m[1], m[5], m[9], 0.0),
        Vec4::new(m[2], m[6], m[10], 0.0),
        Vec4::new(m[3], m[7], m[11], 1.0),
    )
}

/// Block-entity vertex light: sky in the high nibble and block in the low one.
pub fn pack_light(light: mod_api::LightCoords) -> u32 {
    crate::block_entity_render::vertex_light((light.sky << 4) | light.block)
}

/// `LivingEntityRenderer.getOverlayCoords` as the entity shader's overlay code.
pub fn pack_overlay(overlay: mod_api::Overlay) -> u32 {
    if overlay.hurt {
        return 1;
    }
    match u32::from(overlay.white.min(15)) {
        0 => 0,
        column => crate::overlay::WHITE + column,
    }
}

fn copy_visible(
    bounds: [[f32; 3]; 2],
    transform: [f32; 12],
    origin: [f64; 3],
    camera: [f64; 3],
    cull: Option<&impl crate::gfx::Cull>,
) -> bool {
    let Some(cull) = cull else {
        return true;
    };
    if !bounds.iter().flatten().all(|v| v.is_finite()) {
        return true;
    }
    let local = row_major_3x4(transform);
    let delta = DVec3::from(origin) - DVec3::from(camera);
    let shift = Vec3::new(delta.x as f32, delta.y as f32, delta.z as f32);
    // Affine extrema are selected independently for each output axis. Keep the
    // multiply/add order of transform_point3, without transforming eight corners.
    let x0 = local.x_axis.truncate() * bounds[0][0];
    let x1 = local.x_axis.truncate() * bounds[1][0];
    let y0 = local.y_axis.truncate() * bounds[0][1];
    let y1 = local.y_axis.truncate() * bounds[1][1];
    let z0 = local.z_axis.truncate() * bounds[0][2];
    let z1 = local.z_axis.truncate() * bounds[1][2];
    let translation = local.w_axis.truncate();
    let mut min = translation + (z0.min(z1) + (y0.min(y1) + x0.min(x1))) + shift;
    let mut max = translation + (z0.max(z1) + (y0.max(y1) + x0.max(x1))) + shift;
    if !min.is_finite() || !max.is_finite() {
        // Overflow can produce NaNs whose min/max depends on corner order.
        // Preserve that order only for these exceptional finite inputs.
        min = Vec3::splat(f32::INFINITY);
        max = Vec3::splat(f32::NEG_INFINITY);
        for corner in 0..8 {
            let point = Vec3::new(
                bounds[usize::from(corner & 1 != 0)][0],
                bounds[usize::from(corner & 2 != 0)][1],
                bounds[usize::from(corner & 4 != 0)][2],
            );
            let world = local.transform_point3(point) + shift;
            min = min.min(world);
            max = max.max(world);
        }
    }
    !min.is_finite() || !max.is_finite() || cull.intersects(min, max)
}

pub fn validate_instances(instances: &[ModelInstance]) -> Result<(), InstanceError> {
    for instance in instances {
        if instance.origin.iter().any(|v| !v.is_finite())
            || instance.transform.iter().any(|v| !v.is_finite())
        {
            return Err(InstanceError::NonFinite);
        }
        if instance.light.block > 15 || instance.light.sky > 15 {
            return Err(InstanceError::InvalidLight);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stages_end_where_the_next_begins_and_never_go_back() {
        use RenderLevelStage::*;
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        let line = |depth| WorldLine::new([0.0, 0.0, -1.0], [1.0, 0.0, -1.0], !0, 1.0, depth);
        let quad = WorldQuad::new([[0.0, 0.0, -1.0]; 4], !0, WorldDepth::ThroughWalls);
        draw.begin_stage(AfterSky);
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        // Nothing at AfterOpaqueBlocks; a quad through walls at AfterTranslucentBlocks.
        draw.begin_stage(AfterTranslucentBlocks);
        draw.quads(&[quad]).unwrap();
        // Still in the current stage: what follows a backwards step is not reordered.
        draw.begin_stage(AfterSky);
        draw.lines(&[line(WorldDepth::ThroughWalls)]).unwrap();
        assert_eq!(
            draw.batch().stage_ends,
            [[0, 0, 1, 0], [0, 0, 1, 0], [0, 1, 1, 1], [0, 1, 1, 1]],
            "the current stage runs to the end until the next begins"
        );
        draw.begin_stage(AfterLevel);
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        assert_eq!(
            draw.batch().stage_ends,
            [[0, 0, 1, 0], [0, 0, 1, 0], [0, 1, 1, 1], [0, 1, 1, 1]]
        );
        assert_eq!(draw.batch().lines.len(), 2);
        // A new view starts at the first stage again.
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        assert_eq!(draw.batch().stage_ends, [[0, 0, 1, 0]; 4]);
    }
    #[test]
    fn world_geometry_batches_are_atomic_and_reset_per_view() {
        let mut draw = WorldGeometry::default();
        draw.reset([30_000_000.0, 20.0, 0.0], Mat4::IDENTITY, [0.25, 0.75]);
        let line = WorldLine::new(
            [30_000_000.25, 20.0, -1.0],
            [30_000_001.0, 20.0, -1.0],
            0x80402010,
            2.0,
            WorldDepth::Tested,
        );
        draw.lines(&[line]).unwrap();
        assert_eq!(
            &draw.batch().lines[0][..6],
            &[0.25, 0.0, -1.0, 1.0, 0.0, -1.0]
        );
        assert_eq!(draw.batch().lines[0][6].to_bits(), line.color);
        let mut bad = line;
        bad.width = 0.0;
        assert_eq!(draw.lines(&[line, bad]), Err(DrawError::InvalidWidth));
        bad.width = f32::NAN;
        assert_eq!(draw.lines(&[line, bad]), Err(DrawError::NonFinite));
        bad = line;
        bad.to[0] = f64::MAX;
        assert_eq!(draw.lines(&[line, bad]), Err(DrawError::NonFinite));
        assert_eq!(draw.usage().lines, 1);
        let quad = WorldQuad::new([line.from; 4], line.color, WorldDepth::ThroughWalls);
        draw.quads(&[quad]).unwrap();
        assert_eq!(draw.batch().quads_through.len(), 4);
        assert_eq!(draw.batch().quads_through[0].pos, [0.25, 0.0, -1.0]);
        assert_eq!(
            draw.batch().quads_through[0].color.to_le_bytes(),
            [0x40, 0x20, 0x10, 0x80]
        );
        let mut invalid_quad = quad;
        invalid_quad.corners[3][1] = f64::INFINITY;
        assert_eq!(draw.quads(&[quad, invalid_quad]), Err(DrawError::NonFinite));
        assert_eq!(draw.usage().quads, 1);
        let mut through = line;
        through.depth = WorldDepth::ThroughWalls;
        draw.lines(&vec![through; CAPACITY as usize - 1]).unwrap();
        assert_eq!(draw.lines(&[line]), Err(DrawError::CapacityExceeded));
        draw.quads(&vec![quad; CAPACITY as usize - 1]).unwrap();
        assert_eq!(draw.quads(&[quad]), Err(DrawError::CapacityExceeded));
        let capacities = (
            draw.lines.each_ref().map(Vec::capacity),
            draw.quads.each_ref().map(Vec::capacity),
        );
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert!(draw.batch().is_empty());
        assert_eq!(
            capacities,
            (
                draw.lines.each_ref().map(Vec::capacity),
                draw.quads.each_ref().map(Vec::capacity)
            )
        );
    }

    #[test]
    fn pipeline_quads_are_atomic_share_the_quad_capacity_and_run_by_pipeline_and_stage() {
        use RenderLevelStage::*;
        let mut draw = WorldGeometry::default();
        draw.reset([30_000_000.0, 20.0, 0.0], Mat4::IDENTITY, [0.5, 0.5]);
        draw.set_time(12.5);
        let quad = PipelineQuad::new(
            [
                [30_000_000.25, 21.0, -1.0],
                [30_000_000.25, 20.0, -1.0],
                [30_000_001.25, 20.0, -1.0],
                [30_000_001.25, 21.0, -1.0],
            ],
            [[0.0, 0.0], [0.0, 2.0], [3.0, 2.0], [3.0, 0.0]],
            0x80402010,
        );
        draw.begin_stage(AfterSky);
        draw.pipeline_quads(4, &[quad, quad]).unwrap();
        // The same pipeline at the same stage joins the run; another pipeline starts one, and
        // so does the first pipeline again after it, to keep the order submitted.
        draw.pipeline_quads(4, &[quad]).unwrap();
        draw.pipeline_quads(1, &[quad]).unwrap();
        draw.pipeline_quads(4, &[quad]).unwrap();
        draw.begin_stage(AfterWeather);
        draw.pipeline_quads(4, &[quad]).unwrap();
        draw.pipeline_quads(4, &[]).unwrap();

        let pipelines = draw.batch().pipelines;
        let run = |slot, stage, quads| PipelineRun { slot, stage, quads };
        assert_eq!(
            pipelines.runs,
            [run(4, 0, 3), run(1, 0, 1), run(4, 0, 1), run(4, 3, 1)]
        );
        assert_eq!(pipelines.vertices.len(), 24);
        assert_eq!(pipelines.time, 12.5);
        assert!(pipelines.set.is_none());
        // Camera-relative before narrowing to f32, texture coordinates as given, and the
        // colour's bytes red first.
        let corner = pipelines.vertices[2];
        assert_eq!(corner.pos, [1.25, 0.0, -1.0]);
        assert_eq!(corner.uv, [3.0, 2.0]);
        assert_eq!(corner.color.to_le_bytes(), [0x40, 0x20, 0x10, 0x80]);
        assert!(!draw.batch().is_empty());

        // A fault anywhere refuses the whole batch.
        let mut bad_corner = quad;
        bad_corner.corners[3][1] = f64::NAN;
        let mut bad_uv = quad;
        bad_uv.uv[1][0] = f32::INFINITY;
        for bad in [bad_corner, bad_uv] {
            assert_eq!(
                draw.pipeline_quads(4, &[quad, bad]),
                Err(PipelineDrawError::NonFinite)
            );
        }
        assert_eq!(draw.usage().quads, 6);

        // One capacity for plain and pipeline quads.
        let plain = WorldQuad::new(quad.corners, !0, WorldDepth::Tested);
        draw.quads(&vec![plain; CAPACITY as usize - 7]).unwrap();
        assert_eq!(
            draw.pipeline_quads(4, &[quad, quad]),
            Err(PipelineDrawError::CapacityExceeded)
        );
        draw.pipeline_quads(4, &[quad]).unwrap();
        assert_eq!(draw.quads(&[plain]), Err(DrawError::CapacityExceeded));
        assert_eq!(draw.usage().quads, CAPACITY);

        let capacity = (
            draw.pipeline_quads.capacity(),
            draw.pipeline_runs.capacity(),
        );
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert!(draw.batch().is_empty());
        assert_eq!(
            capacity,
            (
                draw.pipeline_quads.capacity(),
                draw.pipeline_runs.capacity()
            ),
            "a view's pipeline quads never grow the storage reserved at setup"
        );
    }

    struct NearX;
    impl crate::gfx::Cull for NearX {
        fn intersects(&self, min: glam::Vec3, max: glam::Vec3) -> bool {
            max.x >= 0.0 && min.x <= 4.0
        }
        fn contains(&self, min: glam::Vec3, max: glam::Vec3) -> bool {
            min.x >= 0.0 && max.x <= 4.0
        }
    }

    #[test]
    fn model_batches_are_atomic_cull_and_stop_at_the_view_budget() {
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        let batch = ModelBatch {
            slot: 3,
            generation: 9,
            translucent: false,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let oversized = vec![ModelInstance::at([f64::NAN; 3], 0); 16_385];
        assert_eq!(
            draw.submit_model(batch, &oversized, None::<&NearX>, |_| panic!(
                "oversized batch staged"
            )),
            Err(InstanceError::CapacityExceeded)
        );
        let kept = draw.usage();
        let mut bad = ModelInstance::at([1.0, 0.0, 0.0], 0xFF11_2233);
        bad.light.block = 16;
        assert_eq!(
            draw.submit_model(
                batch,
                &[ModelInstance::at([1.0, 0.0, 0.0], 1), bad],
                None::<&NearX>,
                |_| false
            ),
            Err(InstanceError::InvalidLight)
        );
        bad.light.block = 15;
        bad.origin[0] = f64::NAN;
        assert_eq!(
            draw.submit_model(batch, &[bad], None::<&NearX>, |_| false),
            Err(InstanceError::NonFinite)
        );
        bad.origin[0] = 1.0;
        bad.transform[4] = f32::INFINITY;
        assert_eq!(
            draw.submit_model(batch, &[bad], None::<&NearX>, |_| false),
            Err(InstanceError::NonFinite)
        );
        assert_eq!(
            draw.submit_model(
                ModelBatch {
                    translucent: true,
                    ..batch
                },
                &[ModelInstance::at([1.0, 0.0, 0.0], 1)],
                None::<&NearX>,
                |_| false,
            ),
            Err(InstanceError::Translucent)
        );
        assert_eq!(draw.usage(), kept);
        let near = ModelInstance::at([1.0, 2.0, 3.0], 0xFF80_C040);
        let far = ModelInstance::at([40.0, 0.0, 0.0], 0xFF80_C040);
        draw.submit_model(batch, &[near, far], Some(&NearX), |_| false)
            .unwrap();
        assert_eq!(draw.copy_count(), 1);
        assert_eq!(draw.copy(0).light, 0xF0F0);
        assert_eq!(draw.copy(0).overlay, 0);
        assert_eq!(draw.usage().instances_used, 1);
        assert_eq!(draw.usage().instances_capacity, INSTANCE_CAPACITY);
        let mut hurt = near;
        hurt.overlay = mod_api::Overlay::new(8, true).unwrap();
        hurt.light = mod_api::LightCoords::new(15, 3).unwrap();
        draw.submit_model(batch, &[hurt], None::<&NearX>, |_| false)
            .unwrap();
        assert_eq!(draw.copy(1).light, 0xF030);
        assert_eq!(draw.copy(1).overlay, 1);
        let fill = INSTANCE_CAPACITY as usize - draw.copy_count();
        draw.submit_model(batch, &vec![near; fill], None::<&NearX>, |_| false)
            .unwrap();
        assert_eq!(
            draw.submit_model(batch, &[near], None::<&NearX>, |_| false),
            Err(InstanceError::CapacityExceeded)
        );
        assert_eq!(draw.copy_count(), INSTANCE_CAPACITY as usize);
        let reserved = draw.copies.capacity();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert_eq!(draw.copy_count(), 0);
        assert!(draw.copies.capacity() >= reserved);
    }
    #[test]
    fn model_capacity_failure_restores_the_previous_submission() {
        struct Visible;
        impl crate::gfx::Cull for Visible {
            fn intersects(&self, _: Vec3, _: Vec3) -> bool {
                true
            }
            fn contains(&self, _: Vec3, _: Vec3) -> bool {
                false
            }
        }
        let batch = ModelBatch {
            slot: 7,
            generation: 3,
            translucent: false,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let mut draw = WorldGeometry::default();
        let original = ModelInstance::at([1.0, 2.0, 3.0], 0xFF123456);
        draw.submit_model(
            batch,
            &vec![original; INSTANCE_CAPACITY as usize - 1],
            Some(&Visible),
            |_| false,
        )
        .unwrap();
        let incoming = ModelInstance::at([4.0, 5.0, 6.0], 0xFFABCDEF);
        let usage = draw.usage();
        let capacity = draw.copies.capacity();
        assert_eq!(
            draw.submit_model(batch, &[incoming, incoming], Some(&Visible), |_| false),
            Err(InstanceError::CapacityExceeded)
        );
        assert_eq!(draw.usage(), usage);
        assert_eq!(draw.copies.capacity(), capacity);
        assert!(
            draw.copies
                .iter()
                .all(|copy| copy.origin == original.origin && copy.tint == original.tint)
        );
        // A later invalid copy still wins over capacity failure.
        let mut invalid = incoming;
        invalid.origin[0] = f64::NAN;
        assert_eq!(
            draw.submit_model(
                batch,
                &[incoming, incoming, invalid],
                Some(&Visible),
                |_| false
            ),
            Err(InstanceError::NonFinite)
        );
        assert_eq!(draw.usage(), usage);
        draw.submit_model(batch, &[incoming], Some(&Visible), |_| false)
            .unwrap();
        assert_eq!(
            draw.copy(INSTANCE_CAPACITY as usize - 1).origin,
            incoming.origin
        );
    }
    #[test]
    fn directly_staged_models_share_the_view_budget() {
        let mut draw = WorldGeometry::default();
        let batch = ModelBatch {
            slot: 7,
            generation: 3,
            translucent: false,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let copy = ModelInstance::at([1.0, 2.0, 3.0], u32::MAX);
        draw.submit_model(
            batch,
            &vec![copy; INSTANCE_CAPACITY as usize],
            None::<&crate::gfx::Frustum>,
            |_| true,
        )
        .unwrap();
        assert_eq!(draw.copy_count(), 0, "direct copies must not also be baked");
        assert_eq!(draw.usage().instances_used, INSTANCE_CAPACITY);
        assert_eq!(
            draw.submit_model(batch, &[copy], None::<&crate::gfx::Frustum>, |_| true),
            Err(InstanceError::CapacityExceeded)
        );
        assert_eq!(draw.usage().instances_used, INSTANCE_CAPACITY);
    }

    #[test]
    fn affine_culling_matches_the_eight_corner_box() {
        use std::cell::Cell;
        struct Bounds(Cell<[Vec3; 2]>);
        impl crate::gfx::Cull for Bounds {
            fn intersects(&self, min: Vec3, max: Vec3) -> bool {
                self.0.set([min, max]);
                true
            }
            fn contains(&self, _: Vec3, _: Vec3) -> bool {
                false
            }
        }
        let capture = Bounds(Cell::new([Vec3::ZERO; 2]));
        let bounds = [[-0.75, -2.0, 0.125], [1.5, 0.5, 3.0]];
        let origin = [30_000_000.25, -100.5, -30_000_000.75];
        let camera = [30_000_000.0, 5.25, -30_000_000.0];
        let shift = (DVec3::from(origin) - DVec3::from(camera)).as_vec3();
        let mut seed = 42u32;
        for case in 0..1024 {
            // Include shear, reflections, nonuniform scales, and finite inputs that overflow.
            let scale = if case < 512 { 1.0 } else { f32::MAX * 0.25 };
            let transform = std::array::from_fn(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 8) as f32 / 2097152.0 - 4.0) * scale
            });
            let matrix = row_major_3x4(transform);
            let mut expected = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
            for corner in 0..8 {
                let point = Vec3::new(
                    bounds[corner & 1][0],
                    bounds[(corner >> 1) & 1][1],
                    bounds[(corner >> 2) & 1][2],
                );
                let point = matrix.transform_point3(point) + shift;
                expected[0] = expected[0].min(point);
                expected[1] = expected[1].max(point);
            }
            assert!(copy_visible(
                bounds,
                transform,
                origin,
                camera,
                Some(&capture)
            ));
            if expected.iter().all(|point| point.is_finite()) {
                assert_eq!(capture.0.get(), expected);
            }
        }
    }
}
