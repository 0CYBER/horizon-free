//! A mod for the runtime's tests: `/spin` loops forever, `/panic` panics,
//! `/grow` asks for a gigabyte, `/hello` chats; `tick` and `frame` do
//! nothing, to measure the cost of a call.
use horizon_mod_sdk::{Mod, Registration, chat};

struct Guest {
    ticks: u64,
}

impl Mod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        let hooks = ["spin", "panic", "grow", "hello"]
            .into_iter()
            .fold(Registration::new(), Registration::command)
            .on_tick()
            .on_frame();
        Ok((Guest { ticks: 0 }, hooks))
    }
    fn tick(&mut self, _tick: u64) {
        self.ticks += 1;
    }
    fn command(&mut self, name: &str, args: &str) -> Result<Option<String>, String> {
        match name {
            "spin" => {
                let mut n = 0u64;
                while std::hint::black_box(n) != u64::MAX {
                    n = n.wrapping_add(2);
                }
                Ok(None)
            }
            "panic" => panic!("asked to"),
            "grow" => {
                let big = std::hint::black_box(vec![1u8; 1 << 30]);
                Ok(Some(format!("{}", big.len())))
            }
            _ => {
                chat::show(&format!("hello {args} after {} ticks", self.ticks));
                Ok(None)
            }
        }
    }
}

horizon_mod_sdk::export_mod!(Guest);
