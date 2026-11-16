use mod_api::__private::game::ServerReloadListenersOrder;
use mod_api::*;
use std::sync::Arc;

fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn files() -> FileToIdConverter {
    FileToIdConverter::json("machines").unwrap()
}

#[test]
fn add_listener_attributes_its_calls_to_the_registering_mod() {
    struct Noop;
    impl ServerReloadListener for Noop {
        fn prepare(
            &self,
            _: &ServerReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            Ok(<dyn ReloadApply>::new(|| {}))
        }
    }
    let guard = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "test_mod",
        hook: "add_server_reload_listeners",
        part: None,
    });
    let mut owned = ServerListenerRegistry::new();
    owned
        .add_listener(id("test_mod:data"), files(), Arc::new(Noop))
        .unwrap();
    drop(guard);
    let mut unowned = ServerListenerRegistry::new();
    unowned
        .add_listener(id("game:data"), files(), Arc::new(Noop))
        .unwrap();
    let listener = owned
        .finish()
        .unwrap()
        .listeners
        .into_iter()
        .find(|listener| listener.id == id("test_mod:data"))
        .unwrap();
    assert_eq!(listener.owner, Some("test_mod"));
    let listener = unowned
        .finish()
        .unwrap()
        .listeners
        .into_iter()
        .find(|listener| listener.id == id("game:data"))
        .unwrap();
    assert_eq!(listener.owner, None);
}
