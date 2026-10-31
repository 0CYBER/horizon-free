//! The game-test runner: `GameTestInfo`, `GameTestRunner`, `GameTestTicker`,
//! `MultipleTestTracker`, `ReportGameListener`, `TestCommand.trackAndStartRunner`
//! and the two test instance types (`BlockBasedTestInstance` and
//! `FunctionGameTestInstance`), over the test instance blocks in a level.
//!
//! Not ported: test sequences (`GameTestSequence`), retries and flaky tests,
//! test environments other than the empty `all_of`, and removing the entities
//! of a finished test.
use super::{Dim, Game, structure_blocks::template_size, test_instances::TestDef};
use horizon_client::{
    containers::BlockEntity,
    lan::data::component_nbt,
    test_block::TestBlockMode,
    test_instance_block::{ErrorMarker, Status, TestInstanceBlock},
    text::{self, Text},
};
use horizon_core::{position::BlockPos, ticks::TickArea};
use horizon_world::{
    level::flags,
    worldgen::structure::{BoundingBox, Mirror, Rotation, StructureTemplate},
};
use std::time::Instant;

type Pos = (i32, i32, i32);

/// `GameTestAssertException`, `GameTestAssertPosException`,
/// `GameTestTimeoutException` and `UnknownGameTestException`.
#[derive(Clone, Debug)]
pub(super) enum TestError {
    Assert {
        message: Text,
        tick: i32,
    },
    AssertPos {
        message: Text,
        absolute: Pos,
        relative: Pos,
        tick: i32,
    },
    Timeout(Text),
    Unknown(String),
}

fn number(v: i32) -> Text {
    Text::literal(v.to_string())
}

impl TestError {
    /// `getDescription`.
    pub(super) fn description(&self) -> Text {
        match self {
            Self::Assert { message, tick } => {
                Text::translate("test.error.tick", &[message.clone(), number(*tick)])
            }
            Self::AssertPos {
                message,
                absolute,
                relative,
                tick,
            } => Text::translate(
                "test.error.position",
                &[
                    message.clone(),
                    number(absolute.0),
                    number(absolute.1),
                    number(absolute.2),
                    number(relative.0),
                    number(relative.1),
                    number(relative.2),
                    number(*tick),
                ],
            ),
            Self::Timeout(message) => message.clone(),
            Self::Unknown(reason) => {
                Text::translate("test.error.unknown", &[Text::literal(reason.clone())])
            }
        }
    }

    /// `getMessage`: the description's characters (a timeout's own message,
    /// an unknown error's reason).
    fn message(&self) -> String {
        match self {
            Self::Unknown(reason) => reason.clone(),
            other => other.description().plain(),
        }
    }
}

/// What `runAtTickTime` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// `BlockBasedTestInstance.run`'s `onEachTick` body.
    BlockBasedTick,
}

/// `GameTestInfo`.
pub(super) struct Info {
    id: u32,
    /// `id()`: the test's registry id.
    key: String,
    def: TestDef,
    /// The test instance block (`getTestBlockPos`), once there is one.
    pos: Pos,
    has_pos: bool,
    dim: Dim,
    extra_rotation: Rotation,
    tick_count: i32,
    started: bool,
    placed: bool,
    done: bool,
    error: Option<TestError>,
    run_at: Vec<(i64, Action)>,
    started_at: Option<Instant>,
    run_ms: u128,
}

impl Info {
    pub(super) fn new(
        id: u32,
        key: String,
        def: TestDef,
        pos: Option<Pos>,
        dim: Dim,
        extra_rotation: Rotation,
    ) -> Self {
        Self {
            id,
            key,
            def,
            pos: pos.unwrap_or_default(),
            has_pos: pos.is_some(),
            dim,
            extra_rotation,
            tick_count: 0,
            started: false,
            placed: false,
            done: false,
            error: None,
            run_at: Vec::new(),
            started_at: None,
            run_ms: 0,
        }
    }

    /// `getRotation`.
    fn rotation(&self) -> Rotation {
        self.def.rotation.get_rotated(self.extra_rotation)
    }

    /// `assertionException(description)`.
    fn assertion(&self, message: Text) -> TestError {
        TestError::Assert {
            message,
            tick: self.tick_count,
        }
    }

    /// `fail(Component)`.
    fn fail(&mut self, message: Text) {
        self.error = Some(self.assertion(message));
    }

    /// `finish`.
    fn finish(&mut self) {
        if !self.done {
            self.done = true;
            if let Some(at) = self.started_at.take() {
                self.run_ms = at.elapsed().as_millis();
            }
        }
    }

    /// `succeed`: finishes unless a failure is set.
    fn succeed(&mut self) {
        if self.error.is_none() {
            self.finish();
        }
    }

    /// `startExecution(tickDelay)`.
    fn start_execution(&mut self, tick_delay: i32) {
        self.tick_count = -(self.def.setup_ticks + tick_delay + 1);
    }

    /// `onEachTick`: the action on every tick from now to the timeout.
    fn on_each_tick(&mut self, action: Action) {
        for i in i64::from(self.tick_count)..i64::from(self.def.max_ticks) {
            self.run_at.push((i, action));
        }
    }
}

/// One `GameTestBatch`.
struct Batch {
    index: i32,
    environment: String,
    infos: Vec<Info>,
}

/// What `MultipleTestTracker` knows of a test.
struct Tracked {
    id: u32,
    required: bool,
    done: bool,
    failed: bool,
}

/// `GameTestRunner`.
struct Runner {
    batches: Vec<Batch>,
    current: usize,
    /// `currentBatchTracker`: the tests of the batch still to finish.
    batch_pending: usize,
    tracked: Vec<Tracked>,
    halt_on_error: bool,
    /// `newStructureSpawner`: where the tests without a block go.
    grid: Option<Grid>,
}

/// `StructureGridSpawner` (for one dimension) without `clearOnBatch`.
pub(super) struct Grid {
    first: Pos,
    next: Pos,
    per_row: i32,
    /// The z extent of `rowBounds` (an `AABB(BlockPos)` is one block deep).
    row_z: (i32, i32),
    count: i32,
}

impl Grid {
    /// `new DimensionGridState(start)`.
    pub(super) fn new(first: Pos, per_row: i32) -> Self {
        Self {
            first,
            next: first,
            per_row,
            row_z: (first.2, first.2 + 1),
            count: 0,
        }
    }
}

/// What a runner says to the server, sent on the next tick (a test's
/// messages come from the server's console, not a player's command).
enum Out {
    /// `sendSuccess(.., broadcast)`, `sendFailure` or `sendSystemMessage` of
    /// the server's command source.
    Console { text: Text, broadcast: bool },
    /// `ReportGameListener.say`: every player gets it.
    Players(Text),
}

/// `GameTestTicker.SINGLETON` and the runner it holds, `FailedTestTracker`.
#[derive(Default)]
pub(super) struct GameTests {
    ticker: Vec<Info>,
    runner: Option<Runner>,
    ticking: bool,
    halting: bool,
    next_id: u32,
    /// `FailedTestTracker.LAST_FAILED_TESTS`.
    pub(super) failed: Vec<String>,
    outbox: Vec<Out>,
}

impl GameTests {
    pub(super) fn next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }
}

impl Game<'_> {
    // ---- GameTestRunner / GameTestTicker ----

    /// `GameTestTicker.clear`: a pass in progress halts at its end.
    pub(super) fn clear_game_tests(&mut self) {
        if self.game_tests.ticking {
            self.game_tests.halting = true;
        } else {
            self.game_tests.ticker.clear();
            self.game_tests.runner = None;
        }
    }

    /// `TestCommand.trackAndStartRunner` for tests already built (batched by
    /// environment, 50 to a batch): the tracker and its listeners, then
    /// `GameTestRunner.start`.
    pub(super) fn track_and_start_runner(
        &mut self,
        infos: Vec<Info>,
        halt_on_error: bool,
        grid: Option<Grid>,
    ) {
        let tracked = infos
            .iter()
            .map(|i| Tracked {
                id: i.id,
                required: i.def.required,
                done: false,
                failed: false,
            })
            .collect();
        // `GameTestBatchFactory.fromGameTestInfo`: one batch per environment
        // (and dimension), in 50s.
        let mut batches: Vec<Batch> = Vec::new();
        let mut counts: Vec<(String, i32)> = Vec::new();
        for info in infos {
            let env = info.def.environment.clone();
            let open = batches
                .iter_mut()
                .find(|b| b.environment == env && b.infos.len() < 50);
            if let Some(batch) = open {
                batch.infos.push(info);
                continue;
            }
            let index = match counts.iter_mut().find(|(e, _)| *e == env) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    counts.push((env.clone(), 0));
                    0
                }
            };
            batches.push(Batch {
                index,
                environment: env,
                infos: vec![info],
            });
        }
        self.game_tests.runner = Some(Runner {
            batches,
            current: 0,
            batch_pending: 0,
            tracked,
            halt_on_error,
            grid,
        });
        self.run_batch(0);
    }

    /// `GameTestRunner.runBatch`.
    fn run_batch(&mut self, index: usize) {
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        let Some(batch) = runner.batches.get_mut(index) else {
            // endCurrentEnvironment, runScheduledRerunTests: nothing is left.
            return;
        };
        runner.current = index;
        let mut infos = std::mem::take(&mut batch.infos);
        let (environment, batch_number) = (batch.environment.clone(), batch.index);
        // createStructuresForBatch: `StructureSpawner.IN_PLACE`.
        let mut spawned = Vec::new();
        for mut info in infos.drain(..) {
            if self.spawn_test(&mut info) {
                spawned.push(info);
            }
        }
        log::info!(
            "Running test environment '{environment}' batch {batch_number} ({} tests)...",
            spawned.len()
        );
        // TestEnvironmentDefinition.activate: the environments' definitions
        // are not read, so none changes the level.
        self.game_tests.outbox.push(Out::Console {
            text: Text::translate(
                "commands.test.batch.starting",
                &[Text::literal(environment), number(batch_number)],
            ),
            broadcast: true,
        });
        if let Some(runner) = self.game_tests.runner.as_mut() {
            runner.batch_pending = spawned.len();
        }
        self.game_tests.ticker.extend(spawned);
    }

    /// `GameTestServer`'s per-tick call: `MinecraftServer.tickServer`'s
    /// `GameTestTicker.SINGLETON.tick()`, for the tests of the level in place.
    pub(super) fn tick_game_tests(&mut self) {
        self.flush_game_test_output();
        if self.game_tests.runner.is_none() {
            return;
        }
        let dim = self.dim;
        self.game_tests.ticking = true;
        let mut infos = std::mem::take(&mut self.game_tests.ticker);
        for info in infos.iter_mut().filter(|i| i.dim == dim) {
            self.tick_test(info);
        }
        infos.retain(|i| !i.done);
        infos.append(&mut self.game_tests.ticker);
        self.game_tests.ticker = infos;
        self.game_tests.ticking = false;
        if std::mem::take(&mut self.game_tests.halting) {
            self.clear_game_tests();
        }
    }

    // ---- GameTestInfo ----

    /// `GameTestRunner.spawn`: a test with a block is prepared where it is
    /// (`StructureSpawner.IN_PLACE`), one without goes to the next place in
    /// the grid (`StructureGridSpawner.spawnStructure`), or is dropped
    /// (`NOT_SET`).
    fn spawn_test(&mut self, info: &mut Info) -> bool {
        if info.has_pos {
            if !self.prepare_test_structure(info) {
                return false;
            }
            info.start_execution(1);
            return true;
        }
        let Some(mut grid) = self
            .game_tests
            .runner
            .as_mut()
            .and_then(|runner| runner.grid.take())
        else {
            return false;
        };
        info.pos = grid.next;
        info.has_pos = true;
        let prepared = self.prepare_test_structure(info);
        if prepared {
            info.start_execution(1);
            if let Some(b) = self.test_bounding_box(info.pos) {
                // `getTestBounds()`: the structure's box and its padding.
                let width = b.max[0] - b.min[0] + 1;
                grid.row_z = (grid.row_z.0.min(b.min[2]), grid.row_z.1.max(b.max[2] + 1));
                grid.next.0 += width + 5;
                grid.count += 1;
                if grid.count >= grid.per_row {
                    grid.count = 0;
                    grid.next.2 += grid.row_z.1 - grid.row_z.0 + 6;
                    grid.next.0 = grid.first.0;
                    grid.row_z = (grid.next.2, grid.next.2 + 1);
                }
            }
        }
        if let Some(runner) = self.game_tests.runner.as_mut() {
            runner.grid = Some(grid);
        }
        prepared
    }

    /// `GameTestInfo.prepareTestStructure`: the test instance block with the
    /// test's data, then `placeStructure`. False when the block is gone.
    fn prepare_test_structure(&mut self, info: &mut Info) -> bool {
        let pos = info.pos;
        self.set_test_block_state(pos, "minecraft:test_instance_block", flags::UPDATE_ALL);
        if self.test_instance_at(pos).is_none() {
            self.world.block_entities.insert(
                pos,
                BlockEntity::TestInstanceBlock(Box::<TestInstanceBlock>::default()),
            );
        }
        let size = self
            .template_entry(&info.def.structure)
            .map_or([1, 1, 1], |entry| template_size(&entry.tag));
        if let Some(block) = self.test_instance_mut(pos) {
            block.data.test = Some(info.key.clone());
            block.data.size = size;
            block.data.rotation = info.extra_rotation;
            block.data.ignore_entities = false;
            block.data.status = Status::Cleared;
            block.data.error_message = None;
        }
        self.mark_block_entity_updated(pos);
        // placeStructure
        if !self.place_test_structure(pos) {
            let name = self.test_name(pos).plain();
            info.fail(Text::translate(
                "test.error.structure.failure",
                &[Text::literal(name)],
            ));
        }
        info.placed = true;
        self.encase_test_structure(pos);
        if let Some(bounds) = self.test_bounding_box(pos) {
            // `getBlockTicks().clearArea`, `clearBlockEvents`.
            let area = TickArea::new(
                BlockPos::new(bounds.min[0], bounds.min[1], bounds.min[2]),
                BlockPos::new(bounds.max[0], bounds.max[1], bounds.max[2]),
            );
            self.block_ticks.blocks.clear_area(area);
            self.block_ticks.block_events.retain(|e| {
                !(bounds.min[0]..=bounds.max[0]).contains(&e.pos.x)
                    || !(bounds.min[1]..=bounds.max[1]).contains(&e.pos.y)
                    || !(bounds.min[2]..=bounds.max[2]).contains(&e.pos.z)
            });
        }
        true
    }

    /// `getTestBoundingBox`: the structure's box and its padding.
    pub(super) fn test_bounding_box(&self, pos: Pos) -> Option<BoundingBox> {
        let padding = self.test_holder_padding(pos);
        let inner = self.test_structure_box(pos)?;
        Some(BoundingBox::new(
            inner.min.map(|v| v - padding),
            inner.max.map(|v| v + padding),
        ))
    }

    /// `encaseStructure`: barriers around the box, but for the block itself.
    fn encase_test_structure(&mut self, pos: Pos) {
        let block = self
            .pack
            .assets
            .registry
            .default_state("minecraft:test_instance_block");
        for at in self.structure_boundary(pos) {
            if Some(self.world.block(at.0, at.1, at.2)) != block {
                self.set_test_block_state(at, "minecraft:barrier", flags::UPDATE_ALL);
            }
        }
    }

    /// `GameTestInfo.tick`.
    fn tick_test(&mut self, info: &mut Info) {
        if info.done {
            return;
        }
        if !info.placed {
            info.fail(Text::translate("test.error.ticking_without_structure", &[]));
        }
        let has_block = self.test_instance_at(info.pos).is_some();
        if !has_block {
            info.fail(Text::translate("test.error.missing_block_entity", &[]));
        }
        if info.error.is_some() {
            info.finish();
        } else if !self.test_chunks_loaded(info.pos) {
            return;
        }
        self.tick_internal(info);
        if info.done {
            self.test_finished(info);
        }
    }

    /// The chunks of the structure are loaded (`areEntitiesActuallyLoadedAndTicking`).
    fn test_chunks_loaded(&self, pos: Pos) -> bool {
        let Some(b) = self.test_structure_box(pos) else {
            return true;
        };
        (b.min[0] >> 4..=b.max[0] >> 4).all(|cx| {
            (b.min[2] >> 4..=b.max[2] >> 4).all(|cz| self.world.chunks.contains_key(&(cx, cz)))
        })
    }

    /// `GameTestInfo.tickInternal`.
    fn tick_internal(&mut self, info: &mut Info) {
        info.tick_count += 1;
        if info.tick_count < 0 {
            return;
        }
        if !info.started {
            self.start_test(info);
        }
        let now = i64::from(info.tick_count);
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut info.run_at)
            .into_iter()
            .partition(|(time, _)| *time <= now);
        info.run_at = later;
        for (_, action) in due {
            if let Err(error) = self.run_action(info, action) {
                info.error = Some(error);
            }
        }
        if info.tick_count > info.def.max_ticks {
            // No sequences: the test had no result in time.
            info.error = Some(TestError::Timeout(Text::translate(
                "test.error.timeout.no_result",
                &[number(info.def.max_ticks)],
            )));
        }
    }

    /// `GameTestInfo.startTest`.
    fn start_test(&mut self, info: &mut Info) {
        info.started = true;
        info.started_at = Some(Instant::now());
        self.set_test_status(info.pos, Status::Running, None);
        if let Err(error) = self.run_instance(info) {
            info.error = Some(error);
        }
    }

    /// `GameTestInstance.run(helper)`.
    fn run_instance(&mut self, info: &mut Info) -> Result<(), TestError> {
        if info.def.block_based {
            return self.run_block_based(info);
        }
        // FunctionGameTestInstance: the function from the test function
        // registry (`BuiltinTestFunctions`).
        let function = info.def.function.as_deref().unwrap_or_default();
        let function = if function.contains(':') {
            function.to_owned()
        } else {
            format!("minecraft:{function}")
        };
        match function.as_str() {
            // ALWAYS_PASS: GameTestHelper::succeed.
            "minecraft:always_pass" => {
                info.succeed();
                Ok(())
            }
            other => Err(TestError::Unknown(format!(
                "Trying to access missing test function: {other}"
            ))),
        }
    }

    fn run_action(&mut self, info: &mut Info, action: Action) -> Result<(), TestError> {
        match action {
            Action::BlockBasedTick => self.block_based_tick(info),
        }
    }

    // ---- GameTestHelper ----

    /// `absolutePos`.
    fn test_absolute_pos(&self, info: &Info, relative: Pos) -> Pos {
        let origin = self.test_origin(info);
        let moved = [
            origin[0] + relative.0,
            origin[1] + relative.1,
            origin[2] + relative.2,
        ];
        let [x, y, z] = StructureTemplate::transform(moved, Mirror::None, info.rotation(), origin);
        (x, y, z)
    }

    /// `getTestOrigin`: the block entity's start corner.
    fn test_origin(&self, info: &Info) -> [i32; 3] {
        let padding = info.def.padding;
        let rotation = self.test_rotation(info.pos);
        self.test_instance_at(info.pos).map_or([0; 3], |b| {
            b.start_corner([info.pos.0, info.pos.1, info.pos.2], padding, rotation)
        })
    }

    /// `assertionException(pos, description)`.
    fn pos_assertion(&self, info: &Info, relative: Pos, message: Text) -> TestError {
        TestError::AssertPos {
            message,
            absolute: self.test_absolute_pos(info, relative),
            relative,
            tick: info.tick_count,
        }
    }

    /// `getBlockEntity(pos, TestBlockEntity.class)`: the absolute position of
    /// the test block.
    fn test_block_entity(&self, info: &Info, relative: Pos) -> Result<Pos, TestError> {
        let absolute = self.test_absolute_pos(info, relative);
        match self.world.block_entities.get(&absolute) {
            None => Err(self.pos_assertion(
                info,
                relative,
                Text::translate("test.error.missing_block_entity", &[]),
            )),
            Some(BlockEntity::TestBlock(_)) => Ok(absolute),
            Some(other) => Err(self.pos_assertion(
                info,
                relative,
                Text::translate(
                    "test.error.wrong_block_entity",
                    &[Text::literal(other.id().to_owned())],
                ),
            )),
        }
    }

    /// `forEveryBlockInStructure`: the template-relative positions, x then
    /// y then z.
    fn test_relative_positions(&self, info: &Info) -> Vec<Pos> {
        let Some(b) = self.test_structure_box(info.pos) else {
            return Vec::new();
        };
        let size = [
            b.max[0] - b.min[0] + 1,
            b.max[1] - b.min[1] + 1,
            b.max[2] - b.min[2] + 1,
        ];
        let size = match info.rotation() {
            Rotation::Clockwise90 | Rotation::Counterclockwise90 => [size[2], size[1], size[0]],
            _ => size,
        };
        let mut out = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    out.push((x, y, z));
                }
            }
        }
        out
    }

    /// `BlockBasedTestInstance.findTestBlocks`.
    fn find_test_blocks(&self, info: &Info, mode: TestBlockMode) -> Vec<Pos> {
        let registry = self.pack.assets.registry;
        self.test_relative_positions(info)
            .into_iter()
            .filter(|&relative| {
                let at = self.test_absolute_pos(info, relative);
                registry
                    .state(self.world.block(at.0, at.1, at.2))
                    .is_some_and(|s| {
                        &*s.name == "minecraft:test_block"
                            && s.properties.get("mode").map(String::as_str)
                                == Some(mode.serialized_name())
                    })
            })
            .collect()
    }

    // ---- BlockBasedTestInstance ----

    /// `BlockBasedTestInstance.run`.
    fn run_block_based(&mut self, info: &mut Info) -> Result<(), TestError> {
        // findStartBlock
        let starts = self.find_test_blocks(info, TestBlockMode::Start);
        let missing = |game: &Self, info: &Info, key: &str, mode: TestBlockMode| {
            game.info_failure(
                info,
                Text::translate(key, &[Text::translate(&mode.label_key(), &[])]),
            )
        };
        if starts.is_empty() {
            return Err(missing(
                self,
                info,
                "test_block.error.missing",
                TestBlockMode::Start,
            ));
        }
        if starts.len() != 1 {
            return Err(missing(
                self,
                info,
                "test_block.error.too_many",
                TestBlockMode::Start,
            ));
        }
        let start = self.test_block_entity(info, starts[0])?;
        self.trigger_test_block(start);
        info.on_each_tick(Action::BlockBasedTick);
        Ok(())
    }

    /// `helper.fail(message)`: the exception it throws.
    fn info_failure(&self, info: &Info, message: Text) -> TestError {
        info.assertion(message)
    }

    /// The body of `BlockBasedTestInstance.run`'s `onEachTick`.
    fn block_based_tick(&mut self, info: &mut Info) -> Result<(), TestError> {
        let accepts = self.find_test_blocks(info, TestBlockMode::Accept);
        if accepts.is_empty() {
            return Err(self.info_failure(
                info,
                Text::translate(
                    "test_block.error.missing",
                    &[Text::translate(&TestBlockMode::Accept.label_key(), &[])],
                ),
            ));
        }
        let mut accepted = false;
        for relative in accepts {
            let at = self.test_block_entity(info, relative)?;
            accepted |= self.test_block_triggered(at);
        }
        if accepted {
            info.succeed();
            return Ok(());
        }
        // forAllTriggeredTestBlocks(FAIL, fail with the block's message)
        for relative in self.find_test_blocks(info, TestBlockMode::Fail) {
            let at = self.test_block_entity(info, relative)?;
            if self.test_block_triggered(at) {
                let message = self.test_block_message(at);
                return Err(self.info_failure(info, Text::literal(message)));
            }
        }
        // forAllTriggeredTestBlocks(LOG, trigger), each one reset after
        for relative in self.find_test_blocks(info, TestBlockMode::Log) {
            let at = self.test_block_entity(info, relative)?;
            if self.test_block_triggered(at) {
                self.trigger_test_block(at);
                self.reset_test_block(at);
            }
        }
        Ok(())
    }

    // ---- TestInstanceBlockEntity ----

    /// `setRunning`, `setSuccess` and `setErrorMessage`: the status and the
    /// error message it carries (`withStatus` clears it).
    fn set_test_status(&mut self, pos: Pos, status: Status, error: Option<&Text>) {
        let Some(block) = self.test_instance_mut(pos) else {
            return;
        };
        block.data.status = status;
        block.data.error_message = error.map(|t| component_nbt(&t.component()));
        self.mark_block_entity_updated(pos);
    }

    /// `markError`.
    fn mark_test_error(&mut self, pos: Pos, at: Pos, message: &Text) {
        let Some(block) = self.test_instance_mut(pos) else {
            return;
        };
        block.error_markers.push(ErrorMarker {
            pos: [at.0, at.1, at.2],
            text: component_nbt(&message.component()),
        });
        self.mark_block_entity_updated(pos);
    }

    // ---- listeners ----

    /// The listeners of a finished test, in the order they were added:
    /// `ReportGameListener`, the `TestSummaryDisplayer`, the failure tracker
    /// and the runner's batch listener.
    fn test_finished(&mut self, info: &Info) {
        let passed = info.error.is_none();
        match &info.error {
            None => {
                // reportPassed
                self.set_test_status(info.pos, Status::Finished, None);
                self.say(
                    text::GREEN,
                    format!(
                        "{} passed! ({}ms / {}gameticks)",
                        info.key, info.run_ms, info.tick_count
                    ),
                );
            }
            Some(error) => {
                // reportFailure
                self.set_test_status(info.pos, Status::Finished, Some(&error.description()));
                let required = info.def.required;
                let message = format!(
                    "{}{} failed! {}",
                    if required { "" } else { "(optional) " },
                    info.key,
                    error.message()
                );
                self.say(if required { text::RED } else { text::YELLOW }, message);
                if let TestError::AssertPos {
                    message, absolute, ..
                } = error
                {
                    self.mark_test_error(info.pos, *absolute, message);
                }
            }
        }
        // TestSummaryDisplayer
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        if let Some(t) = runner.tracked.iter_mut().find(|t| t.id == info.id) {
            t.done = true;
            t.failed = !passed;
        }
        let all_done = runner.tracked.iter().all(|t| t.done);
        let total = runner.tracked.len();
        let failed_required = runner
            .tracked
            .iter()
            .filter(|t| t.failed && t.required)
            .count();
        let failed_optional = runner
            .tracked
            .iter()
            .filter(|t| t.failed && !t.required)
            .count();
        let halt = runner.halt_on_error;
        if all_done {
            let out = &mut self.game_tests.outbox;
            out.push(Out::Console {
                text: Text::translate("commands.test.summary", &[number(total as i32)])
                    .with_color(text::WHITE),
                broadcast: true,
            });
            if failed_required > 0 {
                out.push(Out::Console {
                    text: Text::translate(
                        "commands.test.summary.failed",
                        &[number(failed_required as i32)],
                    )
                    .with_color(text::RED),
                    broadcast: false,
                });
            } else {
                out.push(Out::Console {
                    text: Text::translate("commands.test.summary.all_required_passed", &[])
                        .with_color(text::GREEN),
                    broadcast: true,
                });
            }
            if failed_optional > 0 {
                out.push(Out::Console {
                    text: Text::translate(
                        "commands.test.summary.optional_failed",
                        &[number(failed_optional as i32)],
                    ),
                    broadcast: false,
                });
            }
        }
        // FailedTestTracker.rememberFailedTest
        if !passed {
            self.game_tests.failed.push(info.key.clone());
        }
        // The runner's batch listener.
        if passed {
            self.remove_test_barriers(info.pos);
        } else if halt {
            self.clear_game_tests();
            return;
        }
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        runner.batch_pending = runner.batch_pending.saturating_sub(1);
        if runner.batch_pending == 0 {
            let next = runner.current + 1;
            self.run_batch(next);
        }
    }

    /// `ReportGameListener.say`.
    fn say(&mut self, color: u32, text: String) {
        self.game_tests
            .outbox
            .push(Out::Players(Text::literal(text).with_color(color)));
    }

    /// Sends what the runner said: the server's console log (and the
    /// operators, when it broadcasts) and the players' chat.
    fn flush_game_test_output(&mut self) {
        for out in std::mem::take(&mut self.game_tests.outbox) {
            match out {
                Out::Console { text, broadcast } => {
                    let plain = text.plain();
                    if self.options.dedicated.is_some() {
                        super::dedicated::log("INFO", &plain);
                    } else {
                        log::info!("{plain}");
                    }
                    if broadcast {
                        // CommandSourceStack.broadcastToAdmins as the server.
                        self.console_broadcast = Some(true);
                        self.broadcast_to_admins(text);
                        self.console_broadcast = None;
                        if let Some(mut host) = self.lan.take() {
                            self.send_admin_feedback(&mut host, None);
                            self.lan = Some(host);
                        }
                        self.admin_feedback.clear();
                    }
                }
                Out::Players(text) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.system_message(component_nbt(&text.component()));
                    }
                    if self.serving.is_none() && self.options.dedicated.is_none() {
                        self.system_message(text);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(block_based: bool, max_ticks: i32, setup_ticks: i32) -> Info {
        Info::new(
            1,
            "minecraft:t".into(),
            TestDef::sample(block_based, max_ticks, setup_ticks),
            Some((0, 64, 0)),
            Dim::Overworld,
            Rotation::None,
        )
    }

    #[test]
    fn start_execution_waits_for_the_setup_ticks_and_the_delay() {
        // always_pass: setup_ticks 1, delay 1: ticks -2, -1 and 0 follow.
        let mut i = info(false, 1, 1);
        i.start_execution(1);
        assert_eq!(i.tick_count, -3);
    }

    #[test]
    fn on_each_tick_runs_every_tick_up_to_the_timeout() {
        let mut i = info(true, 5, 0);
        i.tick_count = 2;
        i.on_each_tick(Action::BlockBasedTick);
        let times: Vec<i64> = i.run_at.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, vec![2, 3, 4]);
    }

    #[test]
    fn a_failure_is_not_overwritten_by_success_and_stamps_the_tick() {
        let mut i = info(true, 5, 0);
        i.tick_count = 3;
        i.fail(Text::literal("boom"));
        i.succeed();
        assert!(!i.done);
        assert!(matches!(i.error, Some(TestError::Assert { tick: 3, .. })));
        let mut ok = info(true, 5, 0);
        ok.succeed();
        assert!(ok.done && ok.error.is_none());
    }

    #[test]
    fn unknown_errors_report_their_reason() {
        let error = TestError::Unknown("no such function".into());
        assert_eq!(error.message(), "no such function");
    }
}
