//! A command that resolves an entity selector, parsed and run as a function's line or a
//! repeating command block runs it every tick.
use super::{super::Game, Recorder};
use horizon_client::Result;

impl Game<'_> {
    pub(super) fn hot_commands(&mut self, recorder: &mut Recorder) -> Result<()> {
        for (name, command) in [
            (
                "command.selector_type_128",
                "execute as @e[type=minecraft:cow] run data get entity @s Health",
            ),
            (
                "command.selector_nearest_128",
                "execute at @e[type=minecraft:pig,sort=nearest,limit=4,distance=..48] if block ~ ~-1 ~ minecraft:stone",
            ),
        ] {
            if !recorder.wants(name) {
                continue;
            }
            self.hot_level(4, 2)?;
            self.hot_mobs(128, 2)?;
            const RUNS: u64 = 200;
            for _ in 0..10 {
                self.hot_command(command)?;
            }
            recorder.measure(name, RUNS, || {
                (0..RUNS).filter(|_| self.run_command(command)).count()
            })?;
        }
        Ok(())
    }
}
