#![cfg(test)]
// Mutable slot and cancellation contracts, separate from engine brewing rules.
use crate::*;
use std::sync::{Arc, Mutex};

struct NoHost;
impl Host for NoHost {}
crate::impl_host_areas!(NoHost);
impl ServerHost for NoHost {}
crate::impl_server_host_areas!(NoHost);

struct BrewModifier {
    cancel: bool,
    seen: Arc<Mutex<Vec<String>>>,
}

#[crate::__subscriptions]
impl Hooks for BrewModifier {
    fn potion_brew_pre(&self, _: &mut dyn ServerHost, event: &mut PotionBrewPreEvent<'_>) -> Flow {
        self.seen.lock().unwrap().push("pre".into());
        event.stacks[0] = Some(crate::__private::part_tests::stack(
            "minecraft:diamond",
            1,
            0,
        ));
        if self.cancel {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn potion_brew_post(&self, _: &mut dyn ServerHost, event: &mut PotionBrewPostEvent<'_>) {
        let first = event.stacks[0]
            .as_ref()
            .and_then(|stack| crate::__private::item_key_id(stack.item()))
            .unwrap();
        self.seen.lock().unwrap().push(format!("post {first}"));
        event.stacks[1] = Some(crate::__private::part_tests::stack(
            "minecraft:emerald",
            1,
            0,
        ));
    }
}

#[test]
fn brewing_callbacks_keep_owned_slot_edits_and_report_cancellation() {
    install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| false).unwrap();
    for cancel in [true, false] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "brew-modifier",
            Box::new(BrewModifier {
                cancel,
                seen: seen.clone(),
            }),
        )]
        .into();
        testing::with_mods(mods, || {
            let mut stacks = [
                Some(crate::__private::part_tests::stack(
                    "minecraft:potion",
                    1,
                    0,
                )),
                None,
                None,
                None,
                None,
            ];
            let mut cancelled = false;
            dispatch_potion_brew_pre(&mut NoHost, [1, 2, 3], &mut stacks, &mut cancelled);
            assert_eq!(cancelled, cancel);
            assert_eq!(
                stacks[0],
                Some(crate::__private::part_tests::stack(
                    "minecraft:diamond",
                    1,
                    0
                ))
            );
            // The engine decides whether to apply the pre slots. Dispatch itself
            // threads the caller's owned values through either contract.
            dispatch_potion_brew_post(&mut NoHost, [1, 2, 3], &mut stacks);
            assert_eq!(
                stacks[0],
                Some(crate::__private::part_tests::stack(
                    "minecraft:diamond",
                    1,
                    0
                ))
            );
            assert_eq!(
                stacks[1],
                Some(crate::__private::part_tests::stack(
                    "minecraft:emerald",
                    1,
                    0
                ))
            );
        });
        assert_eq!(*seen.lock().unwrap(), ["pre", "post minecraft:diamond"]);
    }
}

#[cfg(test)]
use super::*;

#[cfg(test)]
use crate::testing;
