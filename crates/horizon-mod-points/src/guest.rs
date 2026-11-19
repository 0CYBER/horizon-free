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
        mod_api::wire::guest::extend(
            mod_api::wire::POINTS,
            mod_api::wire::POINTS_END,
            crate::guest_dispatch,
        );
        Ok(())
    } else {
        Err(crate::RegisterError::Unsupported)
    }
}

/// The places this guest listens at, in registration order: each with its own place id, the
/// place's name and identity (`point_identity`), which the game checks by name, and the
/// listener's inject id.
pub fn subscriptions() -> crate::Subscriptions {
    LISTENERS.with(|slots| {
        slots
            .borrow()
            .iter()
            .map(|(place, injection, _)| {
                let (name, identity) = crate::API_PLACES
                    .iter()
                    .find(|(point, ..)| *point as usize == *place)
                    .map_or(("", 0), |(point, ..)| crate::point_identity(*point));
                (*place as u32, name.to_owned(), identity, injection.clone())
            })
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
    crate::points_layout()
}
