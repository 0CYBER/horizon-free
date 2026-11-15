//! `BundleItem`'s click overrides in a PICKUP click
//! (`tryItemClickBehaviourOverride`).
use super::container_items::{can_be_in_bundle, fits_inside_container_items};
use super::{Env, Menu, tidy};
use crate::bundle::{Contents, INSERT, INSERT_FAIL, REMOVE_ONE, toggle_selected};

impl Menu {
    /// The carried stack's `overrideStackedOnOther`, then the clicked
    /// one's `overrideOtherStackedOnMe`. True when a bundle took the click.
    pub(super) fn bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        self.touch();
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
        self.touch();
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
                if can_be_in_bundle(env.loop_context, &other) {
                    let room = contents.room_for(env.items, &other);
                    if let Some(mut taken) = self.safe_take(env, s, other.count, room) {
                        // `tryInsert` asks `canItemBeInBundle` again: the answer above.
                        moved = contents.try_insert(env.items, &mut taken, true);
                    }
                }
                self.state
                    .player_sounds
                    .push(if moved > 0 { INSERT } else { INSERT_FAIL });
            }
            (None, false) => {
                if let Some(stack) = contents.remove_one(env.items) {
                    let count = stack.count;
                    let mut remainder = Some(stack);
                    self.safe_insert(env, s, &mut remainder, count);
                    match remainder {
                        Some(mut rest) => {
                            let fits = fits_inside_container_items(env.loop_context, &rest);
                            contents.try_insert(env.items, &mut rest, fits);
                        }
                        None => self.state.player_sounds.push(REMOVE_ONE),
                    }
                }
            }
            _ => return false,
        }
        if let Some(carried) = self.state.carried.as_mut() {
            contents.store(carried);
        }
        true
    }

    /// `BundleItem.overrideOtherStackedOnMe`: a click on a bundle in a slot.
    fn clicked_bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        self.touch();
        let Some(mut bundle) = self.slot(s).cloned() else {
            return false;
        };
        let Some(mut contents) = Contents::of(env.items, &bundle) else {
            return false;
        };
        let allowed = self.allow_modification(env, s);
        match (self.state.carried.take(), primary) {
            (Some(mut other), true) => {
                let moved = if allowed {
                    let fits = fits_inside_container_items(env.loop_context, &other);
                    contents.try_insert(env.items, &mut other, fits)
                } else {
                    0
                };
                self.state
                    .player_sounds
                    .push(if moved > 0 { INSERT } else { INSERT_FAIL });
                self.state.carried = Some(other);
                tidy(&mut self.state.carried);
            }
            (None, false) => {
                if allowed && let Some(removed) = contents.remove_one(env.items) {
                    self.state.player_sounds.push(REMOVE_ONE);
                    self.state.carried = Some(removed);
                }
            }
            (carried, _) => {
                // toggleSelectedItem(-1), then the click goes on.
                self.state.carried = carried;
                toggle_selected(&mut bundle, -1);
                let slot = self.state.slots[s];
                self.state.containers[slot.container][slot.index] = Some(bundle);
                return false;
            }
        }
        // `self.set(BUNDLE_CONTENTS, ...)` changes the slot's stack in place.
        contents.store(&mut bundle);
        let slot = self.state.slots[s];
        self.state.containers[slot.container][slot.index] = Some(bundle);
        true
    }
}
