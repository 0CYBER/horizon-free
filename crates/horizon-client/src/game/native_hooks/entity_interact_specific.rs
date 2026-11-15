use super::*;

impl Game<'_> {
    /// a right-click on an entity with the point clicked (`native_entity_interact_specific`).
    pub(in crate::game) fn native_entity_interact_specific(
        &mut self,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        location: [f64; 3],
        claimed: &mut bool,
    ) {
        self.fire_entity_interact_specific(entity, kind, stack, location, claimed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Probe {
        name: &'static str,
        claim: bool,
        seen: Arc<Mutex<Vec<(&'static str, [f64; 3])>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Probe {
        fn entity_interact_specific(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityInteractSpecificEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.entity, 7);
            assert_eq!(event.kind, "minecraft:cow");
            assert_eq!(event.stack.unwrap().item().id(host), "minecraft:stick");
            self.seen.lock().unwrap().push((self.name, event.location));
            if self.claim {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }
    #[test]
    #[ignore = "daily"]
    fn specific_click_keeps_location_and_stops_after_a_claim() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            for claim in [false, true] {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![
                    mod_api::ModEntry::new(
                        "first",
                        Box::new(Probe {
                            name: "first",
                            claim,
                            seen: seen.clone(),
                        }),
                    ),
                    mod_api::ModEntry::new(
                        "second",
                        Box::new(Probe {
                            name: "second",
                            claim: false,
                            seen: seen.clone(),
                        }),
                    ),
                ];
                mod_platform::testing::with_mods(mods.into(), || {
                    let mut claimed = false;
                    let stack = ItemStack::new("minecraft:stick", 1);
                    let at = [0.25, 1.5, -0.125];
                    game.native_entity_interact_specific(
                        7,
                        "minecraft:cow",
                        Some(&stack),
                        at,
                        &mut claimed,
                    );
                    assert_eq!(claimed, claim);
                    let mut expected = vec![("first", at)];
                    if !claim {
                        expected.push(("second", at));
                    }
                    assert_eq!(*seen.lock().unwrap(), expected);
                });
            }
        });
    }
}
