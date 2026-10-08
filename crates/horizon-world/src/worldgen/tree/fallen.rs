//! `minecraft:fallen_tree`: a stump and an optional fallen log with decorators.
use super::{
    NestedPlacer, Run, Sets, TreeDecoder, TreeResult, decorator::Lists, decorator::NestedDecoder,
    decorator::TreeDecorator, kind, read, sample_state, write,
};
use crate::worldgen::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    feature_graph::FeatureExecutionError,
    placement::PlacementContext,
    required,
    state_provider::BlockStateProvider,
    tree_blocks::{self as tb, Axis, PosSet, TreeBlocks},
    value_provider::IntProvider,
};
use horizon_core::{geometry::Direction, random::RandomSource};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct FallenTreeFeature {
    trunk_provider: BlockStateProvider,
    log_length: IntProvider,
    stump_decorators: Vec<TreeDecorator>,
    log_decorators: Vec<TreeDecorator>,
    blocks: Arc<TreeBlocks>,
}
impl FallenTreeFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
        nested: &NestedDecoder<'_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:fallen_tree" {
            return Err(DecodeError::Field("fallen tree feature type"));
        }
        let blocks = bindings.tree_blocks()?;
        let mut decoder = TreeDecoder {
            bindings,
            depth,
            remaining,
        };
        Ok(Self {
            trunk_provider: decoder.provider(required(&object, "trunk_provider")?)?,
            log_length: decoder.int_provider(required(&object, "log_length")?, 0, 16)?,
            stump_decorators: TreeDecorator::decode_list(
                required(&object, "stump_decorators")?,
                &mut decoder,
                None,
                nested,
            )?,
            log_decorators: TreeDecorator::decode_list(
                required(&object, "log_decorators")?,
                &mut decoder,
                None,
                nested,
            )?,
            blocks,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        nested: &mut NestedPlacer<'_, W, R>,
    ) -> TreeResult<bool, W::Error> {
        let mut run = Run {
            world,
            random,
            sets: Sets::default(),
            track: false,
        };
        let stump = self.place_log(&mut run, origin, None)?;
        self.decorate(&mut run, &[stump], &self.stump_decorators, nested)?;
        let direction = tb::HORIZONTAL[run.random.next_int_bound(4)? as usize];
        let length = self.log_length.sample(run.random)? - 2;
        let mut start = tb::relative(origin, direction, 2 + run.random.next_int_bound(2)?);
        start = tb::offset(start, 0, 1, 0);
        for _ in 0..6 {
            if self.may_place_on(&mut run, start)? {
                break;
            }
            start = tb::offset(start, 0, -1, 0);
        }
        if self.can_place_log(&mut run, length, start, direction)? {
            let mut logs = PosSet::default();
            let mut p = start;
            for _ in 0..length {
                logs.insert(self.place_log(&mut run, p, Some(Axis::of(direction)))?);
                p = tb::relative(p, direction, 1);
            }
            self.decorate(&mut run, &logs.to_vec(), &self.log_decorators, nested)?;
        }
        Ok(true)
    }
    fn decorate<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        logs: &[[i32; 3]],
        decorators: &[TreeDecorator],
        nested: &mut NestedPlacer<'_, W, R>,
    ) -> TreeResult<(), W::Error> {
        if decorators.is_empty() {
            return Ok(());
        }
        let lists = Lists::new(logs.to_vec(), vec![], vec![]);
        for decorator in decorators {
            decorator.place(run, &self.blocks, &lists, nested)?;
        }
        Ok(())
    }
    fn place_log<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        p: [i32; 3],
        axis: Option<Axis>,
    ) -> TreeResult<[i32; 3], W::Error> {
        let mut state = sample_state(&self.trunk_provider, run.world, run.random, p)?;
        if let Some(axis) = axis {
            state = self.blocks.try_axis(state, axis);
        }
        write(run.world, p, state, 3)?;
        // Feature.markAboveForPostProcessing.
        let mut above = p;
        for _ in 0..2 {
            above = tb::offset(above, 0, 1, 0);
            if self.blocks.is_air(read(run.world, above)?) {
                break;
            }
            run.world
                .mark_postprocessing(above)
                .map_err(FeatureExecutionError::Query)?;
        }
        Ok(p)
    }
    fn over_solid_ground<W: FeatureWorld + PlacementContext, R>(
        &self,
        run: &mut Run<'_, W, R>,
        p: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let below = read(run.world, tb::offset(p, 0, -1, 0))?;
        run.world
            .state_face_sturdy(below, p, Direction::Up)
            .map_err(FeatureExecutionError::Query)
    }
    fn may_place_on<W: FeatureWorld + PlacementContext, R>(
        &self,
        run: &mut Run<'_, W, R>,
        p: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        Ok(self.blocks.valid_tree_pos(read(run.world, p)?) && self.over_solid_ground(run, p)?)
    }
    fn can_place_log<W: FeatureWorld + PlacementContext, R>(
        &self,
        run: &mut Run<'_, W, R>,
        length: i32,
        start: [i32; 3],
        direction: Direction,
    ) -> TreeResult<bool, W::Error> {
        let mut gap = 0;
        let mut p = start;
        for _ in 0..length {
            if !self.blocks.valid_tree_pos(read(run.world, p)?) {
                return Ok(false);
            }
            if !self.over_solid_ground(run, p)? {
                gap += 1;
                if gap > 2 {
                    return Ok(false);
                }
            } else {
                gap = 0;
            }
            p = tb::relative(p, direction, 1);
        }
        Ok(true)
    }
}
