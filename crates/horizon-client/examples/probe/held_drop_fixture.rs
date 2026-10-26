//! Selected-slot drop state for the vanilla key-handler parity harness.
use horizon_client::command::args::GameMode;
use horizon_client::items::{ItemStack, take_held_drop};

pub fn main() {
    for mode in [
        GameMode::Survival,
        GameMode::Creative,
        GameMode::Adventure,
        GameMode::Spectator,
    ] {
        for all in [false, true] {
            for _selected in [0, 8] {
                for count in [0, 1, 4, 64] {
                    let mut slot = Some(ItemStack::new("minecraft:stone", count));
                    let mut dropped = 0;
                    let mut swings = 0;
                    let mut packets = 0;
                    for _ in 0..2 {
                        if let Some(stack) =
                            take_held_drop(&mut slot, all, mode == GameMode::Spectator)
                        {
                            dropped += stack.count;
                            swings += 1;
                        }
                        if mode != GameMode::Spectator {
                            packets += 1;
                        }
                        println!(
                            "{} {dropped} {swings} {packets}",
                            slot.as_ref().map_or(0, |s| s.count),
                        );
                    }
                }
            }
        }
    }
}
