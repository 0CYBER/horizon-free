//! The hot-path benchmark (docs/hot-path-trends.md): with `HORIZON_HOT_PATHS=OUTPUT` the game
//! loads its resources, runs fixed, seeded workloads through the code a playing game runs and
//! writes one JSON line per workload: its name, how many operations it ran, a checksum of what
//! they did and the wall time. Each measured region is bracketed by Callgrind requests
//! (`horizon_threads::callgrind`), so the same run under Valgrind gives the instructions of each
//! region; setup is never counted. `HORIZON_HOT_PATHS_ONLY=a,b` keeps the workloads whose names
//! start with one of the prefixes. With `HORIZON_HOT_PATHS_MODS=native` the executable
//! (`horizon-hot-paths-game`) has installed one native mod that listens to the entity tick hooks:
//! that run measures only the hook workload, under its listener's name.
//!
//! A workload builds its own world on a server `Game`, as the component harnesses do. Nothing is
//! compared with vanilla here: the checksum only shows that two runs did the same work.
use super::Game;
use horizon_client::Result;
use std::{
    ffi::CString,
    hash::{DefaultHasher, Hash, Hasher},
    io::Write as _,
    time::Instant,
};

mod chunks;
mod commands;
mod mods;
mod ticks;
mod world;

/// Where the measured workloads report.
pub(super) struct Recorder {
    out: std::io::BufWriter<std::fs::File>,
    only: Vec<String>,
}

impl Recorder {
    /// Whether the workload `name` was asked for; a workload skips its setup otherwise.
    fn wants(&self, name: &str) -> bool {
        self.only.is_empty()
            || self
                .only
                .iter()
                .any(|prefix| name.starts_with(prefix.as_str()))
    }

    /// Run `work` as the measured region of `name`: `operations` is what its cost divides by,
    /// and what it returns is hashed into the line's checksum.
    fn measure<T: Hash>(
        &mut self,
        name: &str,
        operations: u64,
        work: impl FnOnce() -> T,
    ) -> Result<()> {
        let label = CString::new(name)?;
        let started = Instant::now();
        horizon_threads::callgrind::start();
        let done = work();
        horizon_threads::callgrind::stop(&label);
        let wall = started.elapsed();
        let mut hash = DefaultHasher::new();
        done.hash(&mut hash);
        writeln!(
            self.out,
            "{}",
            serde_json::json!({
                "name": name,
                "operations": operations,
                "checksum": format!("{:016x}", hash.finish()),
                "wall_ns": wall.as_nanos() as u64,
            })
        )?;
        self.out.flush()?;
        Ok(())
    }
}

impl Game<'_> {
    /// Run the workloads and write their lines to `output`.
    pub(super) fn hot_paths(&mut self, output: &str) -> Result<()> {
        assert!(
            !self.options.execution_role.is_client(),
            "the benchmark requires a server Game"
        );
        let mut recorder = Recorder {
            out: std::io::BufWriter::new(std::fs::File::create(output)?),
            only: std::env::var("HORIZON_HOT_PATHS_ONLY")
                .map(|only| only.split(',').map(str::to_owned).collect())
                .unwrap_or_default(),
        };
        // No world opens here: the command tree's registries and the console's rights come
        // from the resources, as a world's opening sets them.
        self.build_registries();
        self.ui.registries.permission = 4;
        // The levels' handles, which a world's opening publishes to mods.
        self.reset_entity_residency();
        self.native_lifecycle
            .begin_open(mod_api::Authority::DedicatedServer);
        let listening =
            std::env::var_os("HORIZON_HOT_PATHS_MODS").is_some_and(|mods| mods == "native");
        if !listening {
            self.hot_ticks(&mut recorder)?;
            self.hot_commands(&mut recorder)?;
            self.hot_chunks(&mut recorder)?;
        }
        self.hot_mods(&mut recorder, listening)
    }
}
