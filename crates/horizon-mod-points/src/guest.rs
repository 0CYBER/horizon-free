//! Guest listeners sharing the ordinary hook export and wire arena.
use crate::PointId;
use mod_api::__private::points::{ApiKind, SlotListener};
use std::cell::RefCell;

thread_local! {
    static LISTENERS: RefCell<Vec<(usize, String, *const ())>> = const { RefCell::new(Vec::new()) };
}

/// Installs a typed API listener at a game point in this guest and enables the generated export
/// table: the host registers the subscribed places in its ordinary ordered listener list.
pub fn register(listener: &SlotListener) -> Result<(), crate::RegisterError> {
    if let Some(ApiKind::Typed(function)) = listener.api_kind() {
        LISTENERS.with(|slots| {
            slots.borrow_mut().push((
                listener.place().index(),
                listener.injection().to_owned(),
                function,
            ))
        });
        mod_api::wire::guest::extend(0x8000_0000, 0xC000_0000, crate::guest_dispatch);
        Ok(())
    } else {
        Err(crate::RegisterError::Unsupported)
    }
}

/// The places this guest listens at, in registration order.
pub fn subscriptions() -> Vec<(u32, String)> {
    LISTENERS.with(|slots| {
        slots
            .borrow()
            .iter()
            .map(|(place, injection, _)| (*place as u32, injection.clone()))
            .collect()
    })
}

pub(crate) fn fire(
    place: PointId,
    index: u32,
    call: impl FnOnce(*const ()),
) -> Result<(), mod_api::wire::WireError> {
    let function = LISTENERS.with(|slots| {
        slots
            .borrow()
            .get(index as usize)
            .filter(|(at, _, _)| *at == place as usize)
            .map(|(_, _, function)| *function)
    });
    let function = function.ok_or(mod_api::wire::WireError::Invalid("unknown point listener"))?;
    call(function);
    Ok(())
}

/// The full point catalog layout, checked by the host before this guest can subscribe.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn horizon_layout_ext() -> u64 {
    crate::POINT_LAYOUT
}
