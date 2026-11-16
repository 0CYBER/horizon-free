use super::*;
fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn files() -> FileToIdConverter {
    FileToIdConverter::json("machines").unwrap()
}

#[test]
fn registry_views_and_explicit_order_before_vanilla() {
    let mut listeners = ClientListenerRegistry::new();
    let first = Arc::new(Boundary);
    let second = Arc::new(Boundary);
    listeners
        .add_listener(id("test_mod:first"), files(), first.clone())
        .unwrap();
    listeners
        .add_listener(id("test_mod:second"), files(), second.clone())
        .unwrap();
    assert_eq!(
        listeners.get_last_vanilla_listener(),
        VanillaClientListeners::model_manager()
    );
    assert_eq!(
        listeners.get_registry(),
        [
            VanillaClientListeners::first(),
            VanillaClientListeners::model_manager(),
            VanillaClientListeners::last(),
            id("test_mod:first"),
            id("test_mod:second"),
        ]
    );
    listeners
        .add_dependency(
            id("test_mod:second"),
            VanillaClientListeners::model_manager(),
        )
        .unwrap();
    assert!(listeners.get_graph().contains(&(
        id("test_mod:second"),
        VanillaClientListeners::model_manager()
    )));
    let sorted = listeners.finish().unwrap();
    assert_eq!(
        sorted.into_iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [
            VanillaClientListeners::first(),
            id("test_mod:second"),
            VanillaClientListeners::model_manager(),
            id("test_mod:first"),
            VanillaClientListeners::last(),
        ]
    );
}

#[test]
fn failed_registration_is_not_silenced_by_ignoring_the_result() {
    let mut listeners = ClientListenerRegistry::new();
    assert_eq!(
        listeners
            .add_dependency(
                id("mistyped:listener"),
                VanillaClientListeners::model_manager()
            )
            .unwrap_err(),
        ReloadOrderError::UnknownListener {
            id: id("mistyped:listener")
        }
    );
    assert!(matches!(
        listeners.finish(),
        Err(ReloadOrderError::UnknownListener { .. })
    ));
}

#[test]
fn duplicate_objects_cannot_make_the_reverse_lookup_ambiguous() {
    let mut listeners = ClientListenerRegistry::new();
    let listener = Arc::new(Boundary);
    listeners
        .add_listener(id("one:listener"), files(), listener.clone())
        .unwrap();
    assert_eq!(
        listeners.add_listener(id("two:listener"), files(), listener),
        Err(ReloadOrderError::DuplicateObject {
            id: id("two:listener"),
            registered_as: id("one:listener"),
        })
    );
}
