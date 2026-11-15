//! Shared native/compiled-guest listener; all replies are independent literal edits.
use mod_api::{Flow, Hooks, MixedOutputFixtureEvent, ServerHost};

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn mixed_output_fixture(
        &self,
        _: &mut dyn ServerHost,
        event: &mut MixedOutputFixtureEvent<'_>,
    ) -> Flow {
        match event.mode {
            0 | 2 => {
                assert_eq!(
                    event
                        .drops
                        .iter()
                        .map(|stack| stack.count())
                        .collect::<Vec<_>>(),
                    [2]
                );
                assert_eq!((event.damage, event.cancelled), (0, false));
                event.drops[0].set_count(65).unwrap();
                event.damage = 7;
                Flow::Cancel
            }
            1 => {
                let mut second = event.drops[0].clone();
                event.drops[0].set_count(3).unwrap();
                second.set_count(4).unwrap();
                event.drops.push(second);
                event.damage = 9;
                Flow::Continue
            }
            3 => {
                assert_eq!(
                    event
                        .drops
                        .iter()
                        .map(|stack| stack.count())
                        .collect::<Vec<_>>(),
                    [3, 4]
                );
                assert_eq!((event.damage, event.cancelled), (9, false));
                event.drops[0].set_count(66).unwrap();
                event.damage += 2;
                Flow::Continue
            }
            4 => {
                event.damage = 13;
                Flow::Cancel
            }
            _ => panic!("unknown mixed-output fixture case"),
        }
    }
}
