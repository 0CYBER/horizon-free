use mod_api::wire::{Id, ResourceCodec, ResourceScope, WireError, Writer};
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    rc::Rc,
};

#[test]
fn byte_limits_refuse_before_allocating_and_reset_with_the_buffer() {
    use mod_api::wire::{Record, Refused};
    let mut refused = Refused(WireError::Length);
    let mut rejected = Writer::new();
    refused.encode(&mut rejected);
    assert_eq!(rejected.error(), Some(WireError::Length));
    assert_eq!(refused.apply(&[]), Err(WireError::Length));
    let mut writer = Writer::new();
    writer.limit(8);
    writer.put_bytes(&[0; 1024]);
    assert_eq!(writer.error(), Some(WireError::Length));
    assert!(writer.bytes().is_empty());
    assert_eq!(writer.into_bytes().capacity(), 0);

    let mut writer = Writer::new();
    writer.limit(8);
    writer.put_str("12345"); // Nine bytes including the length.
    assert_eq!(writer.error(), Some(WireError::Length));
    assert!(writer.bytes().is_empty());
    writer.fail(WireError::Utf8);
    assert_eq!(
        writer.error(),
        Some(WireError::Length),
        "the first error wins"
    );
    writer.clear();
    writer.put_str("12345");
    assert_eq!(writer.error(), None);
    assert_eq!(writer.bytes().len(), 9);
    writer.limit(0);
    assert_eq!(writer.error(), Some(WireError::Length));
    writer.replace(Vec::new());
    writer.put(&9u32);
    assert_eq!(writer.error(), None);
    assert_eq!(writer.bytes(), 9u32.to_le_bytes());
}

#[test]
fn an_encoding_refusal_does_not_consume_resource_registry_state() {
    struct Registry(RefCell<Vec<Box<dyn Any>>>);
    impl ResourceCodec for Registry {
        fn store(&self, value: Box<dyn Any>) -> Result<Id, WireError> {
            self.0.borrow_mut().push(value);
            Ok(Id(7))
        }
        fn load(&self, _: Id, _: TypeId) -> Result<Box<dyn Any>, WireError> {
            Err(WireError::Invalid("not used"))
        }
    }
    let registry = Rc::new(Registry(RefCell::new(Vec::new())));
    let _scope = ResourceScope::new(registry.clone());
    let mut writer = Writer::new();
    writer.limit(7); // An id needs eight bytes: refuse before storing it.
    writer.put_resource(Box::new(12u32));
    assert!(registry.0.borrow().is_empty());
    assert_eq!(writer.error(), Some(WireError::Length));
    writer.put_resource(Box::new(99u32));
    assert!(registry.0.borrow().is_empty());
    writer.clear();
    writer.put_resource(Box::new(42u32));
    assert_eq!(registry.0.borrow().len(), 1);
    assert_eq!(registry.0.borrow()[0].downcast_ref::<u32>(), Some(&42));
    assert_eq!(writer.bytes(), 7u64.to_le_bytes());
}
