//! The engine side of `alter_ground`: the tree decorator's callback, and a mod's ground
//! provider as the decorator asks it. `mod_api` keeps only the API's provider traits.
use horizon_world::native_worldgen_hooks::{AlterGroundTree, GroundAccess, ModGroundProvider};
use horizon_world::registry::BlockStateId as EngineState;
use mod_api::{
    BlockStateId as ApiState, GroundLevel, GroundProvider, GroundStateProvider, HookId,
    StructureRandom,
};
use std::sync::Arc;

fn api(state: EngineState) -> ApiState {
    ApiState(state.0)
}

/// The engine's `alter_ground` callback: `replaced` is set only when a mod put
/// its provider in place of the decorator's own. Nothing is built or read while
/// no mod listens.
pub(crate) fn alter_ground(
    tree: &AlterGroundTree<'_>,
    replaced: &mut Option<Arc<dyn ModGroundProvider>>,
) {
    if !mod_api::__private::listened(HookId::AlterGround) {
        return;
    }
    let mut provider = GroundProvider::Decorator;
    let asked = super::native_worldgen_parts::with_host(|host| {
        mod_api::world::ground::native_alter_ground(
            host,
            tree.logs,
            tree.leaves,
            tree.roots,
            tree.positions,
            &mut provider,
        );
    });
    if asked.is_none() {
        return;
    }
    if let GroundProvider::Mod(provider) = provider {
        *replaced = Some(Arc::new(Engine(provider)));
    }
}

/// A mod's provider, as the decorator asks it.
struct Engine(Arc<dyn GroundStateProvider>);
impl ModGroundProvider for Engine {
    fn state_at(&self, access: &mut dyn GroundAccess, pos: [i32; 3]) -> Option<EngineState> {
        self.0
            .state_at(&mut Level(access), pos)
            .map(|state| EngineState(state.0))
    }
}

/// The decorator's run, as a provider reads it.
struct Level<'a>(&'a mut dyn GroundAccess);
impl GroundLevel for Level<'_> {
    fn block_state(&mut self, pos: [i32; 3]) -> ApiState {
        api(self.0.block_state(pos))
    }
    fn decorator_state(&mut self, pos: [i32; 3]) -> Option<ApiState> {
        self.0.decorator_state(pos).map(api)
    }
    fn random(&mut self) -> &mut dyn StructureRandom {
        self
    }
}
impl StructureRandom for Level<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.0.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.next_boolean()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    use horizon_world::native_worldgen_hooks::WorldgenRandom;
    use mod_api::{
        AlterGroundEvent, GroundLevel, GroundProvider, GroundStateProvider, Hooks, ModEntry,
        WorldgenHost,
    };
    use mod_platform::testing;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    const GRASS: EngineState = EngineState(1);
    const PODZOL: EngineState = EngineState(2);
    const MOSS: EngineState = EngineState(3);
    const MUD: EngineState = EngineState(4);

    /// The server's context, published as a server's resource load does.
    fn publish() {
        let registries =
            BTreeMap::from([("block".to_owned(), vec!["minecraft:moss_block".to_owned()])]);
        let root = std::env::temp_dir().join("horizon-alter-ground-no-settings");
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(registries, root)),
            [],
        );
        horizon_world::native_worldgen_hooks::publish_context(Arc::new(load.worldgen_context([])));
    }

    /// A tree on grass whose decorator turns grass to podzol, counting its draws.
    #[derive(Default)]
    struct Meadow {
        draws: u32,
    }
    impl GroundAccess for Meadow {
        fn block_state(&mut self, _: [i32; 3]) -> EngineState {
            GRASS
        }
        fn decorator_state(&mut self, pos: [i32; 3]) -> Option<EngineState> {
            self.draws += 1;
            (self.block_state(pos) == GRASS).then_some(PODZOL)
        }
    }
    impl WorldgenRandom for Meadow {
        fn next_int_bound(&mut self, _: i32) -> i32 {
            self.draws += 1;
            0
        }
        fn next_long(&mut self) -> i64 {
            self.draws += 1;
            0
        }
        fn next_float(&mut self) -> f32 {
            self.draws += 1;
            0.0
        }
        fn next_double(&mut self) -> f64 {
            self.draws += 1;
            0.0
        }
        fn next_boolean(&mut self) -> bool {
            self.draws += 1;
            false
        }
    }

    const LOGS: [[i32; 3]; 2] = [[0, 64, 0], [0, 65, 0]];
    fn tree() -> AlterGroundTree<'static> {
        AlterGroundTree {
            logs: &LOGS,
            leaves: &[],
            roots: &[],
            positions: &LOGS[..1],
        }
    }
    fn alter() -> Option<Arc<dyn ModGroundProvider>> {
        publish();
        let mut replaced = None;
        alter_ground(&tree(), &mut replaced);
        replaced
    }

    /// Moss where the provider it found places podzol: a mod that wraps the one before.
    struct Moss(GroundProvider);
    impl GroundStateProvider for Moss {
        fn state_at(&self, level: &mut dyn GroundLevel, pos: [i32; 3]) -> Option<ApiState> {
            let found = self.0.state_at(level, pos);
            if found == Some(api(PODZOL)) {
                Some(api(MOSS))
            } else {
                found
            }
        }
    }
    /// Mud where the provider it found places moss, after one draw of its own.
    struct Mud(GroundProvider);
    impl GroundStateProvider for Mud {
        fn state_at(&self, level: &mut dyn GroundLevel, pos: [i32; 3]) -> Option<ApiState> {
            level.random().next_int_bound(2);
            let found = self.0.state_at(level, pos);
            if found == Some(api(MOSS)) {
                Some(api(MUD))
            } else {
                found
            }
        }
    }

    /// Swaps in its provider, recording what the event and the context told it.
    struct Swaps {
        wrap: fn(GroundProvider) -> Arc<dyn GroundStateProvider>,
        heard: Arc<Mutex<Vec<String>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Swaps {
        fn alter_ground(&self, context: &dyn WorldgenHost, event: &mut AlterGroundEvent<'_>) {
            let blocks = context.registry("block").map(|names| names.join(","));
            self.heard.lock().unwrap().push(format!(
                "{:?} {:?} {}",
                event.positions,
                blocks,
                matches!(event.provider, GroundProvider::Decorator)
            ));
            *event.provider = GroundProvider::Mod((self.wrap)(event.provider.clone()));
        }
    }
    /// Listens and leaves the decorator's own provider.
    struct Keeps;
    #[mod_api::__subscriptions]
    impl Hooks for Keeps {
        fn alter_ground(&self, _: &dyn WorldgenHost, event: &mut AlterGroundEvent<'_>) {
            assert!(matches!(event.provider, GroundProvider::Decorator));
        }
    }

    #[test]
    fn with_no_listener_or_the_decorators_own_kept_nothing_is_replaced() {
        testing::with_mods(Vec::<ModEntry>::new().into(), || assert!(alter().is_none()));
        testing::with_mods(vec![ModEntry::new("keeps", Box::new(Keeps))].into(), || {
            assert!(alter().is_none());
        });
    }

    #[test]
    fn a_mod_swaps_the_provider_and_sees_the_tree_and_the_servers_registries() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "moss",
            Box::new(Swaps {
                wrap: |found| Arc::new(Moss(found)),
                heard: heard.clone(),
            }),
        )];
        let replaced = testing::with_mods(mods.into(), alter).expect("the mod's provider");
        assert_eq!(
            *heard.lock().unwrap(),
            [r#"[[0, 64, 0]] Some("minecraft:moss_block") true"#]
        );
        let mut meadow = Meadow::default();
        assert_eq!(replaced.state_at(&mut meadow, [3, 63, 0]), Some(MOSS));
        // The wrapped decorator's own provider drew once, as it does alone.
        assert_eq!(meadow.draws, 1);
    }

    #[test]
    fn each_mod_wraps_the_provider_the_one_before_left() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new(
                "moss",
                Box::new(Swaps {
                    wrap: |found| Arc::new(Moss(found)),
                    heard: heard.clone(),
                }),
            ),
            ModEntry::new(
                "mud",
                Box::new(Swaps {
                    wrap: |found| Arc::new(Mud(found)),
                    heard: heard.clone(),
                }),
            ),
        ];
        let replaced = testing::with_mods(mods.into(), alter).expect("the mods' provider");
        let decorator = r#"[[0, 64, 0]] Some("minecraft:moss_block")"#;
        assert_eq!(
            *heard.lock().unwrap(),
            [format!("{decorator} true"), format!("{decorator} false")]
        );
        let mut meadow = Meadow::default();
        assert_eq!(replaced.state_at(&mut meadow, [3, 63, 0]), Some(MUD));
        // Mud's own draw, then the decorator's, in the order the providers asked.
        assert_eq!(meadow.draws, 2);
    }
}
