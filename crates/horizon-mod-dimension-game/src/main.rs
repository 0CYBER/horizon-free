//! The game with a native mod that adds levels, for `tools/mod_dimensions_e2e.py`. It is what the
//! launcher's generated `main` builds, with one mod: `dimprobe` registers the level
//! `dimprobe:pocket` on the game's noise, the chunk generator `dimprobe:slab`, and the level
//! `dimprobe:slabs` it fills. The server command `dimprobe enter <level>` sends its player there
//! with `run_command`, inside the command, and `dimprobe later <level>` from the next server tick,
//! where no player's state is swapped in. The run needs a `horizon-native-mods/dimprobe` package
//! beside the executable, which the harness writes.
//!
//! A crate and not an example of horizon-client: an example builds with the dev-dependencies, whose
//! `testing` feature keeps the mods per thread, and the server thread would see none.
use mod_api::__private::native::BuiltMod;
use mod_api::Side;
use mod_api::{
    BlockKey, ChunkAccess, ChunkGenerator, CommandArgumentValue, CommandSource, DimensionSettings,
    Flow, Hooks, ModEntry, RegisterCommandsEvent, Registrar, RegistrationContext,
    RunModCommandEvent, ServerHost, ServerTickEvent, ServerTickPhase, Text,
};
use std::sync::Mutex;

const BUILT: &[BuiltMod] = &[BuiltMod {
    id: "dimprobe",
    version: "1.0.0",
    side: Side::Both,
}];

/// A stone floor up to y=0 under a gold block at each chunk's corner, its y the world's seed
/// modulo 16 above the floor: terrain only this generator makes.
struct Slab {
    stone: BlockKey,
    gold: BlockKey,
}

impl ChunkGenerator for Slab {
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
        let stone = chunk.state(self.stone);
        let gold = chunk.state(self.gold);
        let [x, z] = [chunk.pos().x * 16, chunk.pos().z * 16];
        for (dx, dz) in (0..16).flat_map(|dx| (0..16).map(move |dz| (dx, dz))) {
            for y in chunk.min_y()..=0 {
                chunk.set_block([x + dx, y, z + dz], stone);
            }
        }
        let marker = 1 + chunk.seed().rem_euclid(16) as i32;
        chunk.set_block([x, marker, z], gold);
    }
}

#[derive(Default)]
struct DimProbe {
    /// The line the next server tick runs.
    later: Mutex<Option<String>>,
}

/// The command that sends `name` to `level`, 40 blocks over the world's y=0.
fn travel(level: &str, name: &str) -> String {
    format!("execute in dimprobe:{level} run tp {name} 8 40 8")
}

#[mod_api::__subscriptions]
impl Hooks for DimProbe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        let gold = r.block_key("minecraft:gold_block");
        r.register_chunk_generator("slab", Slab { stone, gold });
        r.register_dimension(
            "pocket",
            DimensionSettings {
                min_y: -128,
                height: 448,
                logical_height: 448,
                ..Default::default()
            },
        );
        r.register_dimension(
            "slabs",
            DimensionSettings {
                min_y: -32,
                height: 128,
                logical_height: 128,
                generator: "dimprobe:slab".into(),
                biome: "minecraft:desert".into(),
                ..Default::default()
            },
        );
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.extend([
            mod_api::Command::new("dimprobe enter <level:word>"),
            mod_api::Command::new("dimprobe later <level:word>"),
        ]);
    }

    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        let (CommandSource::Player { player, .. }, Some(CommandArgumentValue::String(level))) =
            (event.source, event.command.argument("level"))
        else {
            return Flow::Continue;
        };
        let Some(name) = player.name(&*host) else {
            return Flow::Continue;
        };
        let line = travel(level, &name);
        if event.command.literals.iter().any(|word| word == "later") {
            *self.later.lock().unwrap() = Some(line);
            host.commands()
                .feedback(Text::literal("dimprobe queued"))
                .expect("bounded probe text");
        } else {
            let ran = host.commands().run_command(&line);
            host.commands()
                .feedback(Text::literal(format!("dimprobe ran={ran}")))
                .expect("bounded probe text");
        }
        Flow::Cancel
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickStart { .. })
            && let Some(line) = self.later.lock().unwrap().take()
        {
            let ran = host.commands().run_command(&line);
            log::info!("dimprobe later ran={ran}");
        }
    }
}

fn main() {
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    horizon_mod_host::main_with(
        vec![ModEntry::new("dimprobe", Box::<DimProbe>::default())],
        &[],
        BUILT,
        startup,
    );
}
