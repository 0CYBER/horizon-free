//! Plain builds must not accept listeners that can never fire.
use mod_api::__private::points::{Forward, Place, PointHost, SlotListener};
use mod_points::RegisterError;

#[test]
fn native_listener_is_refused_by_mod_and_target() {
    if mod_points::STAGED {
        return;
    }
    let slot = mod_points::listeners::tick("named_mod", 0, "head", tick);
    // The one boundary every registration passes, not a wrapper of it.
    let error = mod_api::__private::points::register(&slot).unwrap_err();
    assert!(matches!(error, RegisterError::Unstaged { .. }));
    let message = error.to_string();
    assert!(
        message.contains("named_mod")
            && message.contains("::tick")
            && message.contains("no point sites"),
        "{message}"
    );
}
fn tick(_: mod_api::HostSide<'_>, _: &mut mod_points::TickArgs) {}

/// A place built outside its table is refused by name, staged or not: no index is trusted.
#[test]
fn a_place_outside_the_table_is_refused() {
    struct Never;
    impl Forward for Never {
        fn forward(&self, _: u32, _: PointHost<'_>, _: &mut dyn mod_api::wire::Record) {}
    }
    let place = Place::new(&mod_points::__private::TABLE, usize::MAX);
    let slot = SlotListener::forwarded("outside_mod", place, &Never, 0);
    let error = mod_api::__private::points::register(&slot).unwrap_err();
    assert_eq!(
        error,
        RegisterError::NoPlace {
            id: "outside_mod",
            index: usize::MAX
        }
    );
    assert_eq!(
        error.to_string(),
        format!(
            "outside_mod: place {} is outside its point table",
            usize::MAX
        )
    );
}
