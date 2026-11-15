#[cfg(test)]
mod tests {
    use mod_api::wire::*;
    use mod_api::wire::{Id, ResourceCodec, ResourceScope, Slots};
    use mod_api::*;
    use std::sync::Arc;
    use std::{
        any::{Any, TypeId},
        cell::RefCell,
        rc::Rc,
        sync::atomic::{AtomicU32, Ordering},
    };

    #[derive(Default)]
    struct Codec(RefCell<Slots<Box<dyn Any>>>);
    impl ResourceCodec for Codec {
        fn store(&self, value: Box<dyn Any>) -> Result<Id, WireError> {
            Ok(self.0.borrow_mut().insert(value))
        }
        fn load(&self, id: Id, _: TypeId) -> Result<Box<dyn Any>, WireError> {
            self.0
                .borrow_mut()
                .remove(id)
                .map_err(|_| WireError::Invalid("stale handler"))
        }
    }

    #[test]
    fn boxed_transfer_and_shared_transfer_preserve_callable_state() {
        let calls = Arc::new(AtomicU32::new(0));
        let state = calls.clone();
        let owned = <dyn ReloadApply>::new(move || {
            state.fetch_add(7, Ordering::SeqCst);
        });
        let _scope = ResourceScope::new(Rc::new(Codec::default()));
        let mut out = Writer::new();
        out.put(&owned);
        assert_eq!(
            out.error(),
            Some(WireError::Invalid("boxed handler needs owned encoding"))
        );
        assert!(out.bytes().is_empty());
        out.clear();
        out.put_owned(owned);
        assert_eq!(out.error(), None);
        assert_eq!(out.bytes(), &[0, 0, 0, 0, 1, 0, 0, 0]);
        let shared: Arc<dyn ReloadApply> = Reader::decode(out.bytes()).unwrap();
        out.clear();
        out.put(&shared);
        assert_eq!(out.error(), None);
        let restored: Arc<dyn ReloadApply> = Reader::decode(out.bytes()).unwrap();
        assert!(Arc::ptr_eq(&shared, &restored));
        ReloadApply::apply(restored.as_ref());
        assert_eq!(calls.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn erased_handler_keeps_identity_and_refuses_expired_resource_scope() {
        let value: Arc<dyn Any + Send + Sync> = Arc::new(73u32);
        let mut out = Writer::new();
        {
            let _scope = ResourceScope::new(Rc::new(Codec::default()));
            out.put(&value);
            let restored: Arc<dyn Any + Send + Sync> = Reader::decode(out.bytes()).unwrap();
            assert!(Arc::ptr_eq(&value, &restored));
            assert_eq!(restored.downcast_ref::<u32>(), Some(&73));
        }
        assert!(matches!(
            Reader::decode::<Arc<dyn Any + Send + Sync>>(out.bytes()),
            Err(WireError::Invalid("no resource context"))
        ));
    }
}
