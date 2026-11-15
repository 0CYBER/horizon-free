use super::*;

impl GameLevel<'_> {
    /// The fluid at `pos` is about to run its scheduled tick (`FlowingFluid.tick`). `fluid` is
    /// `water`, `lava` or a mod fluid's id. Set `cancel` and the tick does nothing at all: no
    /// level change, no spreading and no new tick. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_fluid_tick(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        fluid: &str,
        cancel: &mut bool,
    ) {
        self.fire_fluid_tick(dimension, pos, fluid, cancel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{FluidTickEvent, Hooks, LevelHost, Registrar};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct CancelWater(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for CancelWater {
        fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}
        fn fluid_tick(&self, _: &dyn LevelHost, event: &mut FluidTickEvent<'_>) -> mod_api::Flow {
            assert_eq!(event.dimension, LevelDimension::Overworld);
            assert_eq!(event.pos, BlockPos::new(5, 64, 7));
            assert_eq!(event.fluid, "water");
            self.0.fetch_add(1, Ordering::SeqCst);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn fluid_tick_dispatch_keeps_arguments_cancel_and_authority() {
        let heard = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cancel-water",
            Box::new(CancelWater(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let pos = BlockPos::new(5, 64, 7);
                let flowing = game
                    .pack
                    .assets
                    .registry
                    .resolve(&horizon_world::chunk::BlockState {
                        name: "minecraft:water".into(),
                        properties: [("level".into(), "3".into())].into(),
                    })
                    .unwrap();
                game.world.set(pos.x, pos.y, pos.z, flowing).unwrap();
                game.with_level(|behaviours, level, ticks| {
                    level.native_authoritative = true;
                    assert!(!horizon_world::level::LevelAccess::allow_fluid_tick(
                        level, pos, "water"
                    ));
                    let before = ticks.fluids.count();
                    behaviours.scheduled_fluid_tick(level, ticks, pos);
                    assert_eq!(
                        ticks.fluids.count(),
                        before,
                        "cancel adds no scheduled tick"
                    );
                    level.native_authoritative = false;
                    assert!(horizon_world::level::LevelAccess::allow_fluid_tick(
                        level, pos, "water"
                    ));
                });
                assert_eq!(game.world.block(pos.x, pos.y, pos.z), flowing);
                assert_eq!(
                    heard.load(Ordering::SeqCst),
                    2,
                    "non-authoritative call never fires"
                );
            });
        });
    }
}
