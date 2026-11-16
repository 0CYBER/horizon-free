use mod_api::wire;
#[test]
fn resource_context_is_scoped_and_encoding_keeps_first_error() {
    use std::{
        any::{Any, TypeId},
        cell::RefCell,
        rc::Rc,
    };
    struct Codec(RefCell<Option<Box<dyn Any>>>);
    impl wire::ResourceCodec for Codec {
        fn store(&self, value: Box<dyn Any>) -> Result<wire::Id, wire::WireError> {
            *self.0.borrow_mut() = Some(value);
            Ok(wire::Id(7))
        }
        fn load(&self, id: wire::Id, _: TypeId) -> Result<Box<dyn Any>, wire::WireError> {
            if id != wire::Id(7) {
                return Err(wire::WireError::Invalid("stale resource"));
            }
            self.0
                .borrow_mut()
                .take()
                .ok_or(wire::WireError::Invalid("stale resource"))
        }
    }
    let mut out = wire::Writer::new();
    {
        let _scope = wire::ResourceScope::new(Rc::new(Codec(RefCell::new(None))));
        out.put_resource(Box::new(42u32));
        assert_eq!(out.error(), None);
        assert_eq!(
            wire::Reader::new(out.bytes()).take_resource::<u32>(),
            Ok(42)
        );
    }
    assert_eq!(
        wire::Reader::new(out.bytes()).take_resource::<u32>(),
        Err(wire::WireError::Invalid("no resource context"))
    );
    out.fail(wire::WireError::Trailing);
    out.put_resource(Box::new(43u32));
    assert_eq!(out.error(), Some(wire::WireError::Trailing));
    out.clear();
    assert_eq!(out.error(), None);
}
