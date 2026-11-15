use mod_vocab::wire::{Id, Reader, ResourceAccess, WireError, Writer, install_resource_access};

#[test]
fn unregistered_refusal_allows_retry_but_installed_access_is_not_replaced() {
    let mut out = Writer::new();
    out.put_resource(Box::new(42u32));
    assert_eq!(out.error(), Some(WireError::Invalid("no resource context")));
    assert!(out.bytes().is_empty());
    let encoded = [7, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        Reader::new(&encoded).take_resource::<u32>(),
        Err(WireError::Invalid("no resource context"))
    );

    install_resource_access(ResourceAccess {
        intern: |_| Ok("fixture"),
        store: |value| {
            assert_eq!(*value.downcast::<u32>().unwrap(), 42);
            Ok(Id(7))
        },
        load: |id, view| {
            assert_eq!(id, Id(7));
            assert_eq!(view, std::any::TypeId::of::<u32>());
            Ok(Box::new(42u32))
        },
        refused: |_| {},
    })
    .unwrap();
    assert!(
        install_resource_access(ResourceAccess {
            intern: |_| Ok("fixture"),
            store: |_| panic!("second backend replaced the first"),
            load: |_, _| panic!("second backend replaced the first"),
            refused: |_| panic!("second backend replaced the first"),
        })
        .is_err()
    );

    out.clear();
    out.put_resource(Box::new(42u32));
    assert_eq!(out.error(), None);
    assert_eq!(out.bytes(), encoded);
    assert_eq!(Reader::new(out.bytes()).take_resource::<u32>(), Ok(42));
}
