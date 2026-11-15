/// While a hopper cools down, takes one tick off its real cooldown and skips its tick.
#[derive(Default)]
pub struct Throttle {
    skipped: u64,
}

impl BlockEntityTickSlot for Throttle {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if tick.kind != TickedKind::Hopper {
            return LoopFlow::Continue;
        }
        match tick.game.hoppers.get_mut(&tick.pos) {
            Some(state) if state.cooldown > 1 => {
                let cooldown = state.cooldown;
                state.cooldown -= 1;
                self.skipped += 1;
                if self.skipped % 1000 == 1 {
                    fnr_loop::note(&format!("throttle skips #{}: cooldown {cooldown}", self.skipped));
                }
                LoopFlow::Skip
            }
            _ => LoopFlow::Continue,
        }
    }
}
