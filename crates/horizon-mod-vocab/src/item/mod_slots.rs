//! The entry points of `mod_slot_will_change` and `mod_slot_changed`.
use crate::item::item_stack::ItemStack;
#[allow(unused_imports)]
use crate::*;

/// Ask the pre-change listeners whether the change may go ahead. `cancel`
/// starts false; a listener may set it. Called by a mod from its own
/// transaction, with the host it is running under.
#[inline(never)]
pub fn native_mod_slot_will_change(
    host: &dyn Host,
    holder: EntityRef,
    slot: &str,
    index: usize,
    cosmetic: bool,
    old: Option<&ItemStack>,
    new: Option<&ItemStack>,
    source: &str,
    cancel: &mut bool,
) {
    crate::dispatch_mod_slot_will_change(
        host,
        crate::__private::ModSlotWillChangeCall {
            holder,
            slot,
            index,
            cosmetic,
            old,
            new,
            source,
            cancel,
        },
    );
}

/// Tell the listeners a mod's slot change has been committed.
#[inline(never)]
pub fn native_mod_slot_changed(
    host: &dyn Host,
    holder: EntityRef,
    slot: &str,
    index: usize,
    cosmetic: bool,
    old: Option<&ItemStack>,
    new: Option<&ItemStack>,
    source: &str,
) {
    crate::dispatch_mod_slot_changed(
        host,
        crate::__private::ModSlotChangedCall {
            holder,
            slot,
            index,
            cosmetic,
            old,
            new,
            source,
        },
    );
}
