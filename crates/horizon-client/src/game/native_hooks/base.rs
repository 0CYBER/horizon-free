use super::*;
use mod_api::ModBlockEntity;
impl GameLevel<'_> {
    /// Initial saved data for a declared block entity (`EntityBlock.newBlockEntity`),
    /// fired inside the write that creates it (`LevelChunk.setBlockState`), on
    /// a server and on a client of one. The declaration owns the type id; no
    /// answer starts with an empty compound. The level is read-only, as
    /// `newBlockEntity` gets no level.
    #[inline(always)]
    pub(in crate::game) fn native_block_entity(
        &self,
        block: &str,
        state: BlockStateId,
        entity: &mut Option<Compound>,
    ) {
        self.fire_block_entity(block, state, entity);
    }
    /// A hopper at `pos` is about to pick up `count` of `item`. Set `cancel` to leave the dropped stack in the world. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_hopper_pickup_item(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        item: &str,
        count: u32,
        cancel: &mut bool,
    ) {
        self.fire_hopper_pickup_item(dimension, pos, item, count, cancel);
    }
    /// An authoritative server committed a changed state through `Level.setBlock`.
    /// The world contains `new`; block side effects and neighbour updates follow.
    #[inline(always)]
    pub(in crate::game) fn native_block_changed(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) {
        self.fire_block_changed(dimension, pos, old, new);
    }
    /// A flowing fluid at `pos` is about to change level. `fluid` is `water`
    /// or `lava`; the levels are 0 through 8. Set `cancel` to keep the old
    /// level. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_fluid_level_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        fluid: &str,
        from: i32,
        to: i32,
        cancel: &mut bool,
    ) {
        self.fire_fluid_level_change(dimension, pos, fluid, from, to, cancel);
    }

    /// A non-persistent leaf block at distance 7 is about to decay at `pos`.
    /// Set `cancel` to keep it. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_leaves_decay(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        self.fire_leaves_decay(dimension, pos, cancel);
    }

    /// Farmland at `pos` is about to change moisture from `from` to `to`.
    /// Set `cancel` to keep its old moisture. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_moisture_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: i32,
        to: i32,
        cancel: &mut bool,
    ) {
        self.fire_moisture_change(dimension, pos, from, to, cancel);
    }

    /// A sponge at `pos` absorbed water and became wet. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_sponge_absorb(&self, dimension: LevelDimension, pos: BlockPos) {
        self.fire_sponge_absorb(dimension, pos);
    }

    /// A block at `pos` is about to fade from `from` to `to` because of
    /// melting or coral death. Set `cancel` to keep it. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_block_fade(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
        cancel: &mut bool,
    ) {
        self.fire_block_fade(dimension, pos, from, to, cancel);
    }

    /// Precipitation is about to form or add to a block at `pos`. `from` and
    /// `to` are block state IDs. Set `cancel` to keep the old state. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_block_form(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
        cancel: &mut bool,
    ) {
        self.fire_block_form(dimension, pos, from, to, cancel);
    }

    /// The Ender Dragon fight is about to place its first dragon egg at `pos`.
    /// Set `cancel` to leave the block unchanged. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_dragon_egg_form(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        self.fire_dragon_egg_form(dimension, pos, cancel);
    }

    /// An authoritative server is about to deliver `neighborChanged`.
    /// State and block IDs are valid in this level's registries for this call.
    /// The host is the level with its ticks, as vanilla's `neighborChanged` gets the
    /// level: a part may read its inputs (`signal`, `has_neighbor_signal`) and
    /// schedule its tick here, as a diode does (`DiodeBlock.checkTickOnNeighbor`).
    #[inline(always)]
    pub(in crate::game) fn native_neighbor_changed(
        &self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        changed: mod_api::BlockKey,
        moved: bool,
    ) {
        self.fire_neighbor_changed(ticks, dimension, pos, state, changed, moved);
    }

    /// On an authoritative server, a fluid flows or fire spreads from `from`
    /// into `to` (`kind` is `fluid` or `fire`): `FlowingFluid.spreadTo`, and
    /// `FireBlock.tick` setting fire to `to` or burning it (`checkBurnOut`).
    /// Set `canceled` to leave the block at `to` as it is. Fluids ask on
    /// every flow: answer fast.
    #[inline(always)]
    pub(in crate::game) fn native_block_spread(
        &self,
        dimension: LevelDimension,
        kind: &str,
        from: BlockPos,
        to: BlockPos,
        canceled: &mut bool,
    ) {
        self.fire_block_spread(dimension, kind, from, to, canceled);
    }

    /// On an authoritative server, fire is about to be placed at `pos` by
    /// flint and steel, a fire charge, lava, lightning or fire spread. Set
    /// `player` is the acting server player when there is one. Set `cancel` to
    /// leave the position unchanged.
    #[inline(always)]
    pub(in crate::game) fn native_block_ignite(
        &self,
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        cause: &str,
        cancel: &mut bool,
    ) {
        self.fire_block_ignite(dimension, player, pos, cause, cancel);
    }

    /// On an authoritative server, fire is about to burn the block at `pos`
    /// away. Set `cancel` to keep that block.
    #[inline(always)]
    pub(in crate::game) fn native_block_burn(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        self.fire_block_burn(dimension, pos, cancel);
    }

    /// On an authoritative server, a bell at `pos` is about to ring. Set
    /// `cancel` to stop the ring and its raider reveal.
    #[inline(always)]
    pub(in crate::game) fn native_bell_ring(
        &self,
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        self.fire_bell_ring(dimension, player, pos, cancel);
    }

    /// On an authoritative server, a cauldron at `pos` is about to change
    /// fill level from `from_level` to `to_level` for `reason`. Set `cancel`
    /// to leave it unchanged.
    #[inline(always)]
    pub(in crate::game) fn native_cauldron_level_changed(
        &self,
        call: mod_api::__private::CauldronLevelChangedCall<'_>,
    ) {
        self.fire_cauldron_level_changed(call);
    }

    /// On an authoritative server, the piston at `piston` is about to push
    /// the blocks at `push` one step `direction` (`[0, 1, 0]` is up; pulling
    /// when it retracts, `extending` false) and destroy those at `destroy`:
    /// NeoForge's `PistonEvent.Pre`, in `PistonBaseBlock.moveBlocks`, once its
    /// structure resolved. Set `canceled` and nothing moves, as when the
    /// structure can't; a sticky piston then retracts without its block, and
    /// clients are sent the blocks rather than the retraction to replay.
    #[inline(always)]
    pub(in crate::game) fn native_piston_pre(&self, call: mod_api::__private::PistonPreCall<'_>) {
        self.fire_piston_pre(call);
    }
    /// On an authoritative server, the piston at `piston` pushed the blocks
    /// at `push` one step `direction` and destroyed those at `destroy`:
    /// NeoForge's `PistonEvent.Post`, at the end of
    /// `PistonBaseBlock.moveBlocks`, after the neighbour updates. It only
    /// hears; `native_piston_pre` is the veto.
    #[inline(always)]
    pub(in crate::game) fn native_piston_post(
        &self,
        dimension: LevelDimension,
        piston: BlockPos,
        direction: [i32; 3],
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) {
        self.fire_piston_post(dimension, piston, direction, extending, push, destroy);
    }
    /// The redstone signal the block `state` at `pos` gives on its `face`, the
    /// side that touches the block reading it: `BlockState.getDirectSignal`
    /// when `direct` (what passes through a block it touches, as a lever's
    /// does), else `getSignal`. `signal` holds vanilla's answer: change it when
    /// the block is yours (0 to 15; anything else is clamped). Asked very often,
    /// from inside the level's update, so answer from your own state: check the
    /// block first, and don't read or change the world here. When the answer
    /// changes, and once the block is placed, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::Neighbours)`. Authoritative
    /// servers only.
    #[inline(always)]
    pub(in crate::game) fn native_block_signal(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        face: Direction,
        direct: bool,
        signal: &mut i32,
    ) {
        self.fire_block_signal(dimension, pos, state, face, direct, signal);
    }

    /// `BlockState.isSignalSource`: set `source` for a block of yours that
    /// gives a signal, so redstone wire beside it connects to it. Authoritative
    /// servers only.
    #[inline(always)]
    pub(in crate::game) fn native_is_signal_source(&self, state: BlockStateId, source: &mut bool) {
        self.fire_is_signal_source(state, source);
    }

    /// `BlockState.hasAnalogOutputSignal`: set `output` for a block of yours
    /// that comparators read (how full or far along it is, as a composter's
    /// level), so they ask `native_analog_output`. Asked from inside the
    /// level's update: answer from `state` alone. Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_has_analog_output(&self, state: BlockStateId, output: &mut bool) {
        self.fire_has_analog_output(state, output);
    }

    /// `BlockState.getAnalogOutputSignal`: the signal a comparator reads from
    /// the block `state` at `pos`, `face` the side that touches the comparator
    /// (or the conductor in front of it). `output` holds vanilla's answer:
    /// change it when the block is yours (0 to 15; anything else is clamped).
    /// Asked only for blocks `native_has_analog_output` answered for, from
    /// inside the level's update: don't change the world here. When the
    /// answer changes without the block's state changing, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::OutputSignal)`; a change of
    /// state through
    /// `native_set_block` or `native_set_block_property` (neighbour updates
    /// on) tells the comparators itself, as vanilla's `Level.setBlock` does.
    /// Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_analog_output(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        face: Direction,
        output: &mut i32,
    ) {
        self.fire_analog_output(dimension, pos, state, face, output);
    }

    /// An authoritative server's `ServerLevel.tickChunk` is about to
    /// random-tick the block `state` at `pos` (`BlockState.randomTick`):
    /// only blocks that tick randomly, a mod's as its vanilla template block
    /// does. Bone meal's random ticks (`StemBlock.performBonemeal`) don't
    /// fire it. Change the block with `native_set_block_property` and set
    /// `handled` to skip the block's own random tick (a mod crop growing its
    /// own way); a block changed without `handled` takes its own random tick
    /// as it is now, if it still ticks randomly. Lava's fluid random tick
    /// runs either way.
    #[inline(always)]
    pub(in crate::game) fn native_random_tick(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        handled: &mut bool,
    ) {
        self.fire_random_tick(ticks, dimension, pos, state, handled);
    }

    /// An authoritative server is about to compute the state the block
    /// `state` at `pos` takes now that its neighbour in `direction` is
    /// `neighbour` (`BlockBehaviour.updateShape`). `state` may not be in the
    /// world yet (a piston landing it): decide from `state`, not from the
    /// block at `pos`. Answer only for your own blocks: put the new state in
    /// `new` (`state` keeps the block as it is; air breaks it, with its drops,
    /// on a neighbour update) and the block's own `updateShape` doesn't run,
    /// nor the ticks it would schedule. The level is read-only, as vanilla's
    /// `LevelReader`; ticks go through `native_schedule_block_tick` and
    /// `native_schedule_fluid_tick` with `ticks` (`ScheduledTickAccess`). It
    /// runs often, at least six times per `setBlock` that updates shapes
    /// (more for redstone wire's diagonals and knock-on updates): keep it
    /// short.
    #[inline(always)]
    pub(in crate::game) fn native_update_shape(
        &self,
        ticks: &mut Ticks,
        call: super::fire::UpdateShapeCall<'_>,
    ) {
        self.fire_update_shape(ticks, call);
    }

    /// On an authoritative server, `BlockState.canSurvive` for `state` at
    /// `pos` (`BlockBehaviour.canSurvive`), before the block answers it: as
    /// it is placed (`BlockItem.canPlace`), from a shape update of a block
    /// whose template checks it (a plant, a torch, a sign: false breaks the
    /// block with its drops), and wherever else vanilla asks. `state` may not
    /// be in the world yet, so decide from `state`. Answer only for your own
    /// blocks: `Some(false)` refuses the spot, `Some(true)` allows it, and
    /// the block's own rule doesn't run. The level is read-only. Asked on
    /// every shape update of such a block: keep it short.
    #[inline(always)]
    pub(in crate::game) fn native_can_survive(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        survives: &mut Option<bool>,
    ) {
        self.fire_can_survive(dimension, pos, state, survives);
    }

    /// On an authoritative server, `VegetationBlock.mayPlaceOn` for the
    /// plant `state` on `ground` at `ground_pos`, before the plant answers
    /// it: the blocks a mod plant (one whose template is a vanilla plant,
    /// such as a crop) can stand on. Its template's `canSurvive` asks it, so
    /// the plant is refused there and pops off when its ground changes to
    /// one it can't stand on. Answer only for your own blocks. The level is
    /// read-only.
    #[inline(always)]
    pub(in crate::game) fn native_may_place_on(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
    ) {
        self.fire_may_place_on(dimension, state, ground, ground_pos, may);
    }

    /// An authoritative server's `ServerLevel.tickBlock` is about to run a
    /// due scheduled tick of the block `state` at `pos` (`BlockState.tick`):
    /// the block is still the one the tick was scheduled for. Change the
    /// block with `native_set_block_property` and set `handled` to skip the
    /// block's own tick; a block changed without `handled` takes its own
    /// tick as it is now, if it is still the tick's block.
    #[inline(always)]
    pub(in crate::game) fn native_tick_block(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        handled: &mut bool,
    ) {
        self.fire_tick_block(ticks, dimension, pos, state, handled);
    }

    /// A block is about to take the state it is placed in
    /// (`Block.getStateForPlacement(BlockPlaceContext)`), `state` the block's
    /// own default state. Answer only for your blocks: put the state you want
    /// in `answer` as `Some(Some(state))` — a state of that block, as vanilla
    /// sets properties on its own `defaultBlockState` (another block's is
    /// ignored) — or refuse the placement with `Some(None)`: the block goes
    /// nowhere and the stack is kept, as vanilla's `null`. Left empty, the
    /// block's own placement runs. `context` is
    /// vanilla's `BlockPlaceContext` (the clicked face, the placer's rotation,
    /// the clicked position, whether the clicked block is being replaced, the
    /// item in hand) and `fluid` the fluid in the position the block would go
    /// in, which a waterlogged block reads. Build the state from `state` with
    /// `native_state_with_property`, and know whether `state` is yours with
    /// `native_state_block`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`), and the answer still takes its
    /// survival and entity obstruction checks. A trait mod returns
    /// `Some(Some(state))` to choose a state, `Some(None)` to refuse, or
    /// `None` to leave the answer to the next mod or vanilla.
    /// Authoritative servers only.
    #[inline(always)]
    #[allow(clippy::option_option)]
    pub(in crate::game) fn native_state_for_placement(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        context: &PlacementContext,
        fluid: mod_api::FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        self.fire_state_for_placement(dimension, state, context, fluid, answer);
    }

    /// A block nothing in Horizon knows a `BonemealableBlock` for is about to
    /// take bone meal: vanilla's `BonemealableBlock.isValidBonemealTarget`,
    /// `state` the block's own state at `pos`. Answer `valid` for your own
    /// blocks: the meal is used up on one as vanilla uses it up (the growth
    /// particles and the sound follow, whether or not the roll below grows
    /// it), from a player's `BoneMealItem.useOn`, from a dispenser and, on a
    /// crop (a villager bonemeals only a `CropBlock`), from a villager. Asked
    /// on both sides, as vanilla asks it on both, so a client predicts the
    /// use with your answer: say no where you can only grow on a server
    /// (match `LevelSide`). `Some(false)` refuses the meal, even on a block
    /// whose template takes it (vanilla's `growCrop` then keeps the meal);
    /// left `None`, the block keeps the `BonemealableBlock` its template gives
    /// it, or takes no bone meal.
    #[inline(always)]
    pub(in crate::game) fn native_is_valid_bonemeal_target(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        valid: &mut Option<bool>,
    ) {
        self.fire_is_valid_bonemeal_target(dimension, pos, state, valid);
    }

    /// `BonemealableBlock.isBonemealSuccess` for a block that answered
    /// `native_is_valid_bonemeal_target`, after it did: set `success` to grow
    /// it this time. Vanilla draws the level's random here
    /// (`isBonemealSuccess(Level, RandomSource, BlockPos, BlockState)`), so
    /// draw from `native_level_random` to keep vanilla's order.
    /// Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_is_bonemeal_success(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        success: &mut bool,
    ) {
        self.fire_is_bonemeal_success(dimension, pos, state, success);
    }

    /// `BonemealableBlock.performBonemeal` for a block that answered
    /// `native_is_valid_bonemeal_target` and whose `native_is_bonemeal_success`
    /// said yes: grow it here, `state` its state at `pos`. The meal is used up
    /// either way. Change the block with the level's
    /// `native_set_block_property`, and schedule with `native_schedule_block_tick`
    /// (`ticks` is the hook's). Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_perform_bonemeal(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        self.fire_perform_bonemeal(ticks, dimension, pos, state);
    }

    /// On an authoritative server, `CampfireBlock.isSmokeSource` for the
    /// block `state` under a campfire, as the campfire is placed or the block
    /// under it changes: `Some(true)` for your own blocks makes the campfire a
    /// signal fire whose smoke rises higher, as over a hay bale. The level is
    /// read-only.
    #[inline(always)]
    pub(in crate::game) fn native_smoke_source(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        smoke: &mut Option<bool>,
    ) {
        self.fire_smoke_source(dimension, state, smoke);
    }
}

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The frame rate this window is about to be held to: Max Framerate after the
    /// inactivity throttle and Battery Mode (260 is unlimited). `focused` is whether the
    /// window has focus, `minimized` whether it is minimised and `in_world` whether a world
    /// is open. A mod may lower `limit`, as Dynamic FPS does for a window in the background:
    /// Horizon keeps the lower of its own limit and the mod's, and ignores values below 1.
    pub(in crate::game) fn native_framerate_limit(
        &self,
        focused: bool,
        minimized: bool,
        in_world: bool,
        limit: &mut i32,
    ) {
        self.fire_framerate_limit(focused, minimized, in_world, limit);
    }
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not
    /// (`client_setup` is the once-only moment after the initial load).
    pub(in crate::game) fn native_client_resources_reloaded(&mut self) {
        let phase = self.mod_phase.clone();
        let _reload = phase
            .reloading()
            .expect("client reload notice on its owning thread");
        self.fire_client_resources_reloaded();
    }
    /// The client made a new level for `dimension` (NeoForge's
    /// `LevelEvent.Load` from the `ClientLevel` constructor): on joining a
    /// server, single player's own included, and on changing dimension.
    /// Its chunks come after. When the dimension changes, the old level's
    /// `native_client_level_unload` follows this, as `Minecraft.setLevel`
    /// tells it after the new level is made. Client-side, on the window; the
    /// server's levels have `native_level_load`.
    pub(in crate::game) fn native_client_level_load(&mut self, dimension: &str) {
        self.fire_client_level_load(dimension);
    }
    /// The client dropped its level for `dimension` (`LevelEvent.Unload`
    /// from `Minecraft.setLevel` and `disconnect`): replaced by another
    /// dimension's, or on leaving the server. Once for each
    /// `native_client_level_load`. Not told when the game itself quits.
    /// Client-side.
    pub(in crate::game) fn native_client_level_unload(&mut self, dimension: &str) {
        self.fire_client_level_unload(dimension);
    }
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    pub(in crate::game) fn native_block_highlight(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        partial: f32,
        canceled: &mut bool,
    ) {
        self.fire_block_highlight(pos, state, partial, canceled);
    }
    /// The world frame's local visual sources. Push requests; Horizon validates
    /// and limits them before rendering. Called only while a world is visible.
    pub(in crate::game) fn native_visuals(
        &self,
        rims: &mut Vec<BlockRim>,
        lights: &mut Vec<ColoredLight>,
    ) {
        self.fire_visuals(rims, lights);
    }
    /// At most 32 pairs of already-loaded base and transparent overlay sprites.
    /// Per-pixel alpha in the overlay marks full-bright material, without
    /// illuminating the surrounding block or changing gameplay light.
    pub(in crate::game) fn native_emissive(&self, layers: &mut Vec<(&'static str, &'static str)>) {
        self.fire_emissive(layers);
    }
    /// The world's phases, read-only; `error_text` carries the engine's
    /// own open-failure text, empty otherwise. Server-side. A server loop or final
    /// save failure fires `WorldCloseFailed` with the error text.
    pub(in crate::game) fn native_world_lifecycle(
        &mut self,
        context: &LifecycleContext,
        phase: WorldPhase,
        error_text: &str,
    ) {
        self.fire_world_lifecycle(context, phase, error_text);
    }
    /// After admission, collect once before building and sending this player's recipes.
    /// Reload asks once per online player before their recipes are re-sent; none when empty.
    pub(in crate::game) fn native_datapack_sync(
        &mut self,
        call: mod_api::__private::DatapackSyncCall<'_>,
    ) {
        self.fire_datapack_sync(call);
    }
    /// The server's recipes of `recipe_types` arrived (NeoForge's
    /// `RecipesReceivedEvent`): from now on the recipe helpers
    /// (`native_recipes`, `recipes_of_type`) give the server's recipes
    /// of those types in place of the window's own, so rebuild what you made
    /// of them. It comes after a join, once the window has its player, and
    /// after every `/reload`, on a server whose mods asked for them
    /// (`native_datapack_sync`), single player included; with no types when
    /// the server stopped sending them and the window is back on its own
    /// recipes. A recipe the
    /// window's own recipe book doesn't know has `book_id` -1. Client-side.
    pub(in crate::game) fn native_recipes_received(&mut self, recipe_types: &[String]) {
        self.fire_recipes_received(recipe_types);
    }
    /// The folders of JSON files native mods read from the data
    /// (`AddServerReloadListenersEvent`): push yours, each a listener id and a
    /// folder under `data/<namespace>/`. Asked at the start of every data load
    /// of a server: a world opening, `/reload`. A folder that leaves
    /// The server is assembling its data tables from the world's data packs
    /// (`ReloadableServerResources`): a world is opening, or `/reload` was
    /// run, and the recipes, tags, loot and advancements are built after
    /// this. Push `(path, bytes)` pack files and they join this load. `path`
    /// is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over
    /// the kinds the tables read: `recipe/mymod:thing`,
    /// `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`,
    /// `trade_set/…`, `villager_trade/…`, `dialog/…`. A file that isn't JSON
    /// is skipped with a warning. Asked once per load on both paths, so this
    /// is where a mod recomputes them.
    /// Contributed files sit below the world's own data packs: a pack's file
    /// for the same path still overrides a mod's. Server-side (the single
    /// player's integrated server included).
    pub(in crate::game) fn native_data_pack_files(&mut self, files: &mut Vec<(String, Vec<u8>)>) {
        self.fire_data_pack_files(files);
    }
    /// The server is assembling its data tables, as for
    /// `native_data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is an `id` (`mod_id:name`, not in
    /// `minecraft`) for a `registry`, `minecraft:item` or `minecraft:block`
    /// so far. Its values come from the world's data packs'
    /// `data/<mod_id>/data_maps/<registry path>/<name>.json`, and from files
    /// you push through `native_data_pack_files` under the key
    /// `data_maps/<mod_id>:<registry path>/<name>`: `replace`, `values`
    /// (entry ids and `#tags` to JSON values) and `remove`, a newer value
    /// overriding an older. Push a typed map's `DataMap::ty`
    /// and read its values with `ItemKey::data` and `BlockKey::data`. Server-side,
    /// not synced to clients.
    pub(in crate::game) fn native_data_map_types(&mut self, types: &mut Vec<DataMapType>) {
        self.fire_data_map_types(types);
    }
    /// The server is about to open its worlds: nothing of the world is
    /// loaded yet (NeoForge's `ServerAboutToStartEvent`, "called before the
    /// server begins loading anything"). Vanilla's `MinecraftServer.loadLevel`
    /// start. Server-side: the single player server thread once Play is
    /// pressed, a dedicated server once its settings are read. Each is
    /// followed by `native_server_started` or
    /// `native_world_lifecycle`'s `WorldOpenFailed`. `dedicated` tells a dedicated server from the single
    /// player one.
    pub(in crate::game) fn native_server_about_to_start(&mut self, dedicated: bool) {
        self.fire_server_about_to_start(dedicated);
    }
    /// Server startup has begun, before play can be accepted.
    pub(in crate::game) fn native_server_starting(&mut self, dedicated: bool) {
        self.fire_server_starting(dedicated);
    }
    /// Server teardown has finished, including the final save if possible.
    pub(in crate::game) fn native_server_stopped(&mut self, dedicated: bool) {
        self.fire_server_stopped(dedicated);
    }
    /// The server opened its worlds and is about to tick (NeoForge's
    /// `ServerStartedEvent`): every level loaded, no player joined. Where
    /// `WorldOpenReady` fired: the end of the open, before the tick loop.
    pub(in crate::game) fn native_server_started(&mut self, dedicated: bool) {
        self.fire_server_started(dedicated);
    }
    /// The server begins teardown (NeoForge's `ServerStoppingEvent`):
    /// the start of vanilla's `MinecraftServer.stopServer`, with the worlds
    /// still loaded and the players still connected and not yet saved, so a
    /// mod can save its own data or tell them. `native_world_lifecycle`'s
    /// `WorldCloseFinal` follows once the worlds are saved. A server whose loop
    /// fails still fires this before the final save attempt and `native_server_stopped`.
    pub(in crate::game) fn native_server_stopping(&mut self, dedicated: bool) {
        self.fire_server_stopping(dedicated);
    }
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Read blocks with `native_server_block`, by
    /// dimension: which level `self.world` holds here is not defined. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step.
    pub(in crate::game) fn native_server_tick(
        &mut self,
        context: &LifecycleContext,
        phase: ServerTickPhase,
    ) {
        self.fire_server_tick(context, phase);
    }
    /// Before an authoritative server level's tick pass. `dimension` is its ID.
    pub(in crate::game) fn native_level_tick_pre(&mut self, level: LevelRef) {
        self.fire_level_tick_pre(level);
    }
    /// After a successful authoritative server level tick pass.
    pub(in crate::game) fn native_level_tick_post(&mut self, level: LevelRef) {
        self.fire_level_tick_post(level);
    }
    /// Before one of vanilla's HUD layers is drawn (`RenderGuiLayerEvent.Pre`):
    /// draw here to put something below the layer, or set `canceled` to hide
    /// it. `layer` is the layer's id, in the order Horizon draws them:
    /// `minecraft:crosshair`, `hotbar`, `effects`, `boss_overlay`,
    /// `vehicle_health` (only riding a mount), `contextual_info_bar` (the
    /// mount's jump bar, the locator bar or the experience bar, with the level
    /// number), `player_health` (the health, armor, food and air rows, drawn in
    /// one pass), `selected_item_name`, `sleep_overlay`, `overlay_message`
    /// (the action bar), `title`, `scoreboard_sidebar`, `subtitle_overlay`,
    /// `chat` and `tab_list`. A layer F1 hides fires nothing, and the chat
    /// screen draws the focused chat itself, so `chat` is only the unfocused
    /// lines. The camera overlays and the debug screen are not layers here:
    /// they are not drawn in this pass. Draw with `native_fill`, `native_text`
    /// and the sprite helpers; `draw.size()` is the GUI
    /// size. A layer a mod registered (`native_register_gui_layers`) fires this
    /// and `native_gui_layer_post` too, with its own id, in its place among
    /// these.
    pub(in crate::game) fn native_gui_layer_pre(
        &self,
        layer: &str,
        draw: &mut HudBatch,
        canceled: &mut bool,
    ) {
        self.fire_gui_layer_pre(layer, draw, canceled);
    }
    /// After one of vanilla's HUD layers (or after the layer a mod hid), with
    /// the ids of `native_gui_layer_pre`: draw here to put something above
    /// the layer (`RenderGuiLayerEvent.Post`).
    pub(in crate::game) fn native_gui_layer_post(&self, layer: &str, draw: &mut HudBatch) {
        self.fire_gui_layer_post(layer, draw);
    }
    /// `Player.interactOn`: both logical sides answer before entity interaction.
    /// Only the client's own consuming answer stops its hand loop.
    pub(in crate::game) fn native_entity_interact(
        &mut self,
        hand: Hand,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        result: &mut Option<mod_api::InteractionResult>,
    ) {
        self.fire_entity_interact(hand, entity, kind, stack, result);
    }
    /// A player left-clicks the block `state` at `pos`, `face` the face hit
    /// (`[0, 1, 0]` is the top): where `ServerPlayerGameMode.handleBlockBreakAction`
    /// takes a dig's start, after its reach, spawn protection and world border
    /// checks, before `Block.attack` (a note block plays) or a creative break.
    /// Set `canceled` to ignore the click: the block isn't attacked, dug or
    /// broken on the server, and the player's client gets it back (its own
    /// predicted crack still shows to the clicking player). Server-side, for
    /// every player of the world (the helpers read the acting one).
    pub(in crate::game) fn native_left_click_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        self.fire_left_click_block(pos, face, state, canceled);
    }
    /// NeoForge's non-cancellable PlayerTickEvent.Pre on either logical side.
    pub(in crate::game) fn native_player_tick_pre(
        &mut self,
        call: mod_api::__private::PlayerTickPreCall,
    ) {
        self.fire_player_tick_pre(call);
    }
    /// NeoForge's non-cancellable PlayerTickEvent.Post on either logical side.
    pub(in crate::game) fn native_player_tick_post(
        &mut self,
        call: mod_api::__private::PlayerTickPostCall,
    ) {
        self.fire_player_tick_post(call);
    }
    pub(in crate::game) fn native_health_hud(
        &mut self,
        inputs: &mod_api::client::hud_layout::HealthInputs,
        wi: i32,
        hi: i32,
        draws: &mut Option<Vec<GuiDraw>>,
    ) {
        self.fire_health_hud(inputs, wi, hi, draws);
    }
    /// Raw mouse button input on any screen, before dispatch; cancel skips dispatch and Post.
    /// Button/modifiers use the same GLFW encoding as screen hooks. Synthetic clicks skip it.
    pub(in crate::game) fn native_mouse_button_pre(
        &mut self,
        button: i32,
        pressed: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_mouse_button_pre(button, pressed, modifiers, canceled);
    }
    /// An accepted raw button event finished dispatch, including a screen handler's early return.
    pub(in crate::game) fn native_mouse_button_post(
        &mut self,
        button: i32,
        pressed: bool,
        modifiers: u32,
    ) {
        self.fire_mouse_button_post(button, pressed, modifiers);
    }
    /// Gameplay scrolling after accumulation, before hotbar/spectator handling.
    /// `scroll` is scaled x/y, `accumulated` whole x/y steps, `buttons` left/middle/right,
    /// and `cursor` the GUI position. Cancel skips gameplay scrolling.
    pub(in crate::game) fn native_mouse_scrolling(
        &mut self,
        scroll: [f64; 2],
        accumulated: [i32; 2],
        buttons: [bool; 3],
        cursor: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_mouse_scrolling(scroll, accumulated, buttons, cursor, modifiers, canceled);
    }
    /// A command typed or clicked in chat, without its slash, before the
    /// server and Horizon's commands see it: set `ran` when the mod ran it.
    pub(in crate::game) fn native_command(&mut self, command: &str, ran: &mut bool) {
        self.fire_command(command, ran);
    }
    /// The client's own commands (`RegisterClientCommandsEvent`), which
    /// `native_command` runs: push a `mod_api::Command` per usage. Asked when a
    /// connection starts and again when the server sends its own tree, so the
    /// chat box colours, completes and lists them as the game's own; a name
    /// the game already has keeps that command's behaviour. A client command
    /// has no operator level: one with a nonzero level is left out.
    pub(in crate::game) fn native_register_client_commands(
        &mut self,
        commands: &mut Vec<mod_api::Command>,
    ) {
        self.fire_register_client_commands(commands);
    }
    /// World field of view, after normal calculation and zoom and before projection/Nausea.
    /// Called for world rendering and near-plane overlays; hand/HUD FOV uses its own path.
    /// Only finite values in 1..=179 degrees replace the computed FOV.
    pub(in crate::game) fn native_compute_fov(
        &self,
        partial: f32,
        camera: [f64; 3],
        degrees: &mut f32,
    ) {
        self.fire_compute_fov(partial, camera, degrees);
    }
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    pub(in crate::game) fn native_zoom(&self, zoom: &mut Option<mod_api::Zoom>) {
        self.fire_zoom(zoom);
    }
    /// Computed world fog RGB, after ordinary effects and before upload. Title panoramas skip it.
    /// Only finite RGB components in 0..=1 are accepted; alpha and distances stay unchanged.
    pub(in crate::game) fn native_compute_fog_color(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: mod_api::client::fog::FogType,
        rgb: &mut [f32; 3],
    ) {
        self.fire_compute_fog_color(partial, camera, fog_type, rgb);
    }
    /// Final world fog, after the computed color hook, ordinary calculations and Horizon's fog
    /// option, before upload. Title panoramas skip it. Invalid color or distance values retain the
    /// original whole fog.
    pub(in crate::game) fn native_render_fog(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: mod_api::client::fog::FogType,
        fog: &mut mod_api::client::fog::FogData,
    ) {
        self.fire_render_fog(partial, camera, fog_type, fog);
    }
    /// The camera shake native mods ask for in this frame, `partial` ticks
    /// past the last tick (0 to 1): add your pitch, yaw and roll in degrees
    /// and your offset in blocks, along the camera's right, up and back, to
    /// `shake`, so several mods' shakes add up. Read while playing only;
    /// Horizon scales the sum by Distortion Effects and bounds it
    /// (`native_shake_matrix`).
    pub(in crate::game) fn native_camera_shake(&self, partial: f32, shake: &mut Shake) {
        self.fire_camera_shake(partial, shake);
    }
    /// Final scaled third-person camera distance, before visual-shape collision clipping.
    /// `entity` includes its living scale/distance and optional mount scale/distance.
    /// Only finite requests in 0..=1024 blocks replace the vanilla distance.
    pub(in crate::game) fn native_camera_distance(
        &self,
        partial: f32,
        mirrored: bool,
        entity: &mod_api::client::camera::CameraEntity,
        distance: &mut f32,
    ) {
        self.fire_camera_distance(partial, mirrored, entity, distance);
    }
    /// Change the colours of `biome` (`minecraft:plains`) on this client: the
    /// grass, foliage (leaves, vines), dry foliage (leaf litter) and water
    /// tints, 0xRRGGBB, as its biome file and the colormaps set them. Asked
    /// for every biome when a mod calls `refresh_biome_colors`, which
    /// rebuilds the terrain with them; a season mod answers from its season
    /// and calls that when the season turns. The dark forest's modifier
    /// still applies to the grass set here; the swamp's noise colours
    /// replace it.
    pub(in crate::game) fn native_biome_colors(&self, biome: &str, colors: &mut BiomeColors) {
        self.fire_biome_colors(biome, colors);
    }
    /// Choose this client's music, once a client tick: `music` and `volume`
    /// hold what vanilla chose (`Minecraft.getSituationalMusic` and
    /// `getMusicVolume`: the biome's track, the creative or underwater one, a
    /// boss's, the menu's; None for silence). Set `music` to a track of your
    /// own, a sound event of your content mod's `sounds.json` such as
    /// `Music::new("mymod:music.spring", 1200, 6000, false)` (the delays in
    /// ticks between songs, and whether it cuts off a different song playing),
    /// None to keep vanilla's quiet, or change `volume` (0 to 1) to fade it.
    pub(in crate::game) fn native_music(&self, music: &mut Option<Music>, volume: &mut f32) {
        self.fire_music(music, volume);
    }
    /// The tracks to play over (or instead of) the music on this client,
    /// asked once a client tick: push a `MusicLayer` (a sound event, looped,
    /// and its volume, 0 to 1) for each. A layer eases to a new volume over up
    /// to two seconds, and one left out fades out and stops; layers that
    /// first appear in the same tick start together, so list every stem of a
    /// piece from the start, the silent ones at volume 0, and raise them (a
    /// fuller arrangement as a village grows). Their sound events should be
    /// `stream: true` in `sounds.json`.
    pub(in crate::game) fn native_music_layers(&self, layers: &mut Vec<MusicLayer>) {
        self.fire_music_layers(layers);
    }
    /// The values native mods hand their shader packs in this frame,
    /// `partial` ticks past the last tick (0 to 1): a pack's WGSL reads
    /// them as `frame.mods` (eight `vec4<f32>`). Write the slots your own
    /// pack reads; a mod that bundles its pack as `native-<mod id>` is the
    /// one that pack is for. Asked once a frame while a pack is on; a value
    /// that isn't finite reads as 0 (`native_shader_params_now`).
    pub(in crate::game) fn native_shader_params(&self, partial: f32, params: &mut [[f32; 4]; 8]) {
        self.fire_shader_params(partial, params);
    }
    /// The minimap a native mod shows: set `minimap` when it is none yet.
    pub(in crate::game) fn native_minimap(&self, minimap: &mut Option<mod_api::Minimap>) {
        self.fire_minimap(minimap);
    }
    /// Register namespaced NBT component types before content loads.
    pub(in crate::game) fn native_component_types(
        context: &mod_api::RegistrationContext,
        types: &mut Vec<String>,
    ) {
        Self::fire_component_types(context, types);
    }
    /// Blocks this mod adds, declared in code rather than one file each
    /// (NeoForge's `DeferredRegister.Blocks` drained by `RegisterEvent`):
    /// push a `mod_api::Block` per block, the same declaration
    /// a content mod's `data/<id>/horizon/block/<path>.json` holds (a
    /// vanilla `template`, `properties`, `state_properties`, `shapes`...),
    /// with its id in your mod's namespace. It has a block item, as a file's
    /// does, unless `item` is false (`Block::with_item(false)`). They join the content mods' blocks: the one whose
    /// id is that namespace when it is on (your models, textures, loot and
    /// recipes are its files), else one of their own, with the template's
    /// look. Ids, saves and the check that a joining client has the same
    /// blocks are the files' (docs/modding/src/how-to/content-mods.md). Asked once at start, on every
    /// side, before any resource loads: push the same ones every time, from
    /// nothing but your code. Declarations that don't pass the files' checks
    /// are logged and that namespace's are left out.
    pub(in crate::game) fn native_blocks(
        context: &mod_api::RegistrationContext,
        blocks: &mut Vec<mod_api::Block>,
    ) {
        Self::fire_blocks(context, blocks);
    }
    /// Items this mod adds, declared in code (NeoForge's
    /// `DeferredRegister.Items`), as `native_blocks` declares blocks: a
    /// `mod_api::Item` per item, a content mod's
    /// `data/<id>/horizon/item/<path>.json` (a vanilla `template`,
    /// `max_stack_size`, `rarity`, `components`, `crafting_remainder`). A
    /// block's item comes with its block (`item: true`), not from here.
    /// Asked once at start, on every side.
    pub(in crate::game) fn native_items(
        context: &mod_api::RegistrationContext,
        items: &mut Vec<mod_api::Item>,
    ) {
        Self::fire_items(context, items);
    }
    /// File-backed standalone models and bake-time forms, registered before resource loading.
    /// Every resource load rebakes them; dispatch attributes each form to its owning mod.
    pub(in crate::game) fn native_standalone_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<String>,
        forms: &mut Vec<mod_api::Answered<mod_api::StandaloneModelForm>>,
    ) {
        Self::fire_standalone_models(context, models, forms);
    }
    /// The player changed setting `key` of mod `mod_id` on its page in Options > Mods; the new
    /// value is kept already.
    pub(in crate::game) fn native_setting_changed(&mut self, mod_id: &str, key: &str) {
        self.fire_setting_changed(mod_id, key);
    }
    /// The player did `event` on mod `mod_id`'s panel (`panel_items`):
    /// act on it when it is yours.
    pub(in crate::game) fn native_panel_event(&mut self, mod_id: &str, event: &PanelEvent) {
        self.fire_panel_event(mod_id, event);
    }
    /// Whether block `block`'s block entity ticks (`EntityBlock.getTicker`):
    /// set `ticks` when it is yours. Asked for each of the block's states
    /// when a world opens.
    /// Set `ticks` to true to make it tick, and never clear a value an earlier mod or mixin set.
    pub(in crate::game) fn native_block_entity_ticks(&self, block: &str, ticks: &mut bool) {
        self.fire_block_entity_ticks(block, ticks);
    }
    /// A `minecraft:post_attack` effect of a mod's own kind, on a mob a
    /// player's weapon hit: the entry says `enchanted` attacker and `affected`
    /// victim, its `requirements` passed, and its `effect` names a `type` in a
    /// namespace outside `minecraft:`. Told the enchantment id, the kind, the
    /// `effect` payload as a borrowed `Tag`, the level and the mob's id (it may have just died). The
    /// acting player is the one whose weapon carries the enchantment. Each
    /// such entry is told once a hit, and once for each mob a sweep lands on.
    /// A hit on a player, and an entry inside `minecraft:all_of`, aren't told
    /// yet. An unclaimed kind does nothing.
    pub(in crate::game) fn native_enchant_effect(
        &mut self,
        enchantment: &str,
        kind: &str,
        data: &horizon_core::nbt::Tag,
        level: u32,
        victim: u32,
    ) {
        self.fire_enchant_effect(enchantment, kind, data, level, victim);
    }
    /// Block `block`'s registered item slots (`WorldlyContainer`), read from
    /// its per-block declaration when the ticking table is built.
    /// At most 256 slots, with slot numbers past the count left out.
    /// Its menu must be one of the supported menus:
    /// `containers::MOD_MENUS` or a content mod's menu type
    /// (`data/<id>/horizon/menu/`) with as many
    /// slots, opens when the block is used (its template must open a menu, as
    /// a chest or a furnace does); another is left out. A mod menu type shows
    /// on Horizon clients through the mod's `native_screen` only.
    pub(in crate::game) fn native_block_entity_container(
        &self,
        block: &str,
        container: &mut Option<ModContainer>,
    ) {
        *container = mod_api::__private::parts_installation::block_container(block);
    }
    /// Block `block`'s registered fluid tanks: each tank's capacity in
    /// millibuckets, the tanks each face reaches, and the tanks a fill may
    /// reach and a drain may take. Read from its per-block declaration when the
    /// ticking table is built; at most 16 tanks, capacities clamped to
    /// 1..=1 000 000, and tank numbers past the count are left out. The tanks'
    /// contents are kept in the entity's saved compound, under `fluids`, and
    /// the fluid helpers below read and write them.
    pub(in crate::game) fn native_block_entity_fluids(
        &self,
        block: &str,
        fluids: &mut Option<ModFluidTanks>,
    ) {
        *fluids = mod_api::__private::parts_installation::block_fluids(block);
    }
    /// Block `block`'s registered energy buffer (`Capabilities.Energy.BLOCK`):
    /// its capacity, the most one insert and one extract moves, and the faces
    /// a fill may come through and a drain may leave by.
    /// Read from its per-block declaration when the ticking
    /// table is built; a capacity under 1 leaves the buffer out, and the
    /// limits are clamped to 0..=capacity. The amount is kept in the entity's
    /// saved compound, as the int `energy`, and the energy helpers below read
    /// and write it.
    pub(in crate::game) fn native_block_entity_energy(
        &self,
        block: &str,
        energy: &mut Option<ModEnergyBuffer>,
    ) {
        *energy = mod_api::__private::parts_installation::block_energy(block);
    }
    /// A consumable's effect of a mod's own kind: the item's `on_consume_effects`
    /// entry is of a type in a namespace of its own, and the loader kept it
    /// for this call. The kind and the definition payload as a borrowed `Tag` are here; the acting
    /// player is the one consuming, and the game's own effects,
    /// sounds, teleports and drops are open. An unclaimed kind does
    /// nothing.
    pub(in crate::game) fn native_consume_effect(
        &mut self,
        kind: &str,
        data: &horizon_core::nbt::Tag,
    ) {
        self.fire_consume_effect(kind, data);
    }
    /// What clients get of the block entity at `pos` when its `id` is
    /// yours (`BlockEntity.getUpdateTag`): set `tag` to it, read from
    /// `entity` alone. Asked on the server whenever a chunk holding it is
    /// sent and whenever its block changes, on the server thread or, for a
    /// saved chunk sent past the view distance (`lan::far`), another one; a client's copy of the block
    /// entity (made by `native_block_entity` there) merges it into its `nbt`.
    /// Leave it unset to send no tag of yours. A block change puts the one-shot fields of
    /// `ModBlockEntityView::set_client_update_fields` over it, and sends them alone when it is unset.
    pub(in crate::game) fn native_block_entity_update_tag(
        pos: [i32; 3],
        entity: &ModBlockEntity,
        tag: &mut Option<Compound>,
    ) {
        Self::fire_block_entity_update_tag(pos, entity, tag);
    }
    /// Draw the mod block entity at `pos`, of block `block`
    /// (`mymod:machine`), on this client: once a frame while it is in view
    /// (vanilla's `BlockEntityRenderer.submit`: within its view distance, 64 blocks unless
    /// `block_entity_view_distance` says otherwise, its chunk
    /// faded in, and its bounds, `native_block_entity_render_bounding_box`,
    /// in the frustum), when its `id` is yours. `state` is the block's state
    /// (`blockEntity.getBlockState()`: read its `facing` with
    /// `block_state_property`), `entity` holds what the server sent
    /// (`native_block_entity_update_tag`). Add geometry with
    /// `native_draw_quad`, `native_draw_cube`, `native_draw_block` and
    /// `native_draw_item` under `draw`'s pose stack, which starts at the
    /// block's lower north-west corner, in blocks; `draw` also has the partial
    /// tick, the game time and the block's light.
    pub(in crate::game) fn native_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: BlockStateId,
        entity: &ModBlockEntity,
        draw: &mut BlockEntityDraw,
    ) {
        self.fire_block_entity_submit(pos, block, state, entity, draw);
    }
    /// Where the mod block entity at `pos` draws, for culling
    /// (NeoForge's `getRenderBoundingBox`): `bounds` is min x, y, z then max
    /// x, y, z, in blocks from the block's lower north-west corner. It starts
    /// as the block with a block of margin around it; widen it when your
    /// `native_block_entity_submit` draws further (a tall shaft, a beam), or
    /// set an infinite value for one never culled. Asked once a frame for each
    /// within its view distance (`block_entity_view_distance`) whose chunk is drawn.
    pub(in crate::game) fn native_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: BlockStateId,
        entity: &ModBlockEntity,
        bounds: &mut [f32; 6],
    ) {
        self.fire_block_entity_render_bounding_box(pos, block, state, entity, bounds);
    }
    /// Icons a mod puts on map `map` each time it is drawn: held, in an
    /// item frame or on the cartography table's screen (a map's pixels are
    /// its own; nothing is saved in the map). Push `mod_api::MapDecoration`s
    /// (an atlas sprite, a position in half map pixels, a rotation, and
    /// whether it shows in a frame and on the screen like vanilla's markers
    /// do); they are drawn after the vanilla ones with the same pose, and a
    /// sprite the atlas lacks is left out. `map.center`, `map.scale` and
    /// `map.dimension` say which blocks the map shows, and
    /// `mod_api::MapDecoration::x` has the formula. Vanilla's decorations are
    /// not passed in. Asked every frame a map is drawn, so keep it cheap.
    /// The client half of `MapDecorationType` and `addDecoration`: a type
    /// registered on the server and saved in the map's data is not here.
    pub(in crate::game) fn native_map_decorations(
        &self,
        map: &mod_api::MapInfo<'_>,
        decorations: &mut Vec<mod_api::MapDecoration>,
    ) {
        self.fire_map_decorations(map, decorations);
    }
    /// A menu opens on this client for block `block`, a mod's: set `screen`
    /// when the block is yours, to lay its screen out (`native_screen_draw`
    /// draws it). The server asks it too: a slot it hides is not in the menu
    /// there either, so shift-clicks never fill it.
    pub(in crate::game) fn native_screen(&self, block: &str, screen: &mut Option<mod_api::Screen>) {
        self.fire_screen(block, screen);
    }
    /// Draw block `block`'s screen at `origin` (its top left, GUI pixels),
    /// over a plain panel with its slots' frames and under the items and
    /// labels, from the menu's data slots `data` (the block entity's `data`
    /// on the server): data slots are 16-bit on the wire as in vanilla (a
    /// bigger value wraps), and `data` is empty until the server's first
    /// values arrive, so read it with `get`. Draw with `native_sprite`, `native_fill` and
    /// `native_text`.
    pub(in crate::game) fn native_screen_draw(
        &self,
        block: &str,
        data: &[i32],
        draw: &mut HudBatch,
        origin: [f32; 2],
    ) {
        self.fire_screen_draw(block, data, draw, origin);
    }
    /// The open screen has drawn itself and its tooltips (`Screen.render`,
    /// the point of `ScreenEvent.Render.Post`): draw over it with
    /// `native_fill`, `native_sprite` and `native_text`, `screen.rect` being
    /// its container panel (or the whole window) and `screen.id` the screen
    /// it is. Once a frame on the window, never while playing with no screen;
    /// `&self`, so it reads and draws only. The Win and Options screens call
    /// it on their own path, after their drawing.
    ///
    /// It does not fire for the screens drawn over gameplay after it (a
    /// server dialog, the server pack, link and code-of-conduct prompts, a
    /// book, a sign editor): those keep `Screen::Playing`, which it skips (the
    /// server pack prompt hides an open screen too, and that screen does not
    /// fire under it), and draw over what it drew, while vanilla's
    /// `ScreenEvent.Render.Post` fires for them. The title's own screens
    /// (`title`, `world_select`, `world_create`, `multiplayer`…) fire it too,
    /// their `screen.id` from the title state machine and the whole window
    /// their `rect`, and no dialog or prompt over them.
    pub(in crate::game) fn native_screen_render_post(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
    ) {
        self.fire_screen_render_post(draw, screen);
    }
    /// A mouse button went down over the open screen
    /// (`ScreenEvent.MouseButtonPressed.Pre`, vanilla's
    /// `Screen.mouseClicked`): `screen` is the screen it is, `screen.mouse`
    /// where the pointer is (so there is no position of its own), `button` the
    /// button's GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; a
    /// button past those is the platform's own number, which differs between
    /// systems) and `modifiers` GLFW's bits (shift 1, control 2,
    /// alt 4, super 8). Set `canceled` and the press goes no further: no slot,
    /// no screen button, no drag, no hotbar key, no sort or header button.
    ///
    /// It fires before anything the screen does with the press, on the window
    /// only, and while a screen of the game's or the title's is open (Options
    /// and the pack list opened from the title screen count, with no player
    /// and no `menu`; the title's own screens, their
    /// `screen.id` from the title state machine). It does not fire on the
    /// connect and loading screens, nor for a server
    /// dialog, the server pack, link and code-of-conduct prompts, a book or a
    /// sign editor drawn over gameplay, nor while a world loads or saves: the
    /// click there stays the game's own. A gamepad click or a click the
    /// keyboard's focus sends is not the mouse and fires nothing.
    pub(in crate::game) fn native_screen_mouse_button_pressed_pre(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_button_pressed_pre(screen, button, modifiers, canceled);
    }
    /// A mouse button went up over the open screen
    /// (`ScreenEvent.MouseButtonReleased.Pre`, vanilla's
    /// `Screen.mouseReleased`), with the same `screen`, `button` and
    /// `modifiers` as `native_screen_mouse_button_pressed_pre` and fired at
    /// the same point. Set `canceled` and the release goes no further: a
    /// pressed slot keeps what it took and no drag, slider or scrollbar lets
    /// go.
    ///
    /// Cancel a release only when the press it ends was cancelled too: a
    /// release the screen took ends the drag or the scroll it began, as it
    /// does in vanilla, and no guard here un-cancels it. Same screens as the
    /// press; a press that opened another screen ends on that one.
    pub(in crate::game) fn native_screen_mouse_button_released_pre(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_button_released_pre(screen, button, modifiers, canceled);
    }
    /// Before a real held-button movement reaches screen drag handlers.
    /// `screen.mouse` is the current GUI position; `delta` is its GUI-space movement.
    /// Cancel keeps the new cursor position but skips all screen drag handlers.
    pub(in crate::game) fn native_screen_mouse_dragged_pre(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_dragged_pre(screen, button, delta, modifiers, canceled);
    }
    /// The wheel turned over the open screen (`ScreenEvent.MouseScrolled.Pre`,
    /// vanilla's `Screen.mouseScrolled`): `screen` is the screen it is,
    /// `screen.mouse` where the pointer is (so there is no position of its
    /// own), `scroll` the wheel's movement in x and y as the game's own scroll
    /// handler scales it (a line times the wheel sensitivity: with Discrete
    /// Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are
    /// divided by 30), and
    /// `modifiers` GLFW's bits (shift 1, control 2, alt 4, super 8). Set
    /// `canceled` and nothing the screen would scroll moves: not the bundle
    /// under the pointer, nor a list, slider, scrollbar or map. Nothing else
    /// reads the wheel while a screen of the game's is open, so no hotbar slot
    /// changes either.
    ///
    /// It fires before the screen scrolls anything, for the window's wheel
    /// (horizon-ctl's `scroll-at` sends it through the window too), and
    /// while a screen of the game's or the title's is open (Options and the
    /// pack list opened from the title screen count, with no player and no
    /// `menu`; the title's own). It
    /// does not fire on the connect and loading
    /// screens, nor for a server dialog, the server pack, link and
    /// code-of-conduct prompts, a book or a sign editor drawn over gameplay,
    /// nor while a world loads or saves: the scroll there stays the game's
    /// own. A wheel a gamepad or the keyboard's focus sends is not the mouse
    /// and fires nothing.
    pub(in crate::game) fn native_screen_mouse_scrolled_pre(
        &self,
        screen: &ScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_scrolled_pre(screen, scroll, modifiers, canceled);
    }
    /// A key went down over the open screen (`ScreenEvent.KeyPressed.Pre`,
    /// vanilla's `Screen.keyPressed`): `screen` is the screen it is, `key`
    /// vanilla's name for the key (`key.keyboard.e`) — a key with no name
    /// fires nothing — `repeat` whether it is
    /// the keyboard's auto-repeat of a held press (Horizon's extra; NeoForge
    /// folds repeats into the event), and `modifiers` GLFW's bits (shift 1,
    /// control 2, alt 4, super 8). Set `canceled` and the press goes no
    /// further: no focus move, no Escape-close, no inventory or hotbar key, no
    /// typing by the key itself.
    ///
    /// It fires for the window's presses and their repeats, and for the
    /// control socket's `key` (a release fires nothing), after the window's
    /// own shortcuts have run — the fullscreen and screenshot keys, the debug
    /// keys and the narrator — and while a screen of the game's or the title's
    /// is open (Options and the pack list opened from the title screen count,
    /// with no player and no `menu`; the title's own). It does
    /// not fire on the connect and loading screens, nor for a server dialog, the server pack,
    /// link and code-of-conduct prompts, a book or a sign editor drawn over
    /// gameplay, nor while a world loads or saves: the key there stays the
    /// game's own. A cancelled press's release still runs, and its characters
    /// are GLFW's separate char callback and still come; see
    /// `native_screen_char_typed_pre`.
    pub(in crate::game) fn native_screen_key_pressed_pre(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_pressed_pre(screen, key, repeat, modifiers, canceled);
    }
    /// Before an open screen handles a named key's release (same screen gate as key presses).
    /// Cancel bypasses screen handling; physical keys and mapped keyboard hold/toggle state release.
    pub(in crate::game) fn native_screen_key_released_pre(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_released_pre(screen, key, modifiers, canceled);
    }
    /// A character was typed at the open screen
    /// (`ScreenEvent.CharacterTyped.Pre`, vanilla's `Screen.charTyped`):
    /// `screen` is the screen it is, `c` the character, `modifiers` GLFW's
    /// bits (shift 1, control 2, alt 4, super 8), and `field` a copy of the
    /// focused text box. Set `canceled` and that one character never reaches
    /// a text box.
    ///
    /// `field` is `None` with no box focused, and for the boxes that keep
    /// their own validators: Options' and World Options' fields, the game
    /// rules' and the block editor's. `cursor` and `highlight` count Unicode
    /// scalar characters, and a character sees the box as the one before it
    /// left it. Change `value`, `cursor` or `highlight` and the box takes the
    /// change, cancelled or not, so cancel to replace the character yourself.
    /// New text goes through the box's own filter and length limit, and what
    /// follows the box (chat suggestions, a search's results, the anvil's
    /// name) follows it. `max_length`, `multiline` and whether `field` is
    /// `None` stay the box's: a change to them does nothing.
    ///
    /// It fires once per printable character of a key press's text and once
    /// per character of the input method's committed text, never for a control
    /// character (Enter, Backspace, Tab, Delete, Escape, a Control letter):
    /// those are keys, and the game reads them as keys, not as text. A key's
    /// characters fire after the key has acted, as GLFW's char callback comes
    /// after its key callback (`KeyboardHandler.charTyped`): `screen` is the
    /// screen open then, so a key that closes the screen (the inventory key)
    /// leaves none to hear its character, where vanilla's closed screen hears
    /// nothing, and one that moves the focus types into the box it moved to.
    /// They fire even when `native_screen_key_pressed_pre` cancelled the press
    /// that carried the text, as NeoForge delivers `CharacterTyped.Pre` for a
    /// cancelled `KeyPressed.Pre`: the characters left reach the text box
    /// without the key's own effects. Cancelling one character drops only that
    /// one. The control socket's `type` fires it too, but not for its `\b`
    /// backspace. Same screens as the key press, and it fires with no text box
    /// focused (there is nothing to take the character then).
    pub(in crate::game) fn native_screen_char_typed_pre(
        &self,
        screen: &ScreenView<'_>,
        c: char,
        modifiers: u32,
        field: &mut Option<mod_api::client::widgets::TextEdit>,
        canceled: &mut bool,
    ) {
        self.fire_screen_char_typed_pre(screen, c, modifiers, field, canceled);
    }
    /// A left or right (`right`) press at `at` (GUI pixels from the screen's
    /// top left, so outside it too) on block `block`'s screen, a mod's, over
    /// no slot: set `button` to a menu button id when it is on one of yours.
    /// Horizon sends it to the server (`ContainerButtonClick`,
    /// `native_menu_button` there) and the press goes no further.
    pub(in crate::game) fn native_screen_click(
        &self,
        block: &str,
        data: &[i32],
        at: [f32; 2],
        right: bool,
        button: &mut Option<i32>,
    ) {
        self.fire_screen_click(block, data, at, right, button);
    }
    /// The tooltip at `at` (as in `native_screen_click`) on block `block`'s
    /// screen, a mod's, over no item: push its lines when it is over
    /// something of yours. Legacy colour codes (§) work.
    pub(in crate::game) fn native_screen_tooltip(
        &self,
        block: &str,
        data: &[i32],
        at: [f32; 2],
        lines: &mut Vec<String>,
    ) {
        self.fire_screen_tooltip(block, data, at, lines);
    }
    /// A player pressed a mod container's button (`clickMenuButton`), on
    /// the server. The block entity is out of the world during the call;
    /// mods may change it and the clicking player's carried stack.
    pub(in crate::game) fn native_menu_button(
        &mut self,
        context: &mod_api::item::menu::MenuButtonContext,
        entity: &mut ModBlockEntity,
        carried: &mut Option<mod_api::ItemStack>,
    ) {
        self.fire_menu_button(context, entity, carried);
    }
    /// A player shift-clicked slot `slot` of the menu of the mod block
    /// entity at `pos` (`AbstractContainerMenu.quickMoveStack`, the method a
    /// Java menu overrides), on the server, before the game's own routing:
    /// move the stack where your block wants it. `slot` is the menu's index:
    /// below `container_slots` it is the block's own slot (`entity.items`),
    /// from there on the player's inventory (27 main slots, then the hotbar).
    /// `stack` is a copy of what the slot holds; take what you move out of
    /// it (put it in `entity.items`, or in another block through its
    /// `ITEMS` capability) and set `handled`: the slot then holds what is
    /// left of `stack` (whatever you wrote to that slot of `entity.items`),
    /// the game's routing is skipped, and the player's client gets the menu
    /// again. Left unhandled, `entity.items` goes back to what it held and
    /// the game moves the stack as for any container; a later mod sees what
    /// an earlier one did to `stack`.
    ///
    /// For a mod container on a vanilla menu type and on a mod menu type,
    /// the left or right button, a slot that holds something and that the
    /// mod's screen shows; not for a spectator or a dead player, a menu no
    /// longer valid, nor during a drag.
    /// Asked once per click, where vanilla asks `quickMoveStack` again while
    /// it keeps moving the same item. The player's state is swapped in, so
    /// `native_acting_player` names them; `entity` is out of the world
    /// during the call, as in `native_menu_button`.
    pub(in crate::game) fn native_menu_quick_move(
        &mut self,
        pos: [i32; 3],
        entity: &mut ModBlockEntity,
        container_slots: usize,
        slot: usize,
        stack: &mut mod_api::ItemStack,
        handled: &mut bool,
    ) {
        self.fire_menu_quick_move(pos, entity, container_slots, slot, stack, handled);
    }
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked once when a world opens to players (a dedicated
    /// server, a LAN host, single player's server thread); every player's
    /// chat box then completes them as vanilla's, within their level. A
    /// name vanilla has is left out. Asked again after a mod calls
    /// `native_refresh_commands`. API 8.
    pub(in crate::game) fn native_register_commands(
        &mut self,
        commands: &mut Vec<mod_api::Command>,
    ) {
        self.fire_register_commands(commands);
    }
    /// Execute a parsed mod command with its origin and effective permission.
    pub(in crate::game) fn native_run_mod_command(
        &mut self,
        source: &CommandSource,
        command: &CommandInvocation,
        ran: &mut bool,
    ) {
        self.fire_run_mod_command(source, command, ran);
    }
    /// A player takes an item from an armor stand or puts one on it
    /// (`ArmorStand.interactAt`, after the stand's disabled-slot checks and
    /// `swapItem`'s take/put locks, before anything moves): `stand_id` is the
    /// stand's `Entity.getId` and `slot` the one the click reached
    /// (`mainhand`, `offhand`, `feet`, `legs`, `chest` or `head`). Set
    /// `cancel` and nothing changes: neither the stand nor the player's hand,
    /// and the click is spent. The player's state is swapped in for the call;
    /// `player` is the UUID the helpers take. Not told of a click that changes
    /// nothing (an empty hand on an empty slot). Not covered, other ways a
    /// stand's equipment moves: breaking or blasting the stand (its items
    /// drop), a dispenser putting armour on one, a mod's own changes.
    /// Server-side, for every player of the world.
    pub(in crate::game) fn native_armor_stand_manipulate(
        &mut self,
        player: u128,
        name: &str,
        stand_id: i32,
        slot: &str,
        cancel: &mut bool,
    ) {
        self.fire_armor_stand_manipulate(player, name, stand_id, slot, cancel);
    }
    /// A boat or minecart (`kind`, `minecraft:oak_boat`) moved since the last
    /// tick (`VehicleMoveEvent`): `from` and `to` are its positions, heard
    /// after the tick moved it. Once a tick for each vehicle that carries a
    /// server player, whose state is then swapped in (as for
    /// `native_run_mod_command`: `native_acting_player`, `native_tell` and
    /// the item helpers reach them; `player` is their UUID), and for one a
    /// server player rode and left, until it has stood still for ten
    /// seconds (`player` is none, nobody is swapped in). It only hears.
    /// Cheap when nothing moves: one position compare a tick for each watched
    /// vehicle. Not heard: a vehicle moved before anyone rode it, a mount
    /// (horse, pig...) or a vehicle carrying only a mob. Server-side, in the
    /// level the vehicle is in.
    pub(in crate::game) fn native_vehicle_moved(
        &mut self,
        player: Option<u128>,
        kind: &str,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        self.fire_vehicle_moved(player, kind, from, to);
    }
    /// A player opens one of the game's own menus (`ServerPlayer.openMenu`; the
    /// job of NeoForge's `PlayerContainerEvent.Open`, which comes after the
    /// menu is open and cannot stop it): `kind` is the menu type
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:crafting`,
    /// `minecraft:merchant`, `minecraft:anvil`, `minecraft:lectern`...; a
    /// horse's inventory is `minecraft:horse_inventory`) and `pos` the block
    /// it belongs to, none for a villager, a mount or a vehicle's chest.
    /// Heard after the checks that refuse the menu (a locked container, a
    /// spectator) and before the menu that was open closes. Set `cancel` and
    /// nothing opens: the open menu stays as it was and the click is spent.
    /// The player's state is swapped in for the call, as for
    /// `native_run_mod_command`, so `native_acting_player`, `native_tell` and
    /// the item helpers reach them. Not heard: a menu a mod opens
    /// (`open_menu`), the inventory screen, and the editors (sign,
    /// book, command block). Server-side, for every player of the world.
    pub(in crate::game) fn native_container_opened(
        &mut self,
        player: u128,
        name: &str,
        kind: &str,
        pos: Option<[i32; 3]>,
        cancel: &mut bool,
    ) {
        self.fire_container_opened(player, name, kind, pos, cancel);
    }
    /// A creative-mode player sets a slot of their inventory from the
    /// creative screen (`ServerGamePacketListenerImpl.handleSetCreativeModeSlot`):
    /// `slot` is the inventory menu's slot (5 to 8 armour, 9 to 35 the
    /// inventory, 36 to 44 the hotbar, 45 the off hand; below 0 is a stack
    /// thrown out of the screen, which is dropped) and `item` (`minecraft:air`
    /// for an emptied slot) and `count` what is put there. Heard once the
    /// packet passed vanilla's checks (a creative player, a real slot, a valid
    /// stack) and before the slot changes or the stack drops. Set `cancel`
    /// and the packet is ignored. The player's state is swapped in for the
    /// call, as for `native_run_mod_command`, so `native_acting_player`,
    /// `native_tell` and the item helpers reach them. Not heard: stacks a
    /// creative player gets other ways (middle-click pick block, `/give`).
    /// Server-side, for every player of the world.
    pub(in crate::game) fn native_creative_slot(
        &mut self,
        player: u128,
        name: &str,
        slot: i32,
        item: &str,
        count: i32,
        cancel: &mut bool,
    ) {
        self.fire_creative_slot(player, name, slot, item, count, cancel);
    }
    /// `player` (`name`) changed dimension (NeoForge's
    /// `PlayerChangedDimensionEvent`): `from` and `to` are dimension ids
    /// (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`).
    /// Heard once per move, when it has ended: the portal's exit is found
    /// and the player stands there (after the destination has loaded), a
    /// command's teleport has landed, or a respawn in another level than the
    /// death's is placed for good (a bed or anchor, or the world spawn's
    /// free column, after the fallback when a respawn block was gone). The
    /// level of `to` is the one in place, and the player's state is swapped
    /// in, as for `native_run_mod_command`: `native_acting_player`,
    /// `native_tell`, the item helpers and the rest reach them. Heard for:
    /// Nether and End portals (also riding a vehicle, and leaving the End as
    /// its winner), `/execute in ... run tp` and any other command that
    /// teleports a player into another level, and death respawns, on dedicated
    /// and integrated servers (single player and LAN), where the world is simulated.
    /// Not heard by a remote client window, for a move within a level (`/tp`,
    /// an End gateway), for a player who leaves before the move ends, or for a
    /// move ending where it started (a respawn in the dimension of the death).
    pub(in crate::game) fn native_player_changed_dimension(
        &mut self,
        player: u128,
        name: &str,
        from: &str,
        to: &str,
    ) {
        self.fire_player_changed_dimension(player, name, from, to);
    }
    /// A player's client started (`flying`) or stopped flying (vanilla's
    /// `handlePlayerAbilities`, which honours it only for a player who may
    /// fly: creative or spectator), heard once for each change, on a server,
    /// with the player's state swapped in, so `native_tell` and the other
    /// helpers reach them. The player is already flying or not as asked, as
    /// vanilla leaves them; set `cancel` to refuse it: they keep the state
    /// they had and the server sends their abilities back, as vanilla does
    /// for what it refuses, so the client lands or flies again. Only what a
    /// client asks for is heard: not the flight a game-mode change starts or
    /// ends (`/gamemode`), and not a LAN host's own player.
    pub(in crate::game) fn native_player_toggle_flight(
        &mut self,
        player: u128,
        name: &str,
        flying: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_toggle_flight(player, name, flying, cancel);
    }
    /// A guest's client changed sneaking in its player input packet, after the hub applied the new state.
    /// The guest's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_toggle_sneak(
        &mut self,
        player: u128,
        name: &str,
        sneaking: bool,
    ) {
        self.fire_player_toggle_sneak(player, name, sneaking);
    }
    /// A player finished editing a sign: `ServerGamePacketListenerImpl.updateSignText`
    /// has taken their update packet and every check has passed (they were
    /// editing that sign, within reach, the sign is not waxed, the chat filter
    /// let the text through), right before the lines go on the sign. `pos` in
    /// `dimension`, the `front` or back side, and the four `lines` as written,
    /// plain text with the formatting codes still in (vanilla strips them
    /// afterwards). Rewrite `lines` (made four lines again) and that is what
    /// the sign gets, without a second look from the chat filter, or set
    /// `cancel` and the sign keeps its old text. With the player's state
    /// swapped in, so `native_tell` and the helpers that act on the player
    /// reach them. Once for each update a player sends on a server. Not told:
    /// the host's own editing of a sign on a LAN or single-player world
    /// (the editor writes into their world directly), and the sign changes
    /// that are no edit of the text (dyeing, glow ink, waxing, `/data`,
    /// `/setblock`, mods' `native_with_mod_block_entity`).
    pub(in crate::game) fn native_sign_changed(
        &mut self,
        call: mod_api::__private::SignChangedCall<'_>,
    ) {
        self.fire_sign_changed(call);
    }
    /// A command a player typed (NeoForge's `CommandEvent`, before
    /// `Commands.performCommand` runs it), without its slash: any command,
    /// vanilla's and the mods' own, also one the player may not run. Set
    /// `canceled` to stop it: nothing runs and nothing is answered but the
    /// lines pushed to `replies`, which go to the player as system chat.
    /// No player's state is swapped in and the server's player list is busy
    /// with this one, so `native_tell` and the other helpers that take a UUID
    /// reach nobody here: answer through `replies`. Once for each command a
    /// player sends, on a server, and for each line of a quiet command batch
    /// (`horizon:quiet_commands`, whose replies are dropped). A command the
    /// server refuses before reading it (unsigned chat arguments, the chat
    /// rate limit) never reaches it.
    pub(in crate::game) fn native_player_command(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        canceled: &mut bool,
        replies: &mut Vec<Text>,
    ) {
        self.fire_player_command(player, name, command, canceled, replies);
    }
    /// A server player's or mob's ender pearl is about to teleport its `owner`
    /// (`EntityTeleportEvent.EnderPearl`, `ThrownEnderpearl.onHit`). `owner` is
    /// the live entity handle, `pearl` its projectile id, `from` the owner's feet
    /// and `target` the pearl's previous position. A mod may move the target;
    /// non-finite changes keep the original spot. Canceling leaves the owner
    /// in place with no teleport sound, damage or endermite roll; the pearl is
    /// used up. Player owners keep their acting-player context and mutable fall
    /// `damage` (5; nonpositive is none). Mob owners take no fall damage and
    /// make no endermite draw; their `damage` starts at zero and is player-only.
    /// An owner moved, removed, killed or put to sleep by a listener is not moved
    /// again using the old decision. The impact hook still hears the hit.
    pub(in crate::game) fn native_ender_pearl_teleport(
        &mut self,
        call: mod_api::__private::EnderPearlTeleportCall<'_>,
    ) {
        self.fire_ender_pearl_teleport(call);
    }
    /// A server player or fox that ate chorus fruit (or another item with the
    /// `teleport_randomly` consume effect) is about to try a random spot
    /// (NeoForge's `EntityTeleportEvent.ItemConsumption`, vanilla's
    /// `TeleportRandomlyConsumeEffect.apply`): once per attempt (up to 16),
    /// after they stopped riding. `from` is their feet, `target` the spot
    /// drawn; a mod may move it (its y kept inside the level's height, a
    /// value that is not a number ignored), and the game still looks down
    /// from it for ground it may land on. Canceling ends the effect: no teleport and no
    /// more attempts. `consumed` is the stack captured before use, including its
    /// count and components. The player's state is swapped in
    /// (`native_acting_player`); a fox's full-host callback runs at its inline
    /// consumption boundary, before living AI and movement.
    pub(in crate::game) fn native_item_consumption_teleport(
        &mut self,
        consumer: mod_api::LivingRef,
        consumed: &mod_api::ItemStack,
        from: [f64; 3],
        target: &mut [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_item_consumption_teleport(consumer, consumed, from, target, cancel);
    }
    /// A server player a command teleported (NeoForge's
    /// `EntityTeleportEvent`, for a player): `/tp` and `/teleport`, inside
    /// their level or into another one. `from` and `to` are their feet before
    /// and after, `dimension` the level they are in now. Heard after the move
    /// has reached the player's state and their client (an inter-level
    /// teleport: when the new level is entered, before it has loaded around
    /// them), so it cannot stop it. The player's state is swapped in, so the
    /// helpers that read the acting player and `native_tell` work. Once for
    /// each teleported player, on a server. Ender pearls and chorus fruit
    /// have their own hooks; a spectator's teleport menu is not heard yet;
    /// portals and respawns are not teleports here.
    pub(in crate::game) fn native_player_teleported(
        &mut self,
        player: u128,
        name: &str,
        from: [f64; 3],
        to: [f64; 3],
        dimension: &str,
    ) {
        self.fire_player_teleported(player, name, from, to, dimension);
    }
    /// A player whose login the server has accepted (NeoForge's
    /// `PlayerNegotiationEvent` stage): the name, whitelist, ban and
    /// server-full checks have passed and the player holds a place, but their
    /// saved state is not loaded and they are not placed in a level yet. Set
    /// `cancel` to refuse them: they are disconnected with `reason` (a plain
    /// "Disconnected" if it is left unset) and nothing else of the join (the
    /// join message, `native_player_logged_in`) happens. Once for each player
    /// who joins, on a server. The player is not placed yet and their client
    /// has no world to show a message in, so no player's state is swapped in
    /// and `native_tell` reaches nobody: the refusal `reason` is the answer.
    /// The checks that need no mod (name, whitelist, bans, a full server,
    /// the proxy's forwarding) refuse earlier, on the connection, and this
    /// hook does not hear those players.
    pub(in crate::game) fn native_player_login(
        &mut self,
        player: u128,
        name: &str,
        cancel: &mut bool,
        reason: &mut Option<Text>,
    ) {
        self.fire_player_login(player, name, cancel, reason);
    }
    /// The command tree a server sends a player (NeoForge's and Paper's
    /// "commands sent to a player" stage): when they join and when their
    /// level changes (`/op`, `/deop`, the LAN settings). `names` are the
    /// root commands their client would get, vanilla's they may use and the
    /// mods'; take a name out and that client gets the tree without it
    /// (and without what it leads to), so it neither completes nor
    /// highlights it. This hides the command from the client only: the
    /// server still runs a typed one, `native_player_command` stops it.
    /// For a player who is already on, their state is swapped in and the
    /// helpers (`native_tell`...) work; for one who is joining it is not, and
    /// their client has no world to tell yet. The tree the hub sends again by
    /// itself (after a respawn or a change of level) keeps the last answer
    /// and does not ask again; the integrated server's owner is not asked
    /// when the world's cheats are switched.
    pub(in crate::game) fn native_player_commands_sent(
        &mut self,
        player: u128,
        name: &str,
        names: &mut Vec<String>,
    ) {
        self.fire_player_commands_sent(player, name, names);
    }
    /// A server player's sprint started or stopped: their client sent
    /// `START_SPRINTING` or `STOP_SPRINTING` (the sprint key, a double tap
    /// forward, or letting go) and the server's player is now `sprinting`
    /// (vanilla's `setSprinting` in `handlePlayerCommand`). Heard only when it
    /// changes. It cannot be stopped, and it hears what the client asks, not
    /// what the server does: a sprint that ends by itself (a hit's knockback,
    /// hunger, a wall) is not reported. The player's state is swapped in, so
    /// the helpers and `native_tell` work. Once for each change, on a server.
    pub(in crate::game) fn native_player_sprint(
        &mut self,
        player: u128,
        name: &str,
        sprinting: bool,
    ) {
        self.fire_player_sprint(player, name, sprinting);
    }
    /// A server player's client answered a resource pack in play (vanilla's
    /// `ServerboundResourcePackPacket`): `status` is `Action`'s number, 0
    /// successfully loaded, 1 declined, 2 failed download, 3 accepted, 4
    /// downloaded, 5 invalid URL, 6 failed reload, 7 discarded. Heard when the
    /// packet arrives, after the server has handled it; it cannot be stopped.
    /// The player's state is swapped in, so the helpers and `native_tell`
    /// work. The pack a dedicated server offers in the configuration
    /// (`resource-pack` in `server.properties`) is answered before the
    /// player has joined, and those answers are not heard. Once for each
    /// answer, on a server.
    pub(in crate::game) fn native_resource_pack_status(
        &mut self,
        player: u128,
        name: &str,
        status: i32,
    ) {
        self.fire_resource_pack_status(player, name, status);
    }
    /// A server player places a hanging entity: an item frame
    /// (`minecraft:item_frame`, `minecraft:glow_item_frame`), a painting
    /// (`minecraft:painting`) or a leash knot (`minecraft:leash_knot`, when a
    /// lead is tied to a fence that has none yet), `kind` its entity type and
    /// `pos` the block it takes (the fence's for a knot). Where
    /// `HangingEntityItem.useOn` and `LeadItem.useOn` create it, once it is
    /// known to fit and before anything is placed or used up. Set `cancel`
    /// and nothing is placed: the item stays in the player's hand. The
    /// player's state is swapped in, so `native_tell` answers them. Only for
    /// guests of a server; `/summon`, a dispenser and the game's own player
    /// are not heard, nor is a lead tied to a mob or a boat.
    pub(in crate::game) fn native_hanging_place(
        &mut self,
        player: u128,
        name: &str,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_hanging_place(player, name, kind, pos, cancel);
    }
    /// An item frame (`minecraft:item_frame`, `minecraft:glow_item_frame`) or
    /// a painting (`minecraft:painting`), `kind` its entity type and `pos` the
    /// block it hangs in, is about to break: a server player's hit
    /// (`player`, their state swapped in, so `native_tell` answers them), a
    /// blast, or the wall it hangs on gone (`player` None for both). Where
    /// `BlockAttachedEntity.hurtServer` and its survival check would kill
    /// it, before anything drops or sounds. Set `cancel` and it stays, as it
    /// was. A hit that only takes the item out of a frame does not break the
    /// frame and is not asked. The game's own player's hit is not asked, nor
    /// is `/kill`.
    pub(in crate::game) fn native_hanging_break(
        &mut self,
        player: Option<u128>,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_hanging_break(player, kind, pos, cancel);
    }
    /// A server player empties a bucket at `pos` (`fluid`: `minecraft:water`,
    /// `minecraft:lava`, `minecraft:powder_snow`; a fish or axolotl bucket
    /// empties `minecraft:water`): where `BucketItem.use` has found the block
    /// the fluid goes into and the player may build there, right before it is
    /// placed. Set `cancel` and nothing is poured and the bucket stays full.
    /// The player's state is swapped in, so `native_tell` answers them. Only
    /// for guests of a server. `native_fill_bucket` hears this click too (and
    /// the filling one) without the player and the fluid. Not heard: a
    /// dispenser, a cauldron, and a fluid a block places itself.
    pub(in crate::game) fn native_bucket_empty(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        fluid: &str,
        cancel: &mut bool,
    ) {
        self.fire_bucket_empty(player, name, pos, fluid, cancel);
    }
    /// TNT at `pos` is about to be lit (primed): a flint and steel or a fire
    /// charge on it, fire burning it, a redstone signal, an arrow, or a
    /// dispenser's flint and steel or TNT item. `player` is the server player
    /// whose click lit it, with their state swapped in (so `native_tell`
    /// answers them), None when no player is behind it (fire, redstone, an
    /// arrow, a dispenser). Set `cancel` and it is not primed: a TNT block
    /// stays where it is (one that fire or a signal had already taken out is
    /// put back, without a block update), a dispenser's TNT item stays in it.
    /// Not asked: TNT set off by a blast (`native_explosion_detonate` spares
    /// blocks), a TNT minecart and a sulfur cube. Server-side and in single
    /// player.
    pub(in crate::game) fn native_tnt_primed(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_tnt_primed(player, pos, cancel);
    }
    /// `PlayerSwitchHotbarSlotEvent.Pre`: a server player is switching from hotbar slot `old_slot`
    /// to `new_slot` (0 to 8), after the slot range check and before the selected slot changes.
    /// Fired only when the slot differs. The player's state is not swapped in. Set `cancel` to keep
    /// the old slot; the client is sent that slot again, and the post hook doesn't fire.
    pub(in crate::game) fn native_player_switch_hotbar_slot_pre(
        &mut self,
        player: u128,
        player_entity_id: i32,
        old_slot: i32,
        new_slot: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_switch_hotbar_slot_pre(
            player,
            player_entity_id,
            old_slot,
            new_slot,
            cancel,
        );
    }
    /// `PlayerSwitchHotbarSlotEvent.Post`: a server player switched from hotbar slot `old_slot`
    /// to `new_slot`, after the selected slot changed and a main-hand item use stopped. The
    /// player's state is not swapped in.
    pub(in crate::game) fn native_player_switch_hotbar_slot_post(
        &mut self,
        player: u128,
        player_entity_id: i32,
        old_slot: i32,
        new_slot: i32,
    ) {
        self.fire_player_switch_hotbar_slot_post(player, player_entity_id, old_slot, new_slot);
    }
    /// A server player picked an item from a block or entity, before it enters the hotbar. `pos` is
    /// zero for an entity; `entity_id` is zero for a block. The player's state is swapped in. Set
    /// `cancel` to leave the inventory and client unchanged.
    pub(in crate::game) fn native_player_pick_item(
        &mut self,
        player: u128,
        name: &str,
        from_entity: bool,
        pos: [i32; 3],
        entity_id: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_pick_item(player, name, from_entity, pos, entity_id, cancel);
    }
    /// A server player is about to enter a bed after the sleep checks pass. Set `cancel` to keep them
    /// out. The player's state is swapped in.
    pub(in crate::game) fn native_player_bed_enter(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_player_bed_enter(player, name, pos, cancel);
    }
    /// A server player's book edit or signing packet is about to be applied. `pages` is its page
    /// count; `signing` means the packet has a title. Set `cancel` to drop the edit.
    pub(in crate::game) fn native_player_edit_book(
        &mut self,
        player: u128,
        name: &str,
        pages: i32,
        signing: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_edit_book(player, name, pages, signing, cancel);
    }
    /// A player shearing a mob is about to do so. Set `cancel` to leave the mob and shears unchanged.
    /// The guest's state is swapped in.
    pub(in crate::game) fn native_player_shear_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
        off_hand: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_shear_entity(player, name, entity_id, off_hand, cancel);
    }
    /// A server player completed an advancement, after progress was recorded.
    pub(in crate::game) fn native_player_advancement_done(
        &mut self,
        player: u128,
        name: &str,
        advancement: &str,
    ) {
        self.fire_player_advancement_done(player, name, advancement);
    }
    /// A criterion of a server player's advancement was granted or revoked, after its progress changed
    /// and before any completion rewards. A grant or revoke that changes nothing is not heard.
    /// The call carries a snapshot of all criterion dates and requirement results at this mutation;
    /// no player-progress storage is borrowed while listeners run.
    pub(in crate::game) fn native_player_advancement_progress(
        &mut self,
        call: mod_api::__private::PlayerAdvancementProgressCall<'_>,
    ) {
        self.fire_player_advancement_progress(call);
    }
    /// A statistic is about to be awarded to a server player, before its count and scoreboard criteria
    /// change. `stat_type` and `stat` are ids such as `minecraft:mined` and `minecraft:stone`, and
    /// `value` the amount; all three are writable. Invalid rewrites retain the original. Set `cancel` to award nothing.
    pub(in crate::game) fn native_player_stat_award(
        &mut self,
        player: u128,
        name: &str,
        stat_type: &mut String,
        stat: &mut String,
        value: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_player_stat_award(player, name, stat_type, stat, value, cancel);
    }
    /// An item cooldown is about to start for a server player. `item` is the item id and `ticks` its
    /// duration. The player's state is not swapped in. Set `cancel` to skip starting the cooldown.
    /// Server-side.
    pub(in crate::game) fn native_player_item_cooldown(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        ticks: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_item_cooldown(player, name, item, ticks, cancel);
    }
    /// A recipe is about to be unlocked for a server player. Set `cancel` to keep it locked. The
    /// player's state is not swapped in. Server-side.
    pub(in crate::game) fn native_player_recipe_discovered(
        &mut self,
        player: u128,
        name: &str,
        recipe: &str,
        cancel: &mut bool,
    ) {
        self.fire_player_recipe_discovered(player, name, recipe, cancel);
    }
    /// A server player launched with a riptide trident, after the velocity,
    /// spin attack and grounded lift have been applied. The player's state is
    /// swapped in. No reply is needed.
    pub(in crate::game) fn native_player_riptide(&mut self, player: u128, name: &str) {
        self.fire_player_riptide(player, name);
    }
    /// A server player's fishing rod, with `state`: 0 the cast
    /// (`FishingRodItem.use` throwing the bobber), 1 a fish biting
    /// (`FishingHook.catchingFish`), 2 the reel-in of a bite, which gives the
    /// catch, 3 the reel-in with nothing on the line, 4 the reel-in of a
    /// bobber hooked to an entity or item (like Bukkit's `PlayerFishEvent`;
    /// NeoForge's `ItemFishedEvent` is later, in `FishingHook.retrieve`). Set `cancel` on the cast and nothing
    /// is thrown; on a bite the fish gets away and the wait starts over; on
    /// a reel-in (2 to 4) nothing happens, the bobber stays out and the rod
    /// takes no wear. The bobber ending by itself (too far, the rod put
    /// away, the owner dying), nor the host of a LAN world's rod. The
    /// player's state is swapped in, so `native_acting_player`, `native_tell`
    /// and the item helpers reach them. Once for each cast, bite and reel-in,
    /// on a server.
    pub(in crate::game) fn native_player_fish(
        &mut self,
        player: u128,
        name: &str,
        state: i32,
        cancel: &mut bool,
    ) {
        self.fire_player_fish(player, name, state, cancel);
    }
    /// A sculk sensor or shrieker received a vibration for `event` at `pos`.
    /// Set `cancel` to suppress its response. Server-side.
    pub(in crate::game) fn native_block_receive_game(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        block: &str,
        event: &str,
        cancel: &mut bool,
    ) {
        self.fire_block_receive_game(dimension, pos, block, event, cancel);
    }
    /// A dispenser sheared `entity` of kind `kind` at the block in front of
    /// its own block at `pos`, after the shearing effects. Server-side.
    pub(in crate::game) fn native_block_shear_entity(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        entity: u32,
        kind: &str,
    ) {
        self.fire_block_shear_entity(dimension, pos, entity, kind);
    }
    /// A dispenser equipped `item` on the first eligible `target` in front
    /// of its block at `pos`. `target` is `player`, `mob` or `armor_stand`.
    /// The equip, item consumption, sound and game event have completed.
    /// Server-side.
    pub(in crate::game) fn native_block_dispense_armor(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        item: &str,
        target: &str,
    ) {
        self.fire_block_dispense_armor(dimension, pos, item, target);
    }
    /// A brewing stand at `pos` began brewing `ingredient`, after fuel and the timer were set. Server-side.
    pub(in crate::game) fn native_brewing_start(&mut self, pos: [i32; 3], ingredient: &str) {
        self.fire_brewing_start(pos, ingredient);
    }
    /// A brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side.
    pub(in crate::game) fn native_brewing_fuel(&mut self, pos: [i32; 3], fuel: &str) {
        self.fire_brewing_fuel(pos, fuel);
    }
    /// A brewing stand finished brewing at `pos`, after its bottle contents
    /// were changed. Server-side.
    pub(in crate::game) fn native_brewing_complete(&mut self, pos: [i32; 3]) {
        self.fire_brewing_complete(pos);
    }
    /// An item was placed on a campfire at `pos` and began cooking.
    pub(in crate::game) fn native_campfire_start(&mut self, pos: [i32; 3], item: &str) {
        self.fire_campfire_start(pos, item);
    }
    /// A block finished cooking `item` into `result` at `pos`.
    pub(in crate::game) fn native_block_cook(&mut self, pos: [i32; 3], item: &str, result: &str) {
        self.fire_block_cook(pos, item, result);
    }
    /// A compostable item was added to the composter at `pos`.
    pub(in crate::game) fn native_compost_item(&mut self, pos: [i32; 3], item: &str) {
        self.fire_compost_item(pos, item);
    }
    /// A crafter at `pos` completed recipe `recipe`.
    pub(in crate::game) fn native_crafter_craft(&mut self, pos: [i32; 3], recipe: &str) {
        self.fire_crafter_craft(pos, recipe);
    }
    /// A server player is about to open an inventory supplied by a block at
    /// `pos`. Set `cancel` to leave the inventory closed. The player's state
    /// is not swapped in.
    pub(in crate::game) fn native_inventory_block_start(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_inventory_block_start(player, name, pos, cancel);
    }

    /// A sculk catalyst at `pos` has begun blooming after taking experience.
    /// The catalyst block state has changed. Server-side.
    pub(in crate::game) fn native_sculk_bloom(&mut self, dimension: &str, pos: [i32; 3]) {
        self.fire_sculk_bloom(dimension, pos);
    }

    /// A projectile hit the target block at `pos`, which now has `strength`
    /// output power. Server-side.
    pub(in crate::game) fn native_target_hit(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        strength: i32,
    ) {
        self.fire_target_hit(dimension, pos, strength);
    }

    /// A vault at `pos` changed to `state`. Its block state is updated.
    /// Server-side.
    pub(in crate::game) fn native_vault_change_state(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: &str,
    ) {
        self.fire_vault_change_state(dimension, pos, state);
    }

    /// A dispenser at `pos` failed to dispense `item`. Server-side.
    pub(in crate::game) fn native_block_failed_dispense(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        item: &str,
    ) {
        self.fire_block_failed_dispense(dimension, pos, item);
    }

    /// A beacon at `pos` activated or deactivated after its beam was checked.
    /// `active` says whether it now has a complete pyramid. Server-side.
    pub(in crate::game) fn native_beacon_state_change(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        active: bool,
    ) {
        self.fire_beacon_state_change(dimension, pos, active);
    }

    /// An anvil at `pos` took damage, changing its block from `from` to `to`.
    /// `to` is air when the anvil broke. Server-side.
    pub(in crate::game) fn native_anvil_damaged(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    ) {
        self.fire_anvil_damaged(dimension, pos, from, to);
    }

    /// A server guest's block-breaking animation changed to `progress` at
    /// `pos` (0 through 9).
    pub(in crate::game) fn native_block_break_progress_update(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        progress: i32,
    ) {
        self.fire_block_break_progress_update(player, name, pos, progress);
    }

    /// A guest's block inventory lock check at `pos`. `locked` is the
    /// vanilla result and may be changed. The guest's state is not swapped in.
    pub(in crate::game) fn native_block_lock_check(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        locked: &mut bool,
    ) {
        self.fire_block_lock_check(player, name, pos, locked);
    }

    /// A server player reset their attack strength by swinging (`handlePunch`). The swing and reset
    /// have already happened. The player's state is not swapped in. Server-side.
    pub(in crate::game) fn native_player_attack_cooldown_reset(
        &mut self,
        player: u128,
        name: &str,
    ) {
        self.fire_player_attack_cooldown_reset(player, name);
    }
    /// A server player caught a mob in a bucket, after it was removed and the filled bucket was given.
    /// Server-side.
    pub(in crate::game) fn native_player_bucket_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
        entity: &str,
    ) {
        self.fire_player_bucket_entity(player, name, entity_id, entity);
    }
    /// A server player changed a beacon's selected effects, after they were stored. Each selected
    /// effect is its load-owned key; None means no effect. Server-side.
    pub(in crate::game) fn native_player_change_beacon_effect(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        primary: Option<EffectKey>,
        secondary: Option<EffectKey>,
    ) {
        self.fire_player_change_beacon_effect(player, name, pos, primary, secondary);
    }
    /// A server player has slept for 100 ticks in a bed and now counts as deeply asleep for the
    /// night-skip check.
    pub(in crate::game) fn native_player_deep_sleep(&mut self, player: u128, name: &str) {
        self.fire_player_deep_sleep(player, name);
    }
    /// A server player threw an egg, after its projectile was launched and the use was applied.
    /// Server-side.
    pub(in crate::game) fn native_player_egg_throw(&mut self, player: u128, name: &str) {
        self.fire_player_egg_throw(player, name);
    }
    /// A server player used a firework rocket to boost while gliding, after the rocket was launched
    /// and consumed. Server-side.
    pub(in crate::game) fn native_player_elytra_boost(&mut self, player: u128, name: &str) {
        self.fire_player_elytra_boost(player, name);
    }
    /// A server player put a plant into a flower pot, after the potted block state was stored and the
    /// item consumed. `plant` is the item id and `pos` the pot position. Server-side.
    pub(in crate::game) fn native_player_flower_pot_manipulate(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        plant: &str,
    ) {
        self.fire_player_flower_pot_manipulate(player, name, pos, plant);
    }
    /// A server player inserted a book into a lectern, after the block entity and inventory changed.
    /// `book` is the item id. Server-side.
    pub(in crate::game) fn native_player_insert_lectern_book(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        book: &str,
    ) {
        self.fire_player_insert_lectern_book(player, name, pos, book);
    }
    /// A server player took the book from a lectern, after the menu and block entity changed. `book`
    /// is the removed item id. Server-side.
    pub(in crate::game) fn native_player_take_lectern_book(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        book: &str,
    ) {
        self.fire_player_take_lectern_book(player, name, pos, book);
    }
    /// A server player changed the page of a lectern, after its page and redstone pulse changed.
    /// `page` is the zero-based new page. Server-side.
    pub(in crate::game) fn native_player_lectern_page_change(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        page: i32,
    ) {
        self.fire_player_lectern_page_change(player, name, pos, page);
    }
    /// A server player changed an item frame after its contents or rotation changed. `action` is
    /// `put`, `rotate`, `remove_item` or `break`; `item` is empty when no item remains, and `rotation`
    /// is the resulting step. Server-side.
    pub(in crate::game) fn native_player_item_frame_change(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        action: &str,
        item: &str,
        rotation: u8,
    ) {
        self.fire_player_item_frame_change(player, name, entity_id, action, item, rotation);
    }
    /// Mending repaired a server player's item with experience from an orb. `repair` is durability
    /// restored and `experience` is the orb's value. Server-side.
    pub(in crate::game) fn native_player_item_mend(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        repair: u32,
        experience: u32,
    ) {
        self.fire_player_item_mend(player, name, item, repair, experience);
    }
    /// A server player completed a villager or wandering trader trade. `entity_id` is the merchant
    /// network id. Server-side.
    pub(in crate::game) fn native_player_trade(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
    ) {
        self.fire_player_trade(player, name, entity_id);
    }
    /// A server player traded with a villager or a wandering trader
    /// (NeoForge's `TradeWithVillagerEvent`, at vanilla's
    /// `MerchantResultSlot.onTake` once the offer is taken): `merchant` is
    /// the trader's entity id and `kind` its type (`minecraft:villager`,
    /// `minecraft:wandering_trader`), `offer` the trade as it is after this
    /// one (its `uses` counted, `result` what the player got). Once per
    /// trade, so a shift-click that trades several times is heard for each,
    /// after the trade's statistic and `villager_trade` trigger. Listen
    /// only: the trade is made. The player's state is swapped in
    /// (`native_acting_player`).
    pub(in crate::game) fn native_trade_with_villager(
        &mut self,
        merchant: u32,
        kind: &str,
        offer: &mut MerchantOffer,
    ) {
        self.fire_trade_with_villager(merchant, kind, offer);
    }
    /// A server player's shield was disabled by a blocked melee hit. The
    /// shield cooldown is set immediately after this call. Helpers looking up
    /// the guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 276.
    pub(in crate::game) fn native_player_shield_disable(
        &mut self,
        player: u128,
        name: &str,
        ticks: u32,
    ) {
        self.fire_player_shield_disable(player, name, ticks);
    }
    /// A server player selected a loom pattern, after the loom menu accepted the button. `pattern` is
    /// the zero-based button index. Server-side.
    pub(in crate::game) fn native_player_loom_pattern_select(
        &mut self,
        player: u128,
        name: &str,
        pattern: i32,
    ) {
        self.fire_player_loom_pattern_select(player, name, pattern);
    }
    /// A server player named a mob with a name tag, after its name and persistence changed and the tag
    /// was consumed. Server-side.
    pub(in crate::game) fn native_player_name_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        entity_name: &str,
    ) {
        self.fire_player_name_entity(player, name, entity_id, entity_name);
    }
    /// A server player opened a sign for editing, after the editor state was made. `front` selects the
    /// front or back side. Server-side.
    pub(in crate::game) fn native_player_open_sign(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        front: bool,
    ) {
        self.fire_player_open_sign(player, name, pos, front);
    }
    /// A server player picked up an arrow after it was removed and its pickup was applied. `item` is
    /// the arrow's pickup item id. Server-side.
    pub(in crate::game) fn native_player_pickup_arrow(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        item: &str,
    ) {
        self.fire_player_pickup_arrow(player, name, entity_id, item);
    }
    /// A server player selected a stonecutter recipe, after the menu accepted
    /// its button. `recipe` is the zero-based button index. Server-side.
    pub(in crate::game) fn native_player_stonecutter_recipe_select(
        &mut self,
        player: u128,
        name: &str,
        recipe: i32,
    ) {
        self.fire_player_stonecutter_recipe_select(player, name, recipe);
    }
    /// A server player stopped spectating `entity_id`, after their camera returned to the player.
    pub(in crate::game) fn native_player_stop_spectating_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
    ) {
        self.fire_player_stop_spectating_entity(player, name, entity_id);
    }
    /// A server applied `velocity` to a player, when syncing changed motion to that player's client.
    pub(in crate::game) fn native_player_velocity(
        &mut self,
        player: u128,
        name: &str,
        velocity: [f64; 3],
    ) {
        self.fire_player_velocity(player, name, velocity);
    }
    /// A server player removed an entity's leash, after it was cleared.
    pub(in crate::game) fn native_player_unleash_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
    ) {
        self.fire_player_unleash_entity(player, name, entity_id);
    }
    /// A server player toggled an ageable mob's age lock with a golden
    /// dandelion, after the lock and item use changed.
    pub(in crate::game) fn native_player_toggle_entity_age_lock(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        locked: bool,
    ) {
        self.fire_player_toggle_entity_age_lock(player, name, entity_id, locked);
    }
    /// A server player swapped their main and off hands, after both inventory slots changed.
    /// Server-side.
    pub(in crate::game) fn native_player_swap_with_equipment_slot(
        &mut self,
        player: u128,
        name: &str,
        slot: &str,
    ) {
        self.fire_player_swap_with_equipment_slot(player, name, slot);
    }
    /// A server player filled an empty map, after the filled map and its saved data were created.
    /// Server-side.
    pub(in crate::game) fn native_player_map_filled(
        &mut self,
        player: u128,
        name: &str,
        map_id: i32,
    ) {
        self.fire_player_map_filled(player, name, map_id);
    }
    /// A server player is about to travel through an End gateway at `gateway`. Set `cancel` to leave
    /// them at the entrance; the gateway is not consumed and its cooldown does not start. Server-side.
    pub(in crate::game) fn native_player_teleport_end_gateway(
        &mut self,
        player: u128,
        name: &str,
        gateway: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_player_teleport_end_gateway(player, name, gateway, cancel);
    }
    /// A server player's recipe book requested a known recipe be placed in
    /// their crafting grid. `recipe` is the recipe id; `all` is the shift
    /// click form. Server-side.
    pub(in crate::game) fn native_player_recipe_book_click(
        &mut self,
        player: u128,
        name: &str,
        recipe: &str,
        all: bool,
    ) {
        self.fire_player_recipe_book_click(player, name, recipe, all);
    }
    /// A server player's XP orb pickup cooldown changed to `ticks`. It is
    /// reported after the value changes. Server-side.
    pub(in crate::game) fn native_player_exp_cooldown_change(
        &mut self,
        player: u128,
        name: &str,
        ticks: u32,
    ) {
        self.fire_player_exp_cooldown_change(player, name, ticks);
    }
    /// A server entity was moved by a same-level teleport command, after its
    /// position changed. `entity` is its network id; `from` and `to` are
    /// coordinates in `dimension`. This hook reports command teleports that
    /// use Horizon's same-level target path; it does not change vanilla's
    /// movement.
    pub(in crate::game) fn native_entity_teleport(
        &mut self,
        dimension: &str,
        entity: u32,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        self.fire_entity_teleport(dimension, entity, from, to);
    }

    /// A server entity finished its portal wait and is beginning to leave
    /// `dimension` through `pos`. The destination follows the portal rules.
    pub(in crate::game) fn native_entity_portal(
        &mut self,
        dimension: &str,
        entity: u32,
        pos: [i32; 3],
    ) {
        self.fire_entity_portal(dimension, entity, pos);
    }

    /// A server projectile was added to the level. `player` is set only for
    /// a player launch; other sources have an empty `name`.
    pub(in crate::game) fn native_projectile_launch(
        &mut self,
        player: Option<u128>,
        name: &str,
        entity: u32,
        kind: &str,
    ) {
        self.fire_projectile_launch(player, name, entity, kind);
    }

    /// A player bow or crossbow launched an arrow with `force`, after the
    /// projectile entered the level.
    pub(in crate::game) fn native_entity_shoot_bow(
        &mut self,
        player: u128,
        name: &str,
        projectile: u32,
        force: f32,
    ) {
        self.fire_entity_shoot_bow(player, name, projectile, force);
    }

    /// A guest's fall flying state changed on the server. The change has
    /// already been applied.
    pub(in crate::game) fn native_entity_toggle_glide(
        &mut self,
        player: u128,
        name: &str,
        gliding: bool,
    ) {
        self.fire_entity_toggle_glide(player, name, gliding);
    }

    /// A guest player's synchronized pose changed from `from` to `to`.
    pub(in crate::game) fn native_entity_pose_change(
        &mut self,
        player: u128,
        name: &str,
        from: &str,
        to: &str,
    ) {
        self.fire_entity_pose_change(player, name, from, to);
    }

    /// A dropped item entity was successfully added to the server level.
    pub(in crate::game) fn native_item_spawn(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        self.fire_item_spawn(entity, item, count, pos);
    }

    /// A mob became tamed after its owner and persistent state were set.
    pub(in crate::game) fn native_entity_tame(
        &mut self,
        entity: u32,
        kind: &str,
        owner: Option<u128>,
    ) {
        self.fire_entity_tame(entity, kind, owner);
    }

    /// Two mobs completed a breeding action. `child` is absent when vanilla
    /// lays an egg instead of creating a baby.
    pub(in crate::game) fn native_entity_breed(
        &mut self,
        parent: u32,
        partner: u32,
        child: Option<u32>,
    ) {
        self.fire_entity_breed(parent, partner, child);
    }

    /// A furnace at `pos` began cooking `item` (its cooking timer advanced
    /// from zero). Server-side.
    pub(in crate::game) fn native_furnace_start_smelt(&mut self, pos: [i32; 3], item: &str) {
        self.fire_furnace_start_smelt(pos, item);
    }
    /// A furnace at `pos` consumed fuel and began or resumed burning it.
    /// `fuel` is the item consumed from its fuel slot. Server-side.
    pub(in crate::game) fn native_furnace_burn(&mut self, pos: [i32; 3], fuel: &str) {
        self.fire_furnace_burn(pos, fuel);
    }
    /// A furnace finished smelting `item` into `result` at `pos`. Server-side.
    pub(in crate::game) fn native_furnace_smelt(
        &mut self,
        pos: [i32; 3],
        item: &str,
        result: &str,
    ) {
        self.fire_furnace_smelt(pos, item, result);
    }
    /// A server player took an output from a furnace at `pos`; `item` is the output item id and
    /// `count` is how many were taken. The player's state is not swapped in.
    pub(in crate::game) fn native_furnace_extract(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        item: &str,
        count: u32,
    ) {
        self.fire_furnace_extract(player, name, pos, item, count);
    }
    /// A server player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. The player's state is not swapped in.
    pub(in crate::game) fn native_enchant_item(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        item: &ItemStack,
        levels: i32,
    ) {
        self.fire_enchant_item(player, name, pos, item, levels);
    }
    /// Enchanting offers were calculated for a server player at `pos`. Change `costs` before they are sent to the menu.
    pub(in crate::game) fn native_prepare_item_enchant(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        costs: &mut [i32; 3],
    ) {
        self.fire_prepare_item_enchant(player, name, pos, costs);
    }
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`), before the levels are charged, the inputs used up and the anvil worn. `craft` holds the inputs and the result as they were before. Set `cancel` to refuse the take: the result stays in its slot and the inputs, levels, inventory and cursor are as before the click.
    pub(in crate::game) fn native_anvil_craft_pre(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        craft: &mod_api::item::menu::AnvilCraft,
        cancel: &mut bool,
    ) {
        self.fire_anvil_craft_pre(player, name, pos, craft, cancel);
    }
    /// A player took an anvil's result at `pos` and the levels, inputs and anvil wear are done (`AnvilCraftEvent.Post`); a broken anvil is already air. `craft` holds the inputs and the result as they were before. Not called when `native_anvil_craft_pre` cancelled.
    pub(in crate::game) fn native_anvil_craft_post(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        craft: &mod_api::item::menu::AnvilCraft,
    ) {
        self.fire_anvil_craft_post(player, name, pos, craft);
    }
    /// A smithing result was prepared for a server player at `pos`. Change it before it is sent.
    pub(in crate::game) fn native_prepare_smithing(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        result: &mut Option<ItemStack>,
    ) {
        self.fire_prepare_smithing(player, name, pos, result);
    }
    /// A crafting-grid result was prepared for a server player; `pos` is the crafting table, or None for the inventory grid. Change `result` before it is sent.
    pub(in crate::game) fn native_prepare_item_craft(
        &mut self,
        player: u128,
        name: &str,
        pos: Option<[i32; 3]>,
        result: &mut Option<ItemStack>,
    ) {
        self.fire_prepare_item_craft(player, name, pos, result);
    }
    /// A server player selected trade `index` from `villager` in the trading menu.
    pub(in crate::game) fn native_trade_select(
        &mut self,
        player: u128,
        name: &str,
        villager: Option<u32>,
        index: usize,
    ) {
        self.fire_trade_select(player, name, villager, index);
    }
    /// The server changed its weather parameters. `raining` and `thundering`
    /// are the requested final states. Set `cancel` to keep the previous
    /// weather parameters. Server-side.
    pub(in crate::game) fn native_weather_change(
        &mut self,
        dimension: &str,
        raining: bool,
        thundering: bool,
        cancel: &mut bool,
    ) {
        crate::game::level_control_hooks::LevelControl::Weather
            .hear(|| self.fire_weather_change(dimension, raining, thundering, cancel));
    }

    /// The server changed its thunder state through `/weather`. Set `cancel`
    /// to keep the previous thunder state and timer. Server-side.
    pub(in crate::game) fn native_thunder_change(
        &mut self,
        dimension: &str,
        thundering: bool,
        cancel: &mut bool,
    ) {
        crate::game::level_control_hooks::LevelControl::Weather
            .hear(|| self.fire_thunder_change(dimension, thundering, cancel));
    }

    /// A server lightning bolt is about to be created. `cause` is `weather`,
    /// `trident`, `command` or `mod` (a native mod's `strike_lightning`); `player` is the responsible player when known.
    /// Set `cancel` to prevent the bolt and its effects.
    pub(in crate::game) fn native_lightning_strike(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cause: &str,
        cancel: &mut bool,
    ) {
        self.fire_lightning_strike(player, pos, cause, cancel);
    }

    /// Enough players slept through the night in level `dimension` and the clock is about to
    /// move to the morning (NeoForge's `SleepFinishedTimeEvent`). `adjustment` comes in as
    /// `Marker(minecraft:wake_up_from_sleep)` and a mod may set `Absolute`, `Relative` (from
    /// `current`) or another marker; `new_time` is the vanilla morning, read only (contract 2: in
    /// version 1 a mod wrote it). Times are the level's own clock's; resolved once after every mod
    /// (`game/world/wake_time.rs`): a time before `current` or past `i64::MAX`, or a marker the
    /// clock lacks, moves to the morning. `native_time_skip` then hears the jump and may cancel
    /// it. Set `canceled` and the clock stays, and `native_time_skip` is not asked.
    pub(in crate::game) fn native_sleep_finished_time(
        &mut self,
        dimension: &str,
        current: i64,
        new_time: i64,
        adjustment: &mut ClockAdjustment,
        canceled: &mut bool,
    ) {
        self.fire_sleep_finished_time(dimension, current, new_time, adjustment, canceled);
    }

    /// The overworld clock is about to jump, from sleep or `/time`. Set
    /// `cancel` to keep `from`.
    pub(in crate::game) fn native_time_skip(
        &mut self,
        dimension: &str,
        from: i64,
        to: i64,
        cause: &str,
        cancel: &mut bool,
    ) {
        self.fire_time_skip(dimension, from, to, cause, cancel);
    }
    /// The world's spawn point changed to `pos` and `angle`.
    pub(in crate::game) fn native_spawn_change(&mut self, pos: [i32; 3], angle: [f32; 2]) {
        self.fire_spawn_change(pos, angle);
    }

    /// The server world's border bounds changed, either immediately or when
    /// its timed size change was requested.
    pub(in crate::game) fn native_world_border_bounds_changed(&mut self) {
        crate::game::level_control_hooks::LevelControl::BorderBounds
            .hear(|| self.fire_world_border_bounds_changed());
    }

    /// The server world's border center changed.
    pub(in crate::game) fn native_world_border_center_changed(&mut self) {
        crate::game::level_control_hooks::LevelControl::BorderCenter
            .hear(|| self.fire_world_border_center_changed());
    }

    /// A Nether portal was built for a dimension travel, or an End platform
    /// was made for an arrival. `pos` is the lower corner of the Nether
    /// portal rectangle or the End platform origin. The completed blocks are
    /// already in the world.
    pub(in crate::game) fn native_portal_create(&mut self, dimension: &str, pos: [i32; 3]) {
        self.fire_portal_create(dimension, pos);
    }

    /// A raid wave has been spawned at `pos`; `wave` is its one-based wave
    /// number and `raiders` counts the raiders, not the mobs they ride.
    pub(in crate::game) fn native_raid_spawn_wave(
        &mut self,
        raid: i32,
        pos: [i32; 3],
        wave: i32,
        raiders: usize,
    ) {
        self.fire_raid_spawn_wave(raid, pos, wave, raiders);
    }

    /// A raid at `pos` changed to `state` (`triggered`, `victory`, `loss` or
    /// `stopped`). The handle identifies its owning level, even if parked.
    pub(in crate::game) fn native_raid_state_change(
        &mut self,
        raid: mod_api::RaidRef,
        pos: [i32; 3],
        state: &str,
    ) {
        self.fire_raid_state_change(raid, pos, state);
    }

    /// The server completed a world save cycle. `wait` says chunk writes were
    /// flushed before returning.
    pub(in crate::game) fn native_world_saved(&mut self, wait: bool) {
        self.fire_world_saved(wait);
    }

    /// Entities from `chunk` finished loading into the server world. `count`
    /// is the number of saved entity records read.
    pub(in crate::game) fn native_entities_load(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        count: usize,
    ) {
        self.fire_entities_load(dimension, chunk, count);
    }

    /// Entities in `chunk` left the active server world and were queued for
    /// saving. `count` is the number of entity records.
    pub(in crate::game) fn native_entities_unload(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        count: usize,
    ) {
        self.fire_entities_unload(dimension, chunk, count);
    }

    /// A guest asked to complete `command`; `suggestions` may be added to or
    /// filtered. `range` is the replaced byte range.
    pub(in crate::game) fn native_tab_complete(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        range: [usize; 2],
        suggestions: &mut Vec<Suggestion>,
    ) {
        self.fire_tab_complete(player, name, command, range, suggestions);
    }

    /// A server broadcast line is about to be logged and sent. Change `line`
    /// or set `cancel` to drop it.
    pub(in crate::game) fn native_broadcast_message(&self, line: &mut Text, cancel: &mut bool) {
        self.fire_broadcast_message(line, cancel);
    }

    /// The dedicated server's whitelist changed to `enabled`. 305.
    pub(in crate::game) fn native_whitelist_state_update(&mut self, enabled: bool) {
        self.fire_whitelist_state_update(enabled);
    }

    /// A mob entered love mode after player interaction. `player` is the
    /// feeder when known.
    pub(in crate::game) fn native_entity_enter_love_mode(
        &mut self,
        entity: u32,
        kind: &str,
        player: Option<u128>,
    ) {
        self.fire_entity_enter_love_mode(entity, kind, player);
    }

    /// A thrown splash or lingering potion applied its hit effects.
    pub(in crate::game) fn native_potion_splash(&mut self, item: &str, pos: [f64; 3]) {
        self.fire_potion_splash(item, pos);
    }

    /// A firework rocket completed its explosion effects.
    pub(in crate::game) fn native_firework_explode(
        &mut self,
        entity: u32,
        pos: [f64; 3],
        bursts: usize,
    ) {
        self.fire_firework_explode(entity, pos, bursts);
    }

    /// An experience bottle broke and its experience was awarded.
    pub(in crate::game) fn native_exp_bottle(&mut self, pos: [f64; 3], experience: u32) {
        self.fire_exp_bottle(pos, experience);
    }

    /// A player ignited a creeper with an item.
    pub(in crate::game) fn native_creeper_ignite(&mut self, entity: u32, player: u128, item: &str) {
        self.fire_creeper_ignite(entity, player, item);
    }

    /// Lightning charged a creeper.
    pub(in crate::game) fn native_creeper_power(&mut self, entity: u32, lightning: [f64; 3]) {
        self.fire_creeper_power(entity, lightning);
    }

    /// A mob broke the door at `pos`.
    pub(in crate::game) fn native_entity_break_door(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
    ) {
        self.fire_entity_break_door(entity, kind, pos);
    }

    /// A mob changed a block at `pos`, after the new state was applied.
    pub(in crate::game) fn native_entity_change_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    ) {
        self.fire_entity_change_block(entity, kind, pos, from, to);
    }

    /// A bee entered the hive at `pos` and was stored there.
    pub(in crate::game) fn native_entity_enter_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        block: &str,
    ) {
        self.fire_entity_enter_block(entity, kind, pos, block);
    }

    /// A server player finished loading the world around them after joining, respawning or changing
    /// dimension. The guest's state is not swapped in. Server-side.
    pub(in crate::game) fn native_player_client_loaded_world(&mut self, player: u128, name: &str) {
        self.fire_player_client_loaded_world(player, name);
    }
    /// A server player released the use-item key, or a mod's `LivingRef::release_using_item` did,
    /// before the item finished being used. The player's state is swapped in.
    pub(in crate::game) fn native_player_stop_using_item(&mut self, player: u128, name: &str) {
        self.fire_player_stop_using_item(player, name);
    }
    /// A server player's authoritative recipe book flags changed.
    /// Typed identity keeps custom books separate from vanilla ordinals.
    pub(in crate::game) fn native_player_recipe_book_settings_change(
        &mut self,
        player: u128,
        name: &str,
        book: mod_api::RecipeBookIdentity,
        open: bool,
        filtering: bool,
    ) {
        self.fire_player_recipe_book_settings_change(player, name, book, open, filtering);
    }
    /// A server player released a drawn bow with `arrow` selected, by the use-item
    /// key or a mod's `LivingRef::release_using_item`. The player's state is swapped in.
    pub(in crate::game) fn native_player_ready_arrow(
        &mut self,
        player: u128,
        name: &str,
        arrow: &str,
    ) {
        self.fire_player_ready_arrow(player, name, arrow);
    }
    /// A server player selected a bundle item in `slot` at `index`. The guest's
    /// state is not swapped in during packet handling.
    pub(in crate::game) fn native_player_bundle_item_select(
        &mut self,
        player: u128,
        name: &str,
        slot: i32,
        index: i32,
    ) {
        self.fire_player_bundle_item_select(player, name, slot, index);
    }
    /// A server player stopped digging `pos` before the block broke, after
    /// the server cleared the dig and sent the block back to the client.
    /// The player's state is not swapped in.
    pub(in crate::game) fn native_block_damage_abort(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
    ) {
        self.fire_block_damage_abort(player, name, pos);
    }
    /// A hopper at `pos` began an item transfer attempt. This runs after its
    /// cooldown and enabled checks, before pushing or pulling any items.
    /// Server-side, including the single-player server.
    pub(in crate::game) fn native_prepare_hopper_transfer(&mut self, pos: [i32; 3]) {
        self.fire_prepare_hopper_transfer(pos);
    }
    /// A hopper is about to move one item between containers, a composter's
    /// bone meal included. Set `cancel` to leave it in its source slot. A chest
    /// boat or minecart is reported at the block it is in.
    pub(in crate::game) fn native_inventory_move_item(
        &self,
        source: [i32; 3],
        destination: [i32; 3],
        item: &str,
        cancel: &mut bool,
    ) {
        self.fire_inventory_move_item(source, destination, item, cancel);
    }
    /// A hopper selected a container while searching for items to push or
    /// pull. Exactly one of `container` and `entity` identifies the target.
    pub(in crate::game) fn native_hopper_inventory_search(
        &mut self,
        hopper: [i32; 3],
        container: Option<[i32; 3]>,
        entity: Option<u32>,
    ) {
        self.fire_hopper_inventory_search(hopper, container, entity);
    }
    /// A vault at `pos` is about to eject a loot item.
    pub(in crate::game) fn native_vault_loot(&mut self, pos: [i32; 3], item: &str) {
        self.fire_vault_loot(pos, item);
    }
    /// A timed world-border size change finished on the server, after its
    /// final size was applied. Server-side, including the single-player
    /// server.
    pub(in crate::game) fn native_world_border_bounds_change_finish(&mut self) {
        self.fire_world_border_bounds_change_finish();
    }

    /// A server player launched a projectile item, after it was created.
    /// `item` is the projectile's held item.
    pub(in crate::game) fn native_player_launch_projectile(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
    ) {
        self.fire_player_launch_projectile(player, name, item);
    }
    /// A server explosion is about to begin at `center`, with `radius` (Bukkit's
    /// `ExplosionPrimeEvent`). This is listen-only: the explosion proceeds as
    /// usual. Server-side, including the single-player server.
    pub(in crate::game) fn native_explosion_prime(&mut self, center: [f64; 3], radius: f32) {
        self.fire_explosion_prime(center, radius);
    }
    /// A dropped item entity reached its 6000-tick lifetime and is about to be
    /// removed (Bukkit's `ItemDespawnEvent`).
    pub(in crate::game) fn native_item_despawn(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        self.fire_item_despawn(entity, item, count, pos);
    }
    /// A mob is about to drop `count` of `item` at `pos` (Bukkit's
    /// `EntityDropItemEvent`).
    pub(in crate::game) fn native_entity_drop_item(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        self.fire_entity_drop_item(entity, item, count, pos);
    }
    /// A leashed mob's leash was removed, before any lead drop is handled
    /// (Bukkit's `EntityUnleashEvent`).
    pub(in crate::game) fn native_entity_unleash(&mut self, entity: u32, kind: &str) {
        self.fire_entity_unleash(entity, kind);
    }
    /// A vehicle is about to take damage from a player's attack. `kind` is
    /// `boat` or the minecart type; mods may change `damage` or set `cancel`
    /// to leave its damage and wobble unchanged. The attack itself still
    /// completes. Server-side.
    pub(in crate::game) fn native_vehicle_damage(
        &mut self,
        kind: &str,
        damage: &mut f32,
        cancel: &mut bool,
    ) {
        self.fire_vehicle_damage(kind, damage, cancel);
    }
    /// A boat or minecart was created at `pos`, after it joined the level.
    /// `kind` is the vehicle type. Server-side.
    pub(in crate::game) fn native_vehicle_create(&mut self, kind: &str, pos: [f64; 3]) {
        self.fire_vehicle_create(kind, pos);
    }
    /// A boat or minecart is about to be destroyed at `pos`. TNT minecarts
    /// that prime instead of breaking are not heard. Server-side.
    pub(in crate::game) fn native_vehicle_destroy(&mut self, kind: &str, pos: [f64; 3]) {
        self.fire_vehicle_destroy(kind, pos);
    }
    /// A guest sent the server's current `PlayerInput` flags, after they were applied to the hub. The guest's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_input(&mut self, player: u128, name: &str, flags: i32) {
        self.fire_player_input(player, name, flags);
    }
    /// A guest's arm swing was handled by the server. `hand` is `mainhand`. Server-side.
    pub(in crate::game) fn native_player_arm_swing(
        &mut self,
        player: u128,
        name: &str,
        hand: &str,
    ) {
        self.fire_player_arm_swing(player, name, hand);
    }
    /// A guest's accepted movement made the server jump them from the ground. Server-side.
    pub(in crate::game) fn native_player_jump(&mut self, player: u128, name: &str) {
        self.fire_player_jump(player, name);
    }
    /// A guest was disconnected by a server kick, after the hub recorded the rendered `reason`. The guest's state is
    /// swapped in unless they were already removed in the same poll.
    pub(in crate::game) fn native_player_kicked(&mut self, player: u128, name: &str, reason: &str) {
        self.fire_player_kicked(player, name, reason);
    }
    /// A server player's experience level changed, after the new level was applied.
    pub(in crate::game) fn native_player_level_change(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: i32,
    ) {
        self.fire_player_level_change(player, name, from, to);
    }
    /// A click in a menu a mod opened (`open_menu`), which moved
    /// nothing: the menu's `slot` (its own first, then the player's
    /// inventory, `native_menu_inventory_slot`; -999 outside), the `button`
    /// and the click's `mode` (`ClickType`'s id) of the player `uuid`.
    /// No player's state is swapped in: the menu helpers take the player's
    /// UUID.
    pub(in crate::game) fn native_menu_clicked(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) {
        self.fire_menu_clicked(uuid, window, slot, button, mode);
    }
    /// A menu a mod opened closed: the player closed it, the server or a mod
    /// opened another, or the player left.
    pub(in crate::game) fn native_menu_closed(&mut self, uuid: u128, window: i32) {
        self.fire_menu_closed(uuid, window);
    }
    /// `Item.appendHoverText`: an item's own lines in its tooltip, after its
    /// name and before its components' (enchantments, lore, attributes). Add
    /// lines to `lines`, which arrives empty, each with a colour as `rgba` packs
    /// it (`u32::MAX` keeps the text's own); `Text::legacy` reads `§` codes.
    /// The stack has its components (`stack.data()`). Asked each frame the
    /// tooltip shows, never for one the stack hides or a crossbow's loaded
    /// projectile.
    pub(in crate::game) fn native_tooltip(&self, stack: &ItemStack, lines: &mut Vec<(Text, u32)>) {
        self.fire_tooltip(stack, lines);
    }
    /// Client-side image component for this stack's tooltip. Answer its width/height
    /// in GUI pixels; finite positive dimensions reserve space after text and any
    /// vanilla bundle component. None keeps the vanilla tooltip unchanged.
    pub(in crate::game) fn native_tooltip_component(
        &self,
        stack: &ItemStack,
        size: &mut Option<[f32; 2]>,
    ) {
        self.fire_tooltip_component(stack, size);
    }
    /// Render the measured component at origin, inside the tooltip background.
    /// Stack is the component's data. Use native_sprite/native_fill/native_text;
    /// size is the measurement returned by native_tooltip_component this frame.
    pub(in crate::game) fn native_tooltip_component_draw(
        &self,
        stack: &ItemStack,
        draw: &mut HudBatch,
        origin: [f32; 2],
        size: [f32; 2],
    ) {
        self.fire_tooltip_component_draw(stack, draw, origin, size);
    }

    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (width `0..=13`, colour `rgb`) or set to `None` to hide it.
    /// Asked each frame a stack is drawn. Client-side.
    pub(in crate::game) fn native_item_bar(&self, stack: &ItemStack, bar: &mut Option<(i32, u32)>) {
        self.fire_item_bar(stack, bar);
    }
    /// Append an item's custom decorations after its default bar, cooldown and count.
    /// Coordinates are the icon's top-left in GUI units, as the batch takes them;
    /// no item translation is active.
    /// Runs wherever the shared item-decoration path is used (hotbar, menus, tooltips,
    /// and the other screens that draw item icons: advancements, beacon, stats, trading).
    pub(in crate::game) fn native_item_decorations(
        &self,
        stack: &ItemStack,
        x: f32,
        y: f32,
        draw: &mut HudBatch,
    ) {
        self.fire_item_decorations(stack, x, y, draw);
    }
    /// `ItemStack.useOn`: both logical sides answer after the block passed.
    /// Client reads and local effects cannot perform authoritative writes.
    pub(in crate::game) fn native_use_on(&mut self, call: mod_api::__private::UseOnCall<'_>) {
        self.fire_use_on(call);
    }
    /// `DispenserBlock.getDispenseMethod`: choose a behaviour for any item,
    /// including vanilla items, on the authoritative server (not droppers).
    /// The block the dispenser faces is asked first (`dispense_method`), and
    /// its answer keeps this part from being asked. Leave answer None for
    /// vanilla. Otherwise answer `(remainder, event)`:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the hook
    /// owns consumption: no extra item is removed afterwards). `event` is the
    /// level event the dispenser plays with its smoke: 1000 the click, 1001 the
    /// fail sound (`OptionalDispenseItemBehavior` without success), or 0 for
    /// nothing at all (`DispenseItemBehavior.NOOP`). To drop one normally,
    /// answer `(dispense_default(..), 1000)`.
    pub(in crate::game) fn native_dispense_item(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
    ) {
        self.fire_dispense_item(dimension, pos, facing, stack, answer);
    }
    /// A player uses `call.stack` with this click's `call.hand`: in the air, or
    /// on a block once `native_use_on` let it through, where `Item.use`
    /// starts. Both sides return the full interaction result; Pass continues.
    /// Consume handles without swinging. Cooldown items are skipped.
    pub(in crate::game) fn native_use_item(&mut self, call: mod_api::__private::UseItemCall<'_>) {
        self.fire_use_item(call);
    }
    /// `Item.getUseDuration` of `stack`, in ticks: `duration` arrives with
    /// vanilla's (a consumable's time, 72000 for a shield, a spear, a bow,
    /// 0 for an item with no use over time). A mod's own item (outside
    /// `minecraft:`) with no `consumable`, `blocks_attacks` or kinetic
    /// weapon that answers above 0 is used over time on a right click, as an
    /// `Item.use` that calls `startUsingItem`, until it is let go or the time
    /// runs out (`native_living_entity_use_item_finish`). A consumable's time
    /// changes too, potions and the other vanilla items keep their own.
    /// Asked when a use starts, each tick a mod's item is in use, and each
    /// frame the hand draws one in use, on both sides. It has no user, as
    /// it is asked so often: answer from the stack alone.
    pub(in crate::game) fn native_use_duration(&self, stack: &ItemStack, duration: &mut i32) {
        self.fire_use_duration(stack, duration);
    }
    /// `Item.getUseAnimation` of `stack` in use, with the item's vanilla
    /// answer as the starting animation. Asked on the client each first-person
    /// frame and each local or remote player model tick. A spear override is
    /// refused if the item has no kinetic data.
    pub(in crate::game) fn native_use_animation(
        &self,
        stack: &ItemStack,
        animation: &mut UseAnimation,
    ) {
        self.fire_use_animation(stack, animation);
    }
    /// `Player.canEat` for held `stack` (None for an empty hand) for food and cake.
    /// Cake uses `can_always_eat = false`; `vanilla_can_eat` is vanilla's answer (creative,
    /// `can_always_eat`, or hungry). Answer `Some(false)` to refuse the food,
    /// `Some(true)` to let a full player eat; `can_eat` left `None` keeps
    /// `vanilla_can_eat`. Both sides (the client starts eating as the
    /// server does), so answer from the stack and the player alone.
    pub(in crate::game) fn native_can_eat(
        &self,
        stack: Option<&ItemStack>,
        vanilla_can_eat: bool,
        can_eat: &mut Option<bool>,
    ) {
        self.fire_can_eat(stack, vanilla_can_eat, can_eat);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Start`: `entity` starts using
    /// `stack` in `hand` (`LivingEntity.startUsingItem`), for `duration`
    /// ticks (`native_use_duration`'s answer). `entity` is a mob's
    /// `Entity.getId`, None for a player (`native_acting_player` says who);
    /// a mob fires it only for `LivingRef::start_using_item`. Change `duration` (consumables, potions,
    /// spyglasses and mods' items count to it; below 0 refuses the use, and
    /// 0 runs it out on its first tick, as `native_living_entity_use_item_tick`
    /// running it out, unless the item's own is 0 already), or set
    /// `canceled` and the use doesn't start. Both sides, as
    /// vanilla's: on the server for every player of the world, and on a
    /// server's client for the uses it predicts. Answer the same on both,
    /// and change the world only on the server (`native_window` is false
    /// there), not on a server's client.
    pub(in crate::game) fn native_living_entity_use_item_start(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: &mut i32,
        canceled: &mut bool,
    ) {
        self.fire_living_entity_use_item_start(stack, entity, hand, duration, canceled);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Finish`: `entity`'s use of
    /// `stack` in `hand` ran its time (`LivingEntity.completeUsingItem`),
    /// `duration` the ticks left, always 0. `entity` is as in
    /// `native_living_entity_use_item_start`. `result` arrives with what
    /// `finishUsingItem` left in the hand (the bowl of a stew, none for the
    /// last apple; a mod's own item is kept) and may be replaced. Fired for
    /// consumables, potions, spyglasses and mods' items. Both sides (a server's client sees its prediction);
    /// on the server for every player of the world (the helpers read the acting one).
    pub(in crate::game) fn native_living_entity_use_item_finish(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        result: &mut Option<ItemStack>,
    ) {
        self.fire_living_entity_use_item_finish(stack, entity, hand, duration, result);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Tick`: each tick `entity` uses
    /// `stack` in `hand`, where `LivingEntity.updatingUsingItem` starts,
    /// `duration` the ticks left. `entity` is as in
    /// `native_living_entity_use_item_start`. Change `duration` to move the
    /// end (consumables, potions, spyglasses and mods' items count to it,
    /// and so do a client's eating crumbs and first person pose, as the
    /// started duration of `native_living_entity_use_item_start` does). A
    /// cancel sets it to -1. Once a mod set it, the use goes on
    /// only while it is above 0: at 0 or less, by a cancel or set, the
    /// item's tick is skipped and the server finishes the use
    /// (`completeUsingItem`: `finishUsingItem`, the food eaten,
    /// `native_living_entity_use_item_finish`), while an item used on
    /// release (a crossbow) stays frozen until let go. NeoForge's javadoc
    /// says a cancel stops the use, but its code finishes it; this follows
    /// the code. On a server's client a run-out use stays in use, frozen,
    /// so letting go still tells the server. Both sides, as vanilla's
    /// `updatingUsingItem`: answer the same on both, and change the world
    /// only on the server (`native_window` is false there), not on a
    /// server's client. Runs every tick of every use:
    /// check the item first.
    pub(in crate::game) fn native_living_entity_use_item_tick(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: &mut i32,
        canceled: &mut bool,
    ) {
        self.fire_living_entity_use_item_tick(stack, entity, hand, duration, canceled);
    }
    /// `Item.onUseTick` of `stack`, used by `entity` with `remaining` ticks
    /// left, always above 0: what a mod's own item does each tick it is used
    /// (cook, charge, drain). It comes where `ItemStack.onUseTick` calls the
    /// item, once `native_living_entity_use_item_tick` let the use go on:
    /// after a consumable's crumbs, for consumables, shields and mods' items
    /// (vanilla's other items do their own, and a kinetic weapon has none on
    /// the server). Both sides, as `native_living_entity_use_item_tick`:
    /// change the world only where this `Game` runs the world, not on a
    /// server's client. Runs every tick of every use: check the item first.
    pub(in crate::game) fn native_on_use_tick(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        remaining: i32,
    ) {
        self.fire_on_use_tick(stack, entity, remaining);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Stop`: `entity` lets go of
    /// `stack` in `hand` before its use ran its time
    /// (`LivingEntity.releaseUsingItem`: the use button let go, a screen
    /// opened), `duration` the ticks left. Set `canceled` to skip the item's
    /// release (a bow doesn't shoot, a spyglass makes no sound,
    /// `native_release_using` isn't called); the use stops anyway. Not when
    /// the hand changed item, which stops the use without a release. Both
    /// sides, as `native_living_entity_use_item_tick`.
    pub(in crate::game) fn native_living_entity_use_item_stop(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        canceled: &mut bool,
    ) {
        self.fire_living_entity_use_item_stop(stack, entity, hand, duration, canceled);
    }
    /// NeoForge's `ArrowLooseEvent`: the player releases a bow `stack` with
    /// `has_ammo` and `charge` ticks held. Change `charge` or set `canceled`
    /// to prevent the shot; negative charge also prevents it. Unlike
    /// `native_living_entity_use_item_stop`, this does not skip the whole item
    /// release; `native_entity_shoot_bow` reports a spawned arrow afterward
    /// with read-only force. Fires on the Game running the bow release, including
    /// LAN guests; a remote client's own release path is not covered. Crossbows
    /// are not covered; `native_arrow_nock` is the bow's start.
    pub(in crate::game) fn native_arrow_loose(
        &mut self,
        stack: &ItemStack,
        has_ammo: bool,
        charge: &mut i32,
        canceled: &mut bool,
    ) {
        self.fire_arrow_loose(stack, has_ammo, charge, canceled);
    }
    /// NeoForge's `CriticalHitEvent`: a player's melee hit on a living
    /// `target_kind` (`minecraft:zombie`, `minecraft:armor_stand`) that deals
    /// damage, where `Player.attack` decides whether it is critical.
    /// `vanilla_critical` is vanilla's answer; `critical` starts as it and may
    /// be forced or denied (as in vanilla a critical hit prevents the sweep, so
    /// a forced crit loses it; a denied crit loses the crit sound and
    /// particles); `damage_multiplier` starts at 1.5 for vanilla's critical and
    /// 1.0 otherwise, and scales the damage only while `critical` ends true. Not fired for a
    /// target that is no living entity (a dragon part) or when the hit does
    /// no damage. Fires on the Game that runs the attack.
    pub(in crate::game) fn native_critical_hit(
        &mut self,
        target_kind: &str,
        vanilla_critical: bool,
        critical: &mut bool,
        damage_multiplier: &mut f32,
    ) {
        self.fire_critical_hit(target_kind, vanilla_critical, critical, damage_multiplier);
    }
    /// `Item.releaseUsing` of `stack`: `entity` let it go with `time_left`
    /// ticks of its `native_use_duration` left, so it was held for
    /// `duration - time_left` (a bow-like item shoots by that charge). Set
    /// `used` as vanilla's return, and the stack's use cooldown starts
    /// (`ItemStack.releaseUsing`). After the vanilla item's own release, once
    /// `native_living_entity_use_item_stop` let it through. Both sides, as
    /// vanilla's: change the world only where this `Game` runs the world,
    /// not on a server's client.
    pub(in crate::game) fn native_release_using(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        time_left: i32,
        used: &mut bool,
    ) {
        self.fire_release_using(stack, entity, time_left, used);
    }
    /// A player breaks the block `state` at `pos`, once the game allowed it
    /// (the held item can break blocks, the debug stick didn't take the
    /// click): where `ServerPlayerGameMode.destroyBlock` starts. Set
    /// `canceled` to keep the block: nothing is destroyed or dropped, and a
    /// guest sees it back. Server-side, for every player of the world (the
    /// helpers read the acting one).
    pub(in crate::game) fn native_break_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
        exp_to_drop: &mut Option<u32>,
    ) {
        self.prepare_part_break_experience(pos, state, exp_to_drop);
        let initial = *exp_to_drop;
        self.fire_break_block(pos, state, canceled, exp_to_drop);
        *exp_to_drop = exp_to_drop.or(initial);
    }
    /// A player right clicks a block (NeoForge's
    /// `PlayerInteractEvent.RightClickBlock`): once per hand, where
    /// `useItemOn` starts, on the clicking client and on the server (`click`
    /// says which, with the block, the hit, the hand and its stack, and the
    /// player). Before the block's own use (a chest, a door, a lever) and the
    /// item's (`Item.useOn`, `native_use_on`).
    ///
    /// `use_block`: False skips; True overrides sneak; Default skips when sneaking with either hand full (the both-stacks sneak-bypass exception remains open in #2934). `use_item`: False skips; True forces
    /// even an empty hand or cooldown; Default requires a held stack off cooldown. Empty hands reach
    /// ItemAfterBlock with no stack, but no item Part. Adventure restrictions still apply; block consumption ends the click. Cancelling
    /// skips both and answers `cancellation_result` in their place:
    /// `Pass` (the default) goes on to the item's own use (`Item.use`) and
    /// then the other hand, `Fail` ends the click, and `Success`,
    /// `SuccessServer` or `Consume` end it as used, with the swing they name.
    /// A mod that keeps players out of land cancels with `Fail`.
    pub(in crate::game) fn native_right_click_block(
        &mut self,
        click: &BlockClick<'_>,
        use_block: &mut TriState,
        use_item: &mut TriState,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    ) {
        self.fire_right_click_block(click, use_block, use_item, cancellation_result, canceled);
    }
    /// A player fills (`stack` an empty bucket) or empties a bucket at `pos`:
    /// `BucketItem.use`, where it asks `Level.mayInteract`. Set `canceled`
    /// and the bucket does nothing. Server-side, for every player of the
    /// world (`native_acting_player` says who).
    pub(in crate::game) fn native_fill_bucket(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        canceled: &mut bool,
    ) {
        self.fire_fill_bucket(stack, pos, canceled);
    }
    /// A player attacks the entity `entity` (`Entity.getId`) of type `kind`
    /// (`minecraft:item_frame`, `minecraft:cow`, `minecraft:player`) standing
    /// at `pos`: `ServerGamePacketListenerImpl.handleInteract`'s attack, once
    /// in reach, where `Player.attack` starts. The swing has not begun, so the
    /// attacking player's attack strength is still the charge the blow uses
    /// (vanilla reads it at the start of `Player.attack`; the swing resets it
    /// afterwards). Set `canceled` and the blow does nothing (no damage, no
    /// knockback, the frame keeps its item). Server-side, for every player of
    /// the world (`native_acting_player` says who).
    pub(in crate::game) fn native_attack_entity(
        &mut self,
        entity: i32,
        kind: &str,
        pos: [f64; 3],
        canceled: &mut bool,
    ) {
        self.fire_attack_entity(entity, kind, pos, canceled);
    }
    /// `dimension`'s level opened on the server (NeoForge's `LevelEvent.Load`,
    /// posted in `MinecraftServer.createLevels`): before any of its chunks
    /// (`native_chunk_load`), so a mod sets up what it keeps for the level
    /// here. The level the world opens in comes before `WorldOpenReady`; the
    /// others open when someone first goes there (a player, a portal, an
    /// `execute in` command), with the level in place. Once for each open:
    /// `native_level_unload` follows. Server-side.
    pub(in crate::game) fn native_level_load(&mut self, dimension: &str) {
        self.fire_level_load(dimension);
    }
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`), after its last save:
    /// drop what the mod kept for it. A level other than the Overworld closes
    /// once nobody has been in it for 300 ticks, after its chunks' unloads;
    /// every level still open closes when the server stops, before
    /// `WorldCloseFinal`, and when its open or its loop fails, before
    /// `WorldOpenFailed` or `WorldCloseFailed`. Once for each
    /// `native_level_load`. Don't read its blocks. Server-side.
    pub(in crate::game) fn native_level_unload(&mut self, dimension: &str) {
        self.fire_level_unload(dimension);
    }
    /// The server's difficulty changed, from `from` to `to` (`peaceful`,
    /// `easy`, `normal`, `hard`; NeoForge's `DifficultyChangeEvent`). It is
    /// vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a
    /// dedicated server's management and a test environment's difficulty all
    /// go through, and it is told only when the difficulty is another one. It
    /// only hears: the change has already happened. Server-side.
    pub(in crate::game) fn native_difficulty_changed(&mut self, from: &str, to: &str) {
        self.fire_difficulty_changed(from, to);
    }
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into the world
    /// (NeoForge's `ChunkEvent.Load`): `new` when it was just generated, false
    /// when read from the save. A flat world's chunk that leaves before it's
    /// first lit isn't saved, so it's generated, and new, again; one generated
    /// as the players left is new when it next comes, unless the server
    /// restarts first, or the level closes for being empty, which drops its
    /// memory of what never saved. Its blocks and block entities are there,
    /// but its saved entities join afterwards and its light may not be ready:
    /// change the world from a later tick. The chunks around the spawn come
    /// before `WorldOpenReady`. Not told of a chunk a client receives.
    /// Server-side.
    pub(in crate::game) fn native_chunk_load(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        new: bool,
    ) {
        self.fire_chunk_load(dimension, chunk, new);
    }
    /// `player` (`name`) joined the game (NeoForge's
    /// `PlayerLoggedInEvent`): placed in the world and listed in
    /// `host.players().server_players()`, so `PlayerRef::send_payload` can reach them.
    /// Once a join, with no player's state swapped in. Server-side.
    pub(in crate::game) fn native_player_logged_in(&mut self, player: u128, name: &str) {
        self.fire_player_logged_in(player, name);
    }
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `native_player_logged_in` told of. They've already
    /// left `native_server_players`, except when the server stops, which tells
    /// before it saves, with them still listed and able to get a last message.
    /// Server-side.
    pub(in crate::game) fn native_player_logged_out(&mut self, player: u128, name: &str) {
        self.fire_player_logged_out(player, name);
    }
    /// `PlayerRespawnPositionEvent`: where `player` respawns, once the server found it. The
    /// `original_*` arguments are that point; the mods may change `level`, `position`, `yaw`,
    /// `pitch` and `copy_original_spawn_position` (`game/world/respawn_position.rs` applies them).
    pub(in crate::game) fn native_player_respawn_position(
        &mut self,
        call: mod_api::__private::PlayerRespawnPositionCall<'_>,
    ) {
        self.fire_player_respawn_position(call);
    }
    /// `player` (`name`) respawned (NeoForge's `PlayerRespawnEvent`): at the
    /// end of `PlayerList.respawn`, once they are placed and their health,
    /// food and experience reset, with their state swapped in. Once for each
    /// death that ends in a respawn, on a server; not for the End-exit one.
    pub(in crate::game) fn native_player_respawned(&mut self, player: u128, name: &str) {
        self.fire_player_respawned(player, name);
    }
    /// A player was saved to the world's store (NeoForge's
    /// `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the
    /// world closes, when a dedicated or integrated server saves its guests,
    /// and for an integrated server's owner when the world saves them.
    /// Once for each save. `directory`
    /// is the world's player directory (`players/data`): write your own file
    /// there, named `<uuid>.<suffix>`, never the player's `.dat`. The player
    /// may be leaving, so no player's state is swapped in. Not cancellable.
    /// Autosave snapshots are written asynchronously: this notification runs
    /// on a later server tick after a successful write, possibly after WorldSaved.
    /// Live state may have advanced beyond the saved snapshot; a process exit
    /// before completions are drained can prevent notification delivery.
    pub(in crate::game) fn native_player_save_to_file(
        &mut self,
        player: u128,
        name: &str,
        directory: &std::path::Path,
    ) {
        self.fire_player_save_to_file(player, name, directory);
    }
    /// A player was loaded from the world's save (NeoForge's
    /// `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`): their `.dat`
    /// has been read and applied, and they are not placed in a level yet.
    /// `directory` is the world's player directory (`players/data`): a mod
    /// reads its own file there, named `<uuid>.<suffix>` as
    /// `getPlayerFile(suffix)` names it. Heard once for each join, for the
    /// owner of an integrated server as for anyone else, on a server, with no
    /// player's state swapped in. Not cancellable.
    pub(in crate::game) fn native_player_load_from_file(
        &mut self,
        player: u128,
        name: &str,
        directory: &std::path::Path,
    ) {
        self.fire_player_load_from_file(player, name, directory);
    }
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). `original` reads the old player's
    /// complete data until this hook returns; `player` names the replacement.
    /// Heard before the new player is placed, on a server. Not cancellable.
    pub(in crate::game) fn native_player_clone(
        &mut self,
        player: u128,
        name: &str,
        was_death: bool,
        original: mod_api::OriginalPlayerRef,
    ) {
        self.fire_player_clone(player, name, was_death, original);
    }
    /// The number of players with the mod container at `pos` of `dimension`
    /// open went from `previous` to `viewers` (vanilla's
    /// `ContainerOpenersCounter`, as a barrel keeps it): 0 to more than 0 is
    /// `onOpen`, back to 0 is `onClose`, any change `openerCountChanged`, so a
    /// block can show itself open while `viewers` is not 0. Counted every
    /// tick from the players' open menus, so a player who leaves without
    /// closing (a lost connection, death, another dimension, out of reach, the
    /// block gone) is heard too (`recheckOpeners`). Once for each change, with
    /// no player's state swapped in. Server-side.
    pub(in crate::game) fn native_mod_container_viewers(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        viewers: u32,
        previous: u32,
    ) {
        self.fire_mod_container_viewers(dimension, pos, viewers, previous);
    }
    /// `player` (`name`) opened a menu (NeoForge's `PlayerContainerEvent.Open`,
    /// posted when `ServerPlayer.openMenu` sets the new `containerMenu`): any
    /// menu, vanilla or a mod's. `menu` is the menu type's name
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:merchant`,
    /// `minecraft:crafting`, `minecraft:lectern`, a mod's own), `pos` the
    /// block it is of in `dimension` (none for a trade, a vehicle's, a mod's
    /// native menu or a mount's inventory, which has no menu type in vanilla
    /// and is told as `horizon:mount_inventory`). The player's own inventory
    /// is not one, nor is failed construction. Heard synchronously after construction, the open packet
    /// is queued and full contents sent, with the new menu already current.
    /// Replacing a menu follows its close; even inside one tick, open precedes
    /// `native_player_container_close`. The player's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_container_open(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        menu: &str,
        pos: Option<[i32; 3]>,
    ) {
        self.fire_player_container_open(player, name, dimension, menu, pos);
    }
    /// `player` (`name`) closed the menu `native_player_container_open` told
    /// of (NeoForge's `PlayerContainerEvent.Close`, posted in
    /// `ServerPlayer.doCloseContainer` after the menu's `removed`): they
    /// closed it, walked out of reach, died, changed level, or another menu
    /// opened. A player who leaves closes it before `native_player_logged_out`
    /// (vanilla posts nothing for a disconnect; Horizon does, so each open has
    /// its close). Once for each open; not told when the server stops. The
    /// menu's items are where it left them: the block's, and the cursor stack
    /// back in the inventory. State swapped in, when the player has a body.
    /// Server-side.
    pub(in crate::game) fn native_player_container_close(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        menu: &str,
        pos: Option<[i32; 3]>,
    ) {
        self.fire_player_container_close(player, name, dimension, menu, pos);
    }
    /// A client started tracking an entity (NeoForge's
    /// `PlayerEvent.StartTracking`, posted at the end of
    /// `ServerEntity.addPairing`): `player` (`name`) is now sent `entity`
    /// (its ID, `uuid` its UUID, `kind` its type, `minecraft:zombie`) and its
    /// changes. It comes into range, its client's chunks reach it, or it is
    /// put in the level; another player is an entity too, and a player never
    /// tracks their own. A mod sends the player what they need to show it
    /// (`PlayerRef::send_payload`). The entity is as it was last tick: it
    /// may be gone by now. Heard after the tick's players and entities have
    /// moved, in order, with no player's state swapped in. Server-side.
    pub(in crate::game) fn native_player_start_tracking(
        &mut self,
        player: u128,
        name: &str,
        entity: u32,
        uuid: u128,
        kind: &str,
    ) {
        self.fire_player_start_tracking(player, name, entity, uuid, kind);
    }
    /// A client stopped tracking an entity (NeoForge's
    /// `PlayerEvent.StopTracking`, posted at the end of
    /// `ServerEntity.removePairing`): it went out of range, left the level or
    /// died, or `player` (`name`) left, changed level or respawned, which
    /// stops everything they tracked. `entity`, `uuid` and `kind` are as
    /// `native_player_start_tracking` told. Once for each start, a leaving
    /// player's after `native_player_logged_out`. The entity is not
    /// necessarily still there, and a player who left is gone: no player's
    /// state is swapped in. Server-side.
    pub(in crate::game) fn native_player_stop_tracking(
        &mut self,
        player: u128,
        name: &str,
        entity: u32,
        uuid: u128,
        kind: &str,
    ) {
        self.fire_player_stop_tracking(player, name, entity, uuid, kind);
    }
    /// Whether a mod listens to `native_living_equipment_change` for mobs and
    /// connected players: a mod that hooks it hooks this one too and sets
    /// `listened` (HEAD, `*listened = true`). Without it the game does not read
    /// their equipment each tick and only armor stands are told. A mob or player
    /// counts from empty once it is set. Asked once a tick. Server-side.
    pub(in crate::game) fn native_living_equipment_listened(&mut self, listened: &mut bool) {
        self.fire_living_equipment_listened(listened);
    }
    /// A mob's, an armor stand's or a connected player's equipment slot took a
    /// different stack (NeoForge's `LivingEquipmentChangeEvent`,
    /// `LivingEntity.collectEquipmentChanges`): `entity` its ID, `kind` its type
    /// (`minecraft:player` for a player), `slot` the `EquipmentSlot`, `from` what
    /// the slot held when last heard and `to` what it holds now. Once per changed
    /// slot, after the mobs' tick; one new to the level counts from empty. A
    /// player's `mainhand` is the selected hotbar slot; their `body` and `saddle`
    /// are never told. The world host's own player is heard by `native_on_equip`.
    /// It only hears. Server-side.
    pub(in crate::game) fn native_living_equipment_change(
        &mut self,
        entity: u32,
        kind: &str,
        slot: &str,
        from: Option<&ItemStack>,
        to: Option<&ItemStack>,
    ) {
        self.fire_living_equipment_change(entity, kind, slot, from, to);
    }
    /// `player` died: where `ServerPlayer.die` starts, with the death
    /// message's key when the game knows it (`death.attack.lava`; none for a
    /// message it builds from the combat). It only hears. Server-side, for
    /// every player of the world, whether death messages are shown or not.
    pub(in crate::game) fn native_player_died(&mut self, player: u128, cause: Option<&str>) {
        self.fire_player_died(player, cause);
    }
    /// A new mob of `kind` joined `dimension` at `pos`: `natural` when the
    /// world's spawning made it, false when something else did (a breed, a
    /// raid, a spawner, a conversion, `/summon`, and the second mob of a
    /// jockey the world spawned). One call a mob: a `/summon` with passengers
    /// tells each. Not told of a mob loaded with its chunk, leaving a hive or
    /// changing level. It only hears; the mob is already among the game's
    /// mobs. Server-side.
    pub(in crate::game) fn native_mob_spawned(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        natural: bool,
    ) {
        self.fire_mob_spawned(dimension, kind, pos, natural);
    }
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. 53 told item entities, experience orbs and mobs; it now
    /// tells every kind `native_entity_join_level` does (`kind` is the type), each
    /// once, when the game next notices it gone (the tick or action that removed
    /// it). Check `kind`. Server-side.
    pub(in crate::game) fn native_entity_leave_level(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
    ) {
        self.fire_entity_leave_level(dimension, entity, kind);
    }
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back with its chunk,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `entity` is its ID and `kind` its type
    /// (`minecraft:item`). It is already in the level's lists, so helpers that
    /// take an entity ID find it. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. Tells every
    /// kind in the game's entity lists (items, orbs, mobs, boats, minecarts,
    /// armor stands, primed TNT, falling blocks, projectiles, rockets,
    /// tridents, end crystals, the dragon, cushions, paintings, item frames),
    /// once each, in the order they were created; check `kind`. A drop, an
    /// orb, a mob, a `/summon` and a chunk's entities are told at once, any
    /// other spawn between the tick's phases (it is checked after the blocks,
    /// the vehicles, the projectiles, the items and the mobs, and at the
    /// tick's end). A mod that cancels one of the others sees it removed
    /// before it ticks again. Not told: the entities Horizon keeps without
    /// simulating (displays, markers, interactions). Server-side.
    pub(in crate::game) fn native_entity_join_level(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
        canceled: &mut bool,
    ) {
        self.fire_entity_join_level(dimension, entity, kind, pos, loaded_from_disk, canceled);
    }
    /// An entity mounts or dismounts a vehicle (NeoForge's `EntityMountEvent`, posted from
    /// `Entity.startRiding` and `removeVehicle`): `entity` of type `kind` (`minecraft:player`,
    /// `minecraft:zombie`) in the server level `dimension`, getting on (`mounting`) or off the
    /// `vehicle` of type `vehicle_kind` (`minecraft:oak_boat`, `minecraft:minecart`,
    /// `minecraft:horse`, `minecraft:cushion`). Set `canceled` and it does not: a player stays
    /// where they were, a mob does not board, a `/ride` fails, and a rider whose dismount is
    /// canceled keeps riding, including during vehicle destruction. A removed
    /// boat or cart is no longer a valid handle. Explicit later dismounts notify
    /// again; resident riders also retry in entity-ticking chunks, while rejected
    /// passengers do not tick. Loading a save does not notify. Server-side.
    pub(in crate::game) fn native_entity_mount(
        &mut self,
        call: mod_api::__private::EntityMountCall<'_>,
    ) {
        self.fire_entity_mount(call);
    }
    /// Whether a mod listens to `native_entity_tick_pre` and `native_entity_tick_post`: a mod
    /// that hooks them hooks this one too and sets `listened` (HEAD, `*listened = true`).
    /// Without it the game tells no entity's tick, and keeps no list of them. Asked once for
    /// each family (mobs, items, minecarts, boats) each tick. Server-side.
    pub(in crate::game) fn native_entity_tick_listened(&mut self, listened: &mut bool) {
        self.fire_entity_tick_listened(listened);
    }
    /// An entity of type `kind` (`minecraft:zombie`, `minecraft:item`) with ID `entity` is
    /// about to tick in the server level `dimension` (NeoForge's `EntityTickEvent.Pre`, posted
    /// around `Entity.commonTick` and `tick` in `ServerLevel.tickNonPassenger`). Set `canceled`
    /// and it does not tick this tick: a mob has no AI, physics or effects, an item no movement,
    /// ageing or merging, a minecart or boat no motion, and `native_entity_tick_post` is not told.
    /// Horizon ticks a family of entities together, so mobs and items hear every Pre before the
    /// first of them ticks; minecarts and boats hear theirs one by one. Told for the mobs, item
    /// entities, minecarts and boats that tick: those in an entity-ticking chunk, not a player's
    /// boat or cart (it ticks with the player). Check `kind`; this runs for every ticking entity
    /// each tick, so keep it cheap. Server-side.
    pub(in crate::game) fn native_entity_tick_pre(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
        canceled: &mut bool,
    ) {
        self.fire_entity_tick_pre(dimension, entity, kind, canceled);
    }
    /// The entity `entity` of type `kind` ticked in the server level `dimension` (NeoForge's
    /// `EntityTickEvent.Post`): for each entity `native_entity_tick_pre` let through, after it
    /// ticked, also one that died or was removed in its tick. Mobs and items hear their Posts
    /// once the whole family has ticked; minecarts and boats right after their own tick. Server-side.
    pub(in crate::game) fn native_entity_tick_post(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
    ) {
        self.fire_entity_tick_post(dimension, entity, kind);
    }
    /// Whether the experience orb `into` takes the orb `from` in (vanilla's
    /// `ExperienceOrb.canMerge`): the same value, and ids a multiple of 40 apart. Asked
    /// for each pair whose boxes meet, when an orb spawns (`tryMergeToExisting`: `from` is
    /// 0, the new orb isn't an entity yet, and only `from_value` is known) and every 20
    /// ticks as an orb scans for neighbours (`scanForMerges`). `can_merge` holds vanilla's
    /// answer; change it either way. A merged orb counts as `count` orbs of `into`'s
    /// value, so forcing orbs of different values together drops points unless the mod
    /// raises `into`'s value with `set_experience_orb` in `native_experience_orb_merge`.
    /// Server-side.
    pub(in crate::game) fn native_experience_orb_can_merge(
        &mut self,
        dimension: &str,
        into: u32,
        from: u32,
        into_value: i32,
        from_value: i32,
        can_merge: &mut bool,
    ) {
        self.fire_experience_orb_can_merge(
            dimension, into, from, into_value, from_value, can_merge,
        );
    }
    /// The experience orb merge hook's entry point: its arguments are declared on the hook.
    pub(in crate::game) fn native_experience_orb_merge(
        &mut self,
        call: mod_api::__private::ExperienceOrbMergeCall<'_>,
    ) {
        self.fire_experience_orb_merge(call);
    }
    /// A totem of undying saved the server player `player` (`name`): their
    /// hurt would have killed them and the totem held in either hand fired
    /// (`LivingEntity.checkTotemDeathProtection`; Bukkit's
    /// `EntityResurrectEvent`), heard just before it is used up and its
    /// effects apply. Listen only: whether the totem saves is decided inside
    /// the health state machine, before a mod is asked, so a mod cannot veto
    /// it from here. The player's state is swapped in, so
    /// `native_acting_player`, `native_tell` and the item helpers reach them.
    /// Once for each totem, on a server; the host of a LAN world's totem is
    /// not heard.
    pub(in crate::game) fn native_player_totem(&mut self, player: u128, name: &str) {
        self.fire_player_totem(player, name);
    }
    /// The item `item` of the server player `player` (`name`) broke in `slot`
    /// (`mainhand`, `offhand`, `head`, `chest`, `legs`, `feet`, `body` or
    /// `saddle`): `LivingEntity.onEquippedItemBroken`, Bukkit's
    /// `PlayerItemBreakEvent`. Listen only; armour has a slot, not a hand.
    /// The player's state is swapped in. Once for each break, on a server; a
    /// creative player and the host of a LAN world are not heard.
    pub(in crate::game) fn native_player_destroy_item(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        slot: &str,
    ) {
        self.fire_player_destroy_item(player, name, item, slot);
    }
    /// `player`'s melee blow killed the mob `entity` of `kind` (other deaths
    /// aren't told yet). It only hears, after the blow landed. Server-side.
    pub(in crate::game) fn native_mob_killed(&mut self, player: u128, entity: u32, kind: &str) {
        self.fire_mob_killed(player, entity, kind);
    }
    /// A block broke or a mob died, and `drops` are the items about to drop, in
    /// the order they drop: what the loot tables rolled (after
    /// `native_loot_table_load`'s changes), and for a mob what it wore, held or
    /// carried (a fox's mouth item, an allay's, piglin's or horse's inventory),
    /// but not an enderman's carried block or a copper golem's preserved
    /// equipment, or custom drops carrying item-entity state such as an initial
    /// age, which join the later living-drops collection instead.
    /// A charged creeper's head and a fox's mouth
    /// item come before the mob's own table. A block's stacks get its block
    /// entity's data (a shulker box's items) after this. Add, change or take out
    /// stacks; empty ones don't drop (NeoForge's global loot modifiers, Fabric's
    /// `LootTableEvents.MODIFY_DROPS`); `context` says what dropped them and
    /// how, and it is called even when `drops` is empty. A player's use drops
    /// through it too (`LootDrops::used`: shears, a brush, an item
    /// frame), with that player named. It hears blocks that a
    /// player, an explosion or a mob breaks, not other breaks (leaves decaying,
    /// a torch losing its wall, a piston, `/setblock ... destroy`), nor a
    /// creative break or a tool that can't harvest the block, nor experience.
    /// Server-side, for every player.
    pub(in crate::game) fn native_loot_drops(
        &mut self,
        context: &LootDrops<'_>,
        drops: &mut Vec<ItemStack>,
    ) {
        self.fire_loot_drops(context, drops);
    }
    /// A dropped item reaches its lifetime on the server (`ItemEntity.tick`,
    /// NeoForge's `ItemExpireEvent`). Every mod may set or add to `extra_life`,
    /// in ticks, which starts at zero; a positive total keeps the item that much
    /// longer, zero or less lets it expire. Extensions are capped at 32766
    /// ticks.
    pub(in crate::game) fn native_item_expire(
        &mut self,
        entity: u32,
        stack: &ItemStack,
        extra_life: &mut i32,
    ) {
        self.fire_item_expire(entity, stack, extra_life);
    }
    /// The food level of the server player `player` (`name`) is about to change
    /// from `from` to `to` (Bukkit's `FoodLevelChangeEvent`):
    /// eating, hunger and saturation effects, the exhaustion that costs a
    /// point, starving. Change `to` (kept between 0 and 20; saturation never
    /// exceeds it) or set `cancel` to leave the level at `from`, saturation
    /// included. Heard once for the change a player's turn made as a whole, so
    /// two changes in one tick arrive as one. Not heard: the full food bar a
    /// respawn gives, the food a player joins with and the host of a LAN
    /// world. The player's state is swapped in, so `native_acting_player`,
    /// `native_tell` and the item helpers reach them. On a server.
    pub(in crate::game) fn native_food_level_changed(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_food_level_changed(player, name, from, to, cancel);
    }
    /// Before `player` picks up orb `orb` in `dimension`. Mutations to `value`
    /// and `count` take effect only when not canceled. Value keeps vanilla
    /// signed integer semantics; count is kept at least one.
    /// Canceling leaves the orb and pickup delay unchanged. Server-side.
    pub(in crate::game) fn native_pickup_xp(
        &mut self,
        player: u128,
        dimension: &str,
        orb: u32,
        value: &mut i32,
        count: &mut u32,
        canceled: &mut bool,
    ) {
        self.fire_pickup_xp(player, dimension, orb, value, count, canceled);
    }
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target isn't told yet. Server-side, for
    /// every player of the world (the helpers read the acting one).
    pub(in crate::game) fn native_hurt_enemy(
        &mut self,
        stack: &ItemStack,
        entity: u32,
        kind: &str,
    ) {
        self.fire_hurt_enemy(stack, entity, kind);
    }
    /// A server tick of `stack`, the stack a player carries in `slot`: where
    /// `Inventory.tick` calls `Item.inventoryTick`, for every stack, each tick,
    /// `/tick freeze` included (it stops the world, not the player). `slot` is
    /// as `Inventory` saves it (0 to 35; the armour 100 feet to 103 head; -106
    /// the off hand), and `selected` says it is the held slot.
    /// `ItemPlace::PlayerSlot { player, slot }` reads and changes the stack,
    /// which stays in its slot: an emptied one empties the slot, and the
    /// player's client gets the change. Only the `ItemPart`s attached to
    /// `stack`'s item hear it. Server-side, for every living player of the world
    /// (the helpers read the acting one).
    pub(in crate::game) fn native_inventory_tick(
        &mut self,
        stack: &ItemStack,
        slot: i32,
        selected: bool,
    ) {
        self.fire_inventory_tick(stack, slot, selected);
    }
    /// A player's equipment slot `slot` (`head`, `chest`, `legs`, `feet`,
    /// `offhand`, and `mainhand` on a swap of the hands or `/item`) took a different
    /// item, `new` (none when it was emptied): `LivingEntity.onEquipItem`,
    /// before the equip sound. It only hears. Never for a spectator.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one).
    pub(in crate::game) fn native_on_equip(&mut self, slot: &str, new: Option<&ItemStack>) {
        self.fire_on_equip(slot, new);
    }
    /// `MobEffect.shouldApplyEffectTickThisTick` for a content mod's effect
    /// `effect` (`ruby:bleeding`), each tick a player or a mob has it, in
    /// `MobEffectInstance.tickServer` before it counts down: `tick_count` is
    /// the ticks left (the entity's tick count when infinite). Set `apply`
    /// and `native_apply_effect_tick` follows. Server-side.
    pub(in crate::game) fn native_should_apply_effect_tick_this_tick(
        &self,
        effect: EffectKey,
        tick_count: i32,
        amplifier: i32,
        apply: &mut bool,
    ) {
        self.fire_should_apply_effect_tick_this_tick(effect, tick_count, amplifier, apply);
    }
    /// `MobEffect.applyEffectTick`: the content mod's effect `effect` acts on
    /// `entity` in `dimension` (`minecraft:overworld`). Clear `keep` and the
    /// effect ends now, before it counts down. `entity` is a mob's
    /// `Entity.getId`, None for a player (`native_acting_player` says who).
    /// It runs inside the entity's tick: a mob's world borrows the game
    /// there, so the game is read-only, and it does not hold the ticking
    /// mobs, nor the player's effects. `holder` identifies the holder; Host
    /// reads of a mob holder are unavailable while it is ticking. Queued
    /// changes apply after the hook; clearing `keep` ends the effect.
    /// Server-side.
    pub(in crate::game) fn native_apply_effect_tick(
        &self,
        call: mod_api::__private::ApplyEffectTickCall<'_>,
    ) {
        self.fire_apply_effect_tick(call);
    }
    /// `MobEffect.onEffectAdded`: `LivingEntity.addEffect` gave `entity`
    /// (as in `native_apply_effect_tick`) the content mod's effect `effect`,
    /// which it didn't have. Server-side.
    pub(in crate::game) fn native_on_effect_added(
        &mut self,
        effect: EffectKey,
        entity: Option<i32>,
        amplifier: i32,
    ) {
        self.fire_on_effect_added(effect, entity, amplifier);
    }
    /// NeoForge's `MobEffectEvent.Added`, vanilla's `MobEffect.onEffectStarted`:
    /// every `LivingEntity.addEffect` of the content mod's effect `effect`,
    /// new or not, changed or not, after `native_on_effect_added`.
    /// Server-side.
    pub(in crate::game) fn native_mob_effect_added(
        &mut self,
        effect: EffectKey,
        entity: Option<i32>,
        amplifier: i32,
    ) {
        self.fire_mob_effect_added(effect, entity, amplifier);
    }
    /// LivingEntity.knockback before resistance and motion, after the unchanged v1 hook.
    /// Cancellation from either hook wins. Attacker is the attacking entity's network id,
    /// falling back to the victim's last hurt-by mob, or 0 when unknown; `attacker_main` and
    /// `attacker_off` are what it holds (a player's selected and off-hand stacks, a mob's main hand), none when unknown. Player attackers
    /// include the guest whose action runs. Includes mob melee and armor-stand sweeps, not
    /// direct velocity pushes (explosions, Punch arrows, hoglin throws). Non-finite changes
    /// are restored before resistance. Server-side, without self: damage borrows the victim.
    /// Whether a mod hears projectiles' impacts (`native_projectile_impact`):
    /// set `hears`. Asked once a server tick before the projectiles fly; only
    /// then does the tick keep the copy of each flying projectile that a
    /// canceled impact flies on from, so a game without such a mod pays
    /// nothing. Server-side.
    pub(in crate::game) fn native_hears_projectile_impacts(&self, hears: &mut bool) {
        self.fire_hears_projectile_impacts(hears);
    }
    /// A projectile hit something (NeoForge's `ProjectileImpactEvent`, posted
    /// where vanilla's projectile has found what it meets, before the hit
    /// acts): `projectile` (its entity id) of type `kind` (`minecraft:arrow`),
    /// shot by `owner` (a mob's or a player's entity id, 0 for none) in the level
    /// `dimension`. It hit the entity `entity` of type `entity_kind` (a player is
    /// `minecraft:player`; a dragon's part is `minecraft:ender_dragon`), or, with
    /// `entity` None, the block at `block` on its `face` (the face's normal,
    /// `[0, 1, 0]` for the top), at the point `at`. Set `canceled` and the impact
    /// does not happen: no damage, no knockback, no sound, no block reaction, the
    /// arrow does not stick, a pearl does not teleport, a potion or a fireball
    /// does not burst. The projectile flies on and is not discarded, so a mod
    /// that cancels also decides what becomes of it. It continues through its
    /// whole motion and may hit something on a later tick. Covers what
    /// `tick_projectiles` flies: arrows, tridents, snowballs,
    /// eggs, pearls, potions, fireballs, wither skulls, wind charges, llama spit and
    /// shulker bullets, from players, mobs and dispensers. It does not fire for
    /// a breeze's deflection, for a hit on a wind charge, for fireworks or for
    /// fishing bobbers. A dragon part or End crystal is told by the id of the
    /// dragon or crystal. Heard only when a mod also sets
    /// `native_hears_projectile_impacts`. Server-side.
    pub(in crate::game) fn native_projectile_impact(
        &mut self,
        call: mod_api::__private::ProjectileImpactCall<'_>,
    ) {
        self.fire_projectile_impact(call);
    }

    /// `Block.stepOn` for a mod block, on its authoritative server level.
    /// No self: the simulation owns the entity during this callback.
    pub(in crate::game) fn native_block_step_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockStateId,
        position: [i32; 3],
    ) {
        Self::fire_block_step_on(dimension, entity, kind, state, position);
    }
}

impl HitLevel<'_> {
    /// NeoForge's `LivingIncomingDamageEvent`: `LivingEntity.hurtServer`, past its guards (an
    /// invulnerable or dead entity, fire on a fire-resistant one), is about to hurt entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) by `source`: its damage
    /// type (`minecraft:fall`), causing entity (`DamageSource.getEntity`: the shooter of an
    /// arrow), direct entity (`getDirectEntity`: the projectile's entity id when the
    /// damage type names one, on a player, mob or ender dragon; 0 for `magic()` and any
    /// other source with no direct entity, even during that projectile's impact), and
    /// where it comes from (`getSourcePosition`). `victim` is where the entity stands and
    /// looks, and `weapon` the direct entity's weapon (`getWeaponItem`) when it is known.
    /// `container` holds the damage before any reduction: change its `new_damage`, or the
    /// invulnerability ticks a fresh hit leaves (vanilla's 20), or set `canceled` and nothing
    /// of `hurtServer` happens (no damage, hurt sound, knockback, cooldown, statistics or
    /// criteria); a canceled hit on the ender dragon still counts as landed (`EnderDragon.hurt`
    /// returns true whatever `reallyHurt` did). It fires on every damage route, for players,
    /// mobs and the ender dragon. The level is the hit's: its blocks, random and own
    /// attachments; the hurt entity is in the middle of its hit.
    pub(in crate::game) fn native_living_incoming_damage(
        &mut self,
        call: mod_api::__private::LivingIncomingDamageCall<'_>,
    ) {
        self.fire_living_incoming_damage(call);
    }
    /// NeoForge's `LivingKnockBackEvent`: `LivingEntity.knockback` is about to push entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) `strength` away from the
    /// direction (`x`, `z`), on the raw strength, before the entity's knockback resistance takes
    /// its share and its motion changes. Vanilla's direction points at the source of a hit (the
    /// attacker's side), and the entity is pushed the other way. `source` is the push's
    /// `DamageSource`, named as `native_living_incoming_damage` names a hit's. `attacker` is the
    /// attacking entity's network id, falling back to the victim's last hurt-by mob, or 0 when
    /// unknown; `attacker_main` and `attacker_off` are what it holds (a player's selected and
    /// off-hand stacks, a mob's main hand), none when unknown. Change `strength`, `x` or `z`, or
    /// set `canceled`, and the push follows them: canceled, the entity's motion is left alone. A
    /// `strength` of 0 or less after the hook is no push, and a direction too short to tell
    /// draws a random one after it, as in vanilla. A value a mod leaves that is not a finite
    /// number is put back as it was. It fires for the players, mobs and armor stands the game
    /// knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder, a goat's ram and a mob's melee push. It does not fire for a
    /// sulfur cube with a block that a hit with a causing entity knocks back
    /// (`SulfurCube.knockback` does not call `super` there), for the pushes vanilla doesn't make
    /// with `LivingEntity.knockback` (the ender dragon's wings, Punch arrows, explosions, wind
    /// charges, a mace smash's area, a hoglin's throw), nor for the ones Horizon makes another
    /// way (the ender dragon's). The level is the hit's: its blocks, random and own attachments.
    pub(in crate::game) fn native_living_knockback(
        &mut self,
        call: mod_api::__private::LivingKnockbackCall<'_>,
    ) {
        self.fire_living_knockback(call);
    }
}

/// Server-level helpers for native mods.
#[allow(dead_code)]
impl GameLevel<'_> {
    /// Sets property `key` of the block at `pos` to `value` with
    /// `Level.setBlock(pos, state, flags)`, from a hook that gets the level's
    /// `ticks` (`native_random_tick`, `native_tick_block`). A crop's random
    /// tick grows it with `flags::UPDATE_CLIENTS`: neighbours' shapes update,
    /// but no neighbour gets `neighborChanged` and comparators aren't told;
    /// `flags::UPDATE_ALL` does both. `UPDATE_CLIENTS` sends a block update;
    /// `UPDATE_INVISIBLE` suppresses its notification only on clients, as in
    /// `Level.setBlock`. Any loaded position, not only
    /// the hook's; true when the block changed, false off an authoritative
    /// server, when the block has no such property or value, `pos` is not
    /// loaded, or the property already has `value` (no write, as
    /// `Level.setBlock` returns false for an unchanged state). Elsewhere,
    /// `Game::native_set_block_property_with` does the same.
    pub(in crate::game) fn native_set_block_property(
        &mut self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        if !self.native_authoritative {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.block_state(at);
        let Some(id) = property_change(self.registry, state, key, value) else {
            return false;
        };
        let b = self.behaviours;
        b.set_block(self, ticks, at, id, flags)
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The server-owned holders' attachments; `TYPE.get(host, holder)` reads
    /// one. On a client it is the copy of what the server synced: `set` and
    /// `remove` change only the copy.
    pub(in crate::game) fn attachments(&mut self) -> &mut dyn AttachmentStore {
        self
    }
    /// Read a loaded, light-emitting block. No implicit chunk loading.
    pub(in crate::game) fn lit_block_name(&self, pos: [i32; 3]) -> Option<mod_api::BlockKey> {
        self.world
            .chunks
            .contains_key(&(pos[0].div_euclid(16), pos[2].div_euclid(16)))
            .then(|| {
                self.res
                    .facts()
                    .get(self.world.block(pos[0], pos[1], pos[2]))
            })
            .filter(|f| f.emission > 0)
            .and_then(|f| mod_api::__private::block_key(&f.name))
    }
    /// A small local vanilla particle; caller should rate-limit emission.
    pub(in crate::game) fn flame_particle(&mut self, at: [f64; 3], soul: bool) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        if !self.native_window() || at.iter().any(|v| !v.is_finite()) {
            return;
        }
        let Some(p) = self.native_player() else {
            return;
        };
        if (at[0] - p.x).powi(2) + (at[1] - p.y).powi(2) + (at[2] - p.z).powi(2) > 1024.0 {
            return;
        }
        let kind = if soul {
            ParticleType::SoulFireFlame
        } else {
            ParticleType::Flame
        };
        self.particles
            .add(&ParticleOptions::simple(kind), at, [0.0, 0.01, 0.0]);
    }
    /// Whether the player left mod `mod_id` on in Options > Mods. A mod
    /// turned off does nothing (check this first in each hook) until the
    /// launcher's next build leaves it out.
    pub(in crate::game) fn on(&self, mod_id: &str) -> bool {
        !self.settings.mods.disabled.contains(mod_id)
    }
    /// The most of the item `item` one slot holds (`Item.getDefaultMaxStackSize`),
    /// or `None` for an unknown item: for a mod's container that fills or
    /// merges stacks itself.
    pub(in crate::game) fn max_stack_size(&self, item: mod_api::ItemKey) -> Option<u32> {
        self.res
            .items()
            .get(mod_api::__private::item_key_id(item)?)
            .map(|i| i.max_stack)
    }
    pub(in crate::game) fn max_damage(&self, item: mod_api::ItemKey) -> Option<u32> {
        self.res
            .items()
            .get(mod_api::__private::item_key_id(item)?)
            .map(|i| i.max_damage)
    }
    pub(in crate::game) fn item_key(&self, id: &mod_api::Identifier) -> Option<mod_api::ItemKey> {
        mod_api::__private::item_key_by_identifier(id)
    }
    /// Sets property `key` of the block at `pos` to `value`, as a furnace's
    /// tick sets its `lit` (`Level.setBlock` with `UPDATE_ALL`: neighbours
    /// and clients are told); true when the block changed, false when the
    /// block has no such property or value, or the property already has
    /// `value` (as `Level.setBlock` for an unchanged state). The block
    /// entity stays.
    pub(in crate::game) fn set_block_property(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
    ) -> bool {
        self.set_block_property_with(pos, key, value, flags::UPDATE_ALL)
    }
    /// `native_set_block_property` with `Level.setBlock`'s `flags`, as a
    /// brewing stand's tick sets its bottles with `flags::UPDATE_CLIENTS`.
    /// Players get the write when `UPDATE_CLIENTS` is set; false when this
    /// window isn't a server, and in each case `native_set_block_property`
    /// returns false.
    pub(in crate::game) fn set_block_property_with(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        // As `native_set_block`: a window that isn't a server holds a copy.
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return false;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(id) = property_change(self.pack.assets.registry, state, key, value) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, id, flags))
    }
    /// Sends clients the block at `pos` and its block entity's update tag
    /// (`native_block_entity_update_tag`) again, as `Level.sendBlockUpdated`
    /// does: with the level's other block changes, to the players who see its
    /// chunk, once however often it is called before then. Changes made in a
    /// block entity's tick go out at the start of the next tick, as vanilla's
    /// do (`ServerLevel.tick` broadcasts chunks before it ticks block
    /// entities). Call it when what clients show changed (a machine started),
    /// not for every tick's progress: a menu's data slots carry that. Nothing
    /// happens for an unloaded block. It also sends the one-shot fields of
    /// `ModBlockEntityView::set_client_update_fields`.
    pub(in crate::game) fn block_entity_sync(&mut self, pos: [i32; 3]) {
        if !self.world.loaded(pos[0], pos[2]) {
            return;
        }
        if let Some(log) = self.world.block_log.as_mut() {
            log.push((pos[0], pos[1], pos[2]));
        }
    }
    /// Plays `sound` for this player only (`SoundPlay`'s event: `minecraft:ui.button.click`,
    /// or a mod's own from its `assets/<id>/sounds.json`): nothing is sent to the server, and
    /// other players don't hear it. It takes `&self`, so a screen, key or mouse hook can play
    /// one: the sound is queued and starts at the next audio update (once a client tick),
    /// `after` ticks counted from then (0 to 1200, a minute; `SoundEngine.playDelayed`). `SoundPlay::ui` plays it at the listener as an interface sound
    /// (`SimpleSoundInstance.forUI`); `SoundPlay::at` at a position in the client's level,
    /// fading with distance (`volume` past 1 carries farther, 16 blocks times the volume), as
    /// thunder heard after the flash or a blast heard when its shock wave arrives. Its
    /// `SoundRef` follows and controls this play: `state`, `stop` and `fade`. None, with nothing
    /// played, on a server or for a value out of range; an event the resources don't have plays
    /// nothing, as vanilla's, and its handle reads `Stopped` once the update has tried it.
    pub(in crate::game) fn play_local_sound(
        &self,
        sound: &SoundPlay,
        after: u32,
    ) -> Option<SoundRef> {
        if !self.native_window() || !local_sound_ok(sound, after) {
            return None;
        }
        Some(self.mod_sounds.queue(sound, after))
    }
    /// The particles alive in the client's level (`ParticleEngine.countParticles`): 0 once
    /// a clear is asked, and on a server.
    pub(in crate::game) fn particle_count(&self) -> u32 {
        if !self.native_window() || self.mod_clear_particles.get() {
            return 0;
        }
        u32::try_from(self.client_particle_count()).unwrap_or(u32::MAX)
    }
    /// `ParticleEngine.clearParticles`: every particle and emitter goes before the next
    /// frame is drawn or the next particle tick runs, whichever comes first. It takes
    /// `&self`, so a screen or render hook can ask, as with `play_local_sound`.
    /// False on a server.
    pub(in crate::game) fn clear_particles(&self) -> bool {
        if !self.native_window() {
            return false;
        }
        self.mod_clear_particles.set(true);
        true
    }
    /// The GUI's size in GUI pixels, width then height.
    pub(in crate::game) fn native_size(&self, draw: &HudBatch) -> [f32; 2] {
        [draw.width, draw.height]
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    pub(in crate::game) fn native_fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        let [x, y, w, h] = rect;
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill(&self.gui.atlas, x, y, w, h, rgba(r, g, b, a));
    }
    /// A mod's own RGBA image, uploaded into the HUD atlas slot
    /// `MOD_IMAGE_SPRITE` and drawn with
    /// `native_sprite`: its own source rectangle crops and stretches it. `rgba`
    /// is `width * height` pixels of four bytes, each side at most
    /// `MOD_IMAGE_SIZE`; false when it is not that
    /// size. The upload costs a texture patch of the image, so send one when the
    /// image changed, not every frame.
    pub(in crate::game) fn mod_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        let Some(rect) = self
            .gui
            .atlas
            .sprites
            .get(crate::hud::MOD_IMAGE_SPRITE)
            .copied()
        else {
            return false;
        };
        if !mod_image_fits(width, height, rgba.len(), rect.w, rect.h) {
            return false;
        }
        let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
        self.map_patches.push(horizon_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width,
            height,
            rgba: rgba.to_vec(),
        });
        true
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (vanilla's `gui/...`, or a content mod's GUI texture
    /// `mymod:gui/...`), drawn stretched into `dst` (`[x, y, w, h]` in GUI
    /// pixels), every texel multiplied by `tint` (`0xAARRGGBB`, a fluid's colour)
    /// when there is one. False when there is no such sprite.
    pub(in crate::game) fn native_sprite(
        &self,
        batch: &mut HudBatch,
        name: &str,
        src: [f32; 4],
        dst: [f32; 4],
        tint: Option<u32>,
    ) -> bool {
        let Some(mut r) = self.gui.atlas.sprites.get(name).copied() else {
            return false;
        };
        let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
        r.u0 += src[0] * du;
        r.v0 += src[1] * dv;
        r.u1 = r.u0 + src[2] * du;
        r.v1 = r.v0 + src[3] * dv;
        let [a, red, green, blue] = tint.unwrap_or(u32::MAX).to_be_bytes();
        batch.rect_uv(dst[0], dst[1], dst[2], dst[3], r, rgba(red, green, blue, a));
        true
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    pub(in crate::game) fn native_item(
        &self,
        batch: &mut HudBatch,
        item: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.res.items().get(item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, &ItemStack::new(item, 1), at[0], at[1]);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// Draw a stack and its slot decorations, except the count, at `at`,
    /// `size` GUI pixels square. False when nothing was drawn: an unknown item
    /// or an empty stack (air, or a count of 0, as `ItemStack.isEmpty`).
    pub(in crate::game) fn native_item_stack(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        // GuiGraphicsExtractor.item and itemDecorations draw nothing for an
        // empty stack.
        if &*stack.item == "minecraft:air" || stack.count == 0 {
            return false;
        }
        if self.res.items().get(&stack.item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, stack, at[0], at[1]);
        self.draw_item_decorations(batch, stack, at[0], at[1], Some(String::new()));
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as `native_item`
    /// draws one; false for an unknown item.
    pub(in crate::game) fn native_item_model(
        &self,
        batch: &mut HudBatch,
        item: &str,
        model: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.res.items().get(item).is_none() {
            return false;
        }
        let mut stack = ItemStack::new(item, 1);
        let mut data = horizon_client::item_data::StackData::default();
        data.set_other(
            "minecraft:item_model",
            Some(horizon_core::nbt::Tag::String(model.into())),
        );
        stack.extra = Some(Box::new(data));
        let start = batch.vertices.len();
        self.draw_item_model(batch, &stack, at[0], at[1]);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, what `native_fill`'s rectangles can't draw.
    pub(in crate::game) fn native_quad(
        &self,
        batch: &mut HudBatch,
        points: [[f32; 2]; 4],
        argb: u32,
    ) {
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill_quad(&self.gui.atlas, points, rgba(r, g, b, a));
    }
    /// Opens a screen of mod `mod_id`'s own over the world: the screen hooks
    /// fire over it as over any other, so the mod draws it in
    /// `native_screen_render_post` and takes its clicks, scroll, keys and
    /// typing there; the mouse is freed, and Escape closes it. Every mod's own
    /// screen has the `ScreenView.id` `mod`, so a mod's screen hooks
    /// check `mod_screen()` is its own before drawing. It doesn't pause
    /// a single-player game (as the inventory doesn't). True when it is open:
    /// false while another screen is (or another mod's), over the title
    /// screen, a book, a sign editor, a dialog or a prompt, while the world
    /// loads, and anywhere but the window's own client (the server thread, a
    /// dedicated server). A gamepad can close it (B, Start) but not click on
    /// it: its clicks don't reach the mouse hooks.
    pub(in crate::game) fn mod_screen_open(&mut self, mod_id: &str) -> bool {
        // What `screen_hooks_open` would refuse over the world: the mod's
        // screen would be up with no hook to draw or click it.
        let blocked = self.renderer.is_none() || self.mod_screen_blocked();
        let owner = self.mod_screen();
        if let Some(open) = mod_screen_opens(self.screen, blocked, owner, mod_id) {
            return open;
        }
        self.native_mod_screen = Some(mod_id.to_owned());
        self.set_screen(Screen::Mod);
        if self.screen != Screen::Mod {
            // `screen_opening` kept the world.
            self.native_mod_screen = None;
            return false;
        }
        self.mod_screen() == Some(mod_id)
    }
    /// Closes mod `mod_id`'s own screen, if it is open.
    pub(in crate::game) fn mod_screen_close(&mut self, mod_id: &str) {
        if self.mod_screen().is_some_and(|id| id == mod_id) {
            self.close_mod_screen();
        }
    }
    /// The mod whose own screen is open, if one is (Escape, or another screen
    /// replacing it, closes it).
    pub(in crate::game) fn mod_screen(&self) -> Option<&str> {
        self.native_mod_screen
            .as_deref()
            .filter(|_| self.screen == Screen::Mod)
    }
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text.
    pub(in crate::game) fn screen_title(&self) -> Option<String> {
        self.remote_menu_title().map(|t| t.plain())
    }
    /// Where the mouse is, in GUI pixels.
    pub(in crate::game) fn cursor(&self) -> [f32; 2] {
        [self.cursor.0, self.cursor.1]
    }
    /// A line of text at `(x, y)` in GUI pixels, in `argb` as vanilla's `GuiGraphics` takes it
    /// (`0xFFFFFFFF` is opaque white). Fully transparent text draws nothing, as vanilla's.
    pub(in crate::game) fn native_text(
        &self,
        batch: &mut HudBatch,
        text: &str,
        at: [f32; 2],
        argb: u32,
        shadow: bool,
    ) {
        let [a, r, g, b] = argb.to_be_bytes();
        if a == 0 {
            return;
        }
        let [x, y] = at;
        batch.text_styled(
            &self.gui.atlas,
            &Text::literal(text),
            x,
            y,
            rgba(r, g, b, a),
            shadow,
        );
    }
    /// The clock of the world this window shows, for client mods: its game
    /// time and time of day as the server last sent them
    /// (`ClientboundSetTimePacket`), advanced each tick between packets; none
    /// on the title screen.
    pub(in crate::game) fn clock(&self) -> Option<mod_api::Clock> {
        self.player.is_some().then_some(mod_api::Clock {
            game_time: self.world.game_time,
            day_time: self.world.day_time,
        })
    }
    /// The saved key and modifiers for an owner-bound registration, or its declared default.
    pub(in crate::game) fn mod_key(
        &self,
        mod_id: &str,
        name: &str,
    ) -> Option<(String, ClientModifiers)> {
        let (_, registration) = mod_api::__private::key_mappings()
            .iter()
            .find(|(owner, mapping)| *owner == mod_id && mapping.id == name)?;
        let (key, modifiers) = self.settings.mods.binding(
            mod_id,
            name,
            (&registration.key, registration.modifiers.bits),
        );
        Some((key, ClientModifiers::new(modifiers)))
    }
    /// The instance (game) folder, which its players share: `saves/`,
    /// `resourcepacks/`, `shaderpacks/` and `horizon-mods/` are in it, and the
    /// title's screens create worlds and read packs there. A world's own
    /// `datapacks/` is in that world's folder (`saves/<world>/datapacks`).
    /// The folder the game runs in (`--player-dir` moves a player's options
    /// and servers only, not the instance). Nothing is created.
    ///
    /// Not on wasm: a path in the game's file system, which a wasm mod has none of; a wasm mod
    /// reads its own data and assets through reload listeners.
    pub(in crate::game) fn instance_dir(&self) -> PathBuf {
        Self::game_dir()
    }
    /// A mod's setting `key` (one file per key in `native_data_dir`). A mod's hook or part reads
    /// only its own mod's: another mod's id gives None.
    pub(in crate::game) fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        if Self::another_mods(mod_id) {
            return None;
        }
        self.stored_setting(mod_id, key)
    }
    pub(super) fn stored_setting(&self, mod_id: &str, key: &str) -> Option<String> {
        mod_settings::read_raw(&self.native_data_dir(mod_id), mod_id, key)
    }
    /// Keeps a mod's setting `key`. A mod's hook or part keeps only its own mod's.
    pub(in crate::game) fn set_setting(&self, mod_id: &str, key: &str, value: &str) {
        if Self::another_mods(mod_id) {
            return;
        }
        self.store_setting(mod_id, key, value);
    }
    pub(super) fn store_setting(&self, mod_id: &str, key: &str, value: &str) -> bool {
        mod_settings::store(&self.native_data_dir(mod_id), mod_id, key, value)
    }
    /// A mod's hook or part is running, and `mod_id` is another mod's.
    fn another_mods(mod_id: &str) -> bool {
        mod_api::content::crash::current_native_mod().is_some_and(|running| running != mod_id)
    }
    /// Shows the panel beside container screens with these items (at most
    /// 8192), as the game's own slots in pages, with a search box; the
    /// player's clicks, searches and keys over items come back as
    /// `native_panel_event`. The panel is the last mod's to show one; empty
    /// items and no recipe page take yours away.
    pub(in crate::game) fn panel_items(&mut self, mod_id: &str, items: Vec<mod_api::Stack>) {
        self.panel.show_items(mod_id, items);
    }
    /// The recipes of type `kind` (`minecraft:smelting`, `mymod:crusher`), vanilla's
    /// types and mods' own alike, from `data/<namespace>/recipe/*.json` of vanilla,
    /// the content mods (`.hmod`) and the world's data packs, in file order: each
    /// one typed (`Recipe`: its ingredients, results, group, cooking time and
    /// experience, or its stonecutting, smithing or special crafting rule),
    /// with its file as JSON for a mod's own type. Dynamic rules are not fixed
    /// outputs; use `LevelRef::craft` to assemble a crafting grid. These are the
    /// recipes in force (`RecipeManager.getAllRecipesFor`): data packs merged and
    /// load conditions held, so a pack's override or removal shows. A data pack
    /// changes them on `/reload`, so look them up again when you need them. Empty
    /// for a type nobody uses and outside a world. These are the server's table on
    /// the server thread, a dedicated server or a LAN host; a client, the
    /// single-player window included, sees vanilla's and the content mods' recipes,
    /// not the world's data packs', unless a mod had the server send it the type
    /// (`native_datapack_sync`).
    pub(in crate::game) fn recipes_of_type(
        &self,
        kind: &str,
    ) -> Arc<[crate::crafting::RecipeSnapshot]> {
        if self.outside_world() {
            return Default::default();
        }
        self.recipes()
            .by_type
            .get(kind)
            .cloned()
            .unwrap_or_default()
    }
    /// An ingredient as a recipe's JSON names it: an item, a `#tag`, or a list
    /// of them, a bare id as `minecraft:` (as vanilla reads it); `test` says
    /// whether a stack fits. Items the game doesn't know are left out; None
    /// for missing tags, invalid inputs or when no named item is known, and
    /// outside a world. Existing-empty tags retain an empty ingredient.
    pub(in crate::game) fn recipe_ingredient(
        &self,
        json: &str,
    ) -> Result<Option<Ingredient>, mod_api::RecipeJsonError> {
        if self.outside_world() {
            return Ok(None);
        }
        let json: serde_json::Value =
            serde_json::from_str(json).map_err(|_| mod_api::RecipeJsonError::InvalidJson)?;
        let qualify = |s: &str| match s.strip_prefix('#') {
            Some(tag) => String::from("#") + &qualified(tag),
            None => qualified(s),
        };
        let json = match &json {
            serde_json::Value::String(s) => serde_json::Value::String(qualify(s)),
            serde_json::Value::Array(a) => serde_json::Value::Array(
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(|s| serde_json::Value::String(qualify(s)))
                    .collect(),
            ),
            _ => return Ok(None),
        };
        let items = self.items();
        let Some(ingredient) = horizon_client::crafting::Ingredient::parse(&json, items) else {
            return Ok(None);
        };
        let known: Vec<String> = ingredient
            .order
            .into_iter()
            .filter(|item| items.get(item).is_some())
            .collect();
        Ok((ingredient.items.is_empty() || !known.is_empty()).then(|| Ingredient::of(known)))
    }
    /// A stack as a recipe's `result` names it: `id` (a bare one as
    /// `minecraft:`), `count` (1 by default, up to 99) and `components`. None
    /// without an `id`, for an item the game doesn't know or a count outside
    /// that, and outside a world.
    pub(in crate::game) fn recipe_result(
        &self,
        json: &str,
    ) -> Result<Option<ItemStack>, mod_api::RecipeJsonError> {
        if self.outside_world() {
            return Ok(None);
        }
        let mut json: serde_json::Value =
            serde_json::from_str(json).map_err(|_| mod_api::RecipeJsonError::InvalidJson)?;
        if let Some(id) = json.as_str() {
            json = qualified(id).into();
        } else if let Some(id) = json["id"].as_str() {
            json["id"] = qualified(id).into();
        }
        Ok(recipe_result(&json).filter(|s| self.items().get(&s.item).is_some()))
    }
    /// The `book_id`s of the recipes the player's recipe book holds now
    /// (the server's book on a server); empty outside a world.
    pub(in crate::game) fn known_recipes(&mut self) -> Arc<[i32]> {
        if self.player.is_none() {
            return Arc::from([]);
        }
        self.known_displays()
    }
    /// Places a recipe in the open screen's grid as a click in the recipe
    /// book does (`ServerboundPlaceRecipePacket`): the inventory's grid or a
    /// crafting table's. `all` places as many as the items allow, as with
    /// Shift. Refused while no such screen is open and for a recipe the
    /// player's book doesn't hold; one missing items shows as a ghost.
    pub(in crate::game) fn place_recipe(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        self.recipe_place(book_id, all)
    }
    /// Transfer a viewer recipe without learning it.
    pub(in crate::game) fn transfer_recipe(&mut self, book_id: i32) {
        self.transfer_viewer_recipe(book_id);
    }
    /// Creative mode only: gives the player `count` of `item`
    /// (`minecraft:diamond`) as taking it from the creative inventory does
    /// (`ServerboundSetCreativeModeSlot`): the selected slot first, then the
    /// hotbar, then the inventory. The count is cut to one stack.
    pub(in crate::game) fn give(
        &mut self,
        item: mod_api::ItemKey,
        count: u32,
    ) -> Result<(), String> {
        let item = mod_api::__private::item_key_id(item).ok_or("unknown item key")?;
        self.recipe_give(item, count)
    }
}

#[cfg(test)]
#[path = "base_tests/screen_open_tests.rs"]
mod screen_open_tests;

#[cfg(test)]
mod text_tests {
    use super::*;

    /// `DrawContext::text` takes ARGB as vanilla's `GuiGraphics` does: the alpha reaches the
    /// vertices, and fully transparent text draws nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn text_takes_argb_and_skips_transparent_text() {
        use mod_api::DrawContext;
        crate::game::native_entity_refs::tests::with_game(|game| {
            let drawn = |argb: u32| {
                let mut batch = HudBatch::scratch([640, 480]);
                crate::game::host::draw::GameDrawContext {
                    game,
                    surface: &mut batch,
                }
                .text("Hi", [4.0, 4.0], argb, false);
                batch.vertices
            };
            let opaque = drawn(0xFFFF_FFFF);
            let half = drawn(0x80FF_FFFF);
            assert!(!opaque.is_empty());
            assert_eq!(opaque.len(), half.len());
            assert!(opaque.iter().all(|v| v.color >> 24 == 0xFF));
            assert!(half.iter().all(|v| v.color >> 24 == 0x80));
            // RGB alone, as the API took before, is fully transparent now.
            assert!(drawn(0x00FF_FFFF).is_empty());
        });
    }
}

#[cfg(test)]
mod held_break_slot_tests {
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn held_durability_retains_the_stack_hand_and_explicit_break_slot() {
        use crate::game::{Audio, Game, Hand, Options, Progress, Settings, load_resources};
        use horizon_client::{items::ItemStack, motion::PlayerMotion};
        use horizon_core::equipment_slot::EquipmentSlot;
        use horizon_core::geometry::Vec3;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 70.0, 0.0)).unwrap());
        game.set_mode_id(0);
        // Stage an acting guest: break events are queued instead of rendering local crumbs.
        game.lan_acting = Some(1);
        game.hand = Hand::Off;
        let main = ItemStack::new("minecraft:stick", 3);
        game.inventory[game.selected] = Some(main.clone());
        let max = game
            .items()
            .get("minecraft:flint_and_steel")
            .unwrap()
            .max_damage;
        game.offhand = Some(ItemStack::new("minecraft:flint_and_steel", 1));
        game.damage_held_in_slot(1, EquipmentSlot::OffHand);
        assert_eq!(game.offhand.as_ref().unwrap().damage, 1);
        assert!(game.guest_equipment_breaks.is_empty());
        for (explicit, event) in [(true, 48), (false, 47)] {
            let mut tool = ItemStack::new("minecraft:flint_and_steel", 1);
            tool.damage = max - 1;
            game.offhand = Some(tool);
            if explicit {
                game.damage_held_in_slot(1, EquipmentSlot::OffHand);
            } else {
                game.damage_held(1);
            }
            assert!(game.offhand.is_none());
            assert_eq!(game.inventory[game.selected], Some(main.clone()));
            assert_eq!(*game.guest_equipment_breaks.last().unwrap(), event);
        }
        game.set_mode_id(1);
        game.offhand = Some(ItemStack::new("minecraft:flint_and_steel", 1));
        game.damage_held_in_slot(1, EquipmentSlot::OffHand);
        assert_eq!(game.offhand.as_ref().unwrap().damage, 0);
    }
}
