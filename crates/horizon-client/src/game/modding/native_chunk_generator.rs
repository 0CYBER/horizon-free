//! Native terrain adapter for the API's worker context.
use super::native_worldgen::WorkerResources;
use horizon_world::worldgen::mod_generators::{
    ChunkAccess as EngineChunk, ModChunkGenerator, ModChunkGenerators, VanillaStages,
    install_mod_chunk_generators,
};
use mod_api::{BlockKey, BlockMetadata, BlockState, ChunkAccess, ChunkGenerator, ChunkPos};
use std::sync::Arc;

/// A mod's generator with the vanilla stages it asked for at registration.
struct Adapter {
    stages: VanillaStages,
    generator: Arc<dyn ChunkGenerator>,
}
impl ModChunkGenerator for Adapter {
    fn build_terrain(&self, chunk: &mut dyn EngineChunk) {
        let Ok(worker) = WorkerResources::get(chunk.registry()) else {
            log::error!("Horizon: mod terrain has no resolved worker metadata");
            return;
        };
        self.generator.build_terrain(&mut Converting {
            inner: chunk,
            worker,
        });
    }
    fn vanilla_stages(&self) -> VanillaStages {
        self.stages
    }
}

struct Converting<'a> {
    inner: &'a mut dyn EngineChunk,
    worker: &'a WorkerResources,
}
impl ChunkAccess for Converting<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn pos(&self) -> ChunkPos {
        self.inner.pos()
    }
    fn level(&self) -> &str {
        self.inner.level()
    }
    fn seed(&self) -> i64 {
        self.inner.seed()
    }
    fn min_y(&self) -> i32 {
        self.inner.min_y()
    }
    fn height(&self) -> i32 {
        self.inner.height()
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker
            .state_with(self.inner.registry(), block, properties)
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool {
        self.worker
            .index
            .to_engine(state)
            .is_ok_and(|state| self.inner.set_block(pos, state))
    }
}

pub(in crate::game) fn installed_chunk_generators() -> ModChunkGenerators {
    mod_api::__private::chunk_generator_declarations()
        .into_iter()
        .map(|(id, stages, generator)| {
            let mod_api::VanillaStages {
                surface,
                carvers,
                structures,
                features,
            } = stages;
            let stages = VanillaStages {
                surface,
                carvers,
                structures,
                features,
            };
            (
                id,
                Arc::new(Adapter { stages, generator }) as Arc<dyn ModChunkGenerator>,
            )
        })
        .collect()
}

/// Hand the chunk generators the mods registered to the engine, before any world opens.
pub(in crate::game) fn install() {
    if install_mod_chunk_generators(installed_chunk_generators()).is_err() {
        log::warn!("Mod chunk generators already installed; keeping the first table");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::install_parts;
    use mod_api::{DimensionSettings, Hooks, ModEntry, Registrar, RegistrationContext};

    /// Terrain alone, with the surface and the features after it.
    struct Decorated;
    impl ChunkGenerator for Decorated {
        fn build_terrain(&self, _: &mut dyn ChunkAccess) {}
        fn vanilla_stages(&self) -> mod_api::VanillaStages {
            mod_api::VanillaStages {
                surface: true,
                features: true,
                ..Default::default()
            }
        }
    }

    /// The engine sees the stages the mod's generator asked for, read once, at registration.
    #[test]
    fn a_generators_vanilla_stages_reach_the_engine_read_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        struct Counted;
        impl ChunkGenerator for Counted {
            fn build_terrain(&self, _: &mut dyn ChunkAccess) {}
            fn vanilla_stages(&self) -> mod_api::VanillaStages {
                ASKED.fetch_add(1, Ordering::Relaxed);
                Decorated.vanilla_stages()
            }
        }
        struct Registers;
        #[mod_api::__subscriptions]
        impl Hooks for Registers {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_chunk_generator("chaos", Counted);
            }
        }
        install_parts(&[ModEntry::new("test", Box::new(Registers))]).unwrap();
        let generators = installed_chunk_generators();
        for _ in 0..2 {
            assert_eq!(
                generators["test:chaos"].vanilla_stages(),
                VanillaStages {
                    surface: true,
                    carvers: false,
                    structures: false,
                    features: true,
                }
            );
        }
        assert_eq!(ASKED.load(Ordering::Relaxed), 1);
    }

    /// A wasm mod's registrar refuses a chunk generator with vanilla stages by its name, as it
    /// refuses any chunk generator: neither crosses to wasm.
    #[test]
    #[should_panic(
        expected = "chunk generator \"chaos\": a wasm mod cannot register a chunk generator"
    )]
    fn a_wasm_mods_staged_generator_is_refused_by_name() {
        struct Guest;
        impl mod_api::DimensionsRegistrar for Guest {
            fn add_dimension(&mut self, _: &str, _: DimensionSettings) {}
        }
        mod_api::DimensionsRegistrar::add_chunk_generator(&mut Guest, "chaos", Arc::new(Decorated));
    }
}
