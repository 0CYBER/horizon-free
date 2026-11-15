use super::*;
thread_local! { static ANSWER: std::cell::Cell<f32> = const { std::cell::Cell::new(0.98) }; }
struct Surface;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Surface {
    fn friction(&self, context: &dyn LoopContext, event: &mut mod_api::FrictionEvent<'_>) {
        assert_eq!(context.side(), mod_api::LoopSide::Server);
        assert_eq!(event.entity, 7);
        assert_eq!(event.kind, "minecraft:oak_boat");
        assert_eq!(event.pos, mod_api::BlockPos::new(5, 64, 4));
        assert_eq!(
            event.state,
            mod_api::BlockStateRecord::new(REGISTERED_BLOCK.get().unwrap(), 0).into()
        );
        assert_eq!(event.friction, 0.6);
        event.friction = ANSWER.get();
    }
}
#[test]
fn wire_loopback_friction_preserves_context_inputs_and_float_writeback() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockPart>(Arc::new(
        Surface,
    ))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    for answer in [0.98, f32::NAN, f32::INFINITY, -0.5, 3.0] {
        ANSWER.set(answer);
        let mut event = mod_api::FrictionEvent {
            entity: 7,
            kind: "minecraft:oak_boat",
            pos: mod_api::BlockPos::new(5, 64, 4),
            state: mod_api::BlockStateRecord::new(REGISTERED_BLOCK.get().unwrap(), 0).into(),
            friction: 0.6,
        };
        mod_api::BlockPart::friction(&proxy, &Context, &mut event);
        assert_eq!(event.friction.to_bits(), answer.to_bits());
    }
}
