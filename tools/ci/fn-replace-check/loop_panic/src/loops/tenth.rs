#[derive(Default)]
pub struct Tenth {
    hoppers: u32,
}

impl BlockEntityTickSlot for Tenth {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if tick.kind == TickedKind::Hopper {
            self.hoppers += 1;
            if self.hoppers == 10 {
                panic!("fnr: loop_panic on the tenth hopper, at {:?}", tick.pos);
            }
        }
        LoopFlow::Continue
    }
}
