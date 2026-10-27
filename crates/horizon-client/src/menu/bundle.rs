//! `BundleItem`'s click overrides in a PICKUP click
//! (`tryItemClickBehaviourOverride`).
use super::{Env, Menu, tidy};
use crate::bundle::{Contents, INSERT, INSERT_FAIL, REMOVE_ONE, can_be_in_bundle, toggle_selected};

impl Menu {
    /// The carried stack's `overrideStackedOnOther`, then the clicked
    /// one's `overrideOtherStackedOnMe`. True when a bundle took the click.
    pub(super) fn bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        let took = self.carried_bundle_click(env, s, primary)
            || self.clicked_bundle_click(env, s, primary);
        if took {
            // broadcastChangesOnContainerMenu: the open menu's
            // slotsChanged for the player's inventory.
            self.inventory_slots_changed(env);
        }
        took
    }

    /// `BundleItem.overrideStackedOnOther`: the carried bundle on a slot.
    fn carried_bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        let Some(mut contents) = self
            .carried
            .as_ref()
            .and_then(|c| Contents::of(env.items, c))
        else {
            return false;
        };
        match (self.slot(s).cloned(), primary) {
            (Some(other), true) => {
                // tryTransfer: what fits, taken by the slot's rules.
                let mut moved = 0;
                if can_be_in_bundle(&other) {
                    let room = contents.room_for(env.items, &other);
                    if let Some(mut taken) = self.safe_take(env, s, other.count, room) {
                        moved = contents.try_insert(env.items, &mut taken);
                    }
                }
                self.player_sounds
                    .push(if moved > 0 { INSERT } else { INSERT_FAIL });
            }
            (None, false) => {
                if let Some(stack) = contents.remove_one(env.items) {
                    let count = stack.count;
                    let mut remainder = Some(stack);
                    self.safe_insert(env, s, &mut remainder, count);
                    match remainder {
                        Some(mut rest) => {
                            contents.try_insert(env.items, &mut rest);
                        }
                        None => self.player_sounds.push(REMOVE_ONE),
                    }
                }
            }
            _ => return false,
        }
        if let Some(carried) = self.carried.as_mut() {
            contents.store(carried);
        }
        true
    }

    /// `BundleItem.overrideOtherStackedOnMe`: a click on a bundle in a slot.
    fn clicked_bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        let Some(mut bundle) = self.slot(s).cloned() else {
            return false;
        };
        let Some(mut contents) = Contents::of(env.items, &bundle) else {
            return false;
        };
        let allowed = self.allow_modification(env, s);
        match (self.carried.take(), primary) {
            (Some(mut other), true) => {
                let moved = if allowed {
                    contents.try_insert(env.items, &mut other)
                } else {
                    0
                };
                self.player_sounds
                    .push(if moved > 0 { INSERT } else { INSERT_FAIL });
                self.carried = Some(other);
                tidy(&mut self.carried);
            }
            (None, false) => {
                if allowed && let Some(removed) = contents.remove_one(env.items) {
                    self.player_sounds.push(REMOVE_ONE);
                    self.carried = Some(removed);
                }
            }
            (carried, _) => {
                // toggleSelectedItem(-1), then the click goes on.
                self.carried = carried;
                toggle_selected(&mut bundle, -1);
                let slot = self.slots[s];
                self.containers[slot.container][slot.index] = Some(bundle);
                return false;
            }
        }
        // `self.set(BUNDLE_CONTENTS, ...)` changes the slot's stack in place.
        contents.store(&mut bundle);
        let slot = self.slots[s];
        self.containers[slot.container][slot.index] = Some(bundle);
        true
    }
}
