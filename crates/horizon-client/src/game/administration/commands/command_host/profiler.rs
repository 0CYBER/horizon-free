use crate::game::Game;
use horizon_client::command::Context;
use horizon_gameplay::commands::{host::R, profiler::ProfilerHost};
use std::time::Instant;

impl ProfilerHost for Game<'_> {
    fn profiler_running(&self) -> bool {
        self.cmds.profiler_delay || self.cmds.profiler.is_some()
    }
    fn start_profiler_delayed(&mut self) {
        self.cmds.profiler_delay = true;
    }
    fn take_profiler_run(&mut self) -> Option<(f64, f64)> {
        let (started, ticks) = self.cmds.profiler.take()?;
        Some((
            started.elapsed().as_nanos() as f64 / 1.0e9,
            (self.ticks - ticks) as f64,
        ))
    }
    fn debug_function(&mut self, ctx: &Context) -> R<i32> {
        self.cmd_debug_function(ctx)
    }
    fn jfr_running(&self) -> bool {
        self.cmds.jfr.is_some()
    }
    fn start_jfr(&mut self, stamp: String) {
        self.cmds.jfr = Some((Instant::now(), self.ticks, stamp));
    }
    fn take_jfr(&mut self) -> Option<(f64, u64, String)> {
        let (started, ticks, stamp) = self.cmds.jfr.take()?;
        Some((started.elapsed().as_secs_f64(), self.ticks - ticks, stamp))
    }
}
