use super::*;

/// The runtime-owned operations must not reach the generated API dispatcher.
///
/// # Safety
/// `pointer` must outlive every synchronous call through the returned transport.
pub(super) unsafe fn transport(pointer: *mut boundary::Cx) -> guest::Transport {
    Rc::new(move |id, bytes| {
        if id == mod_api::wire::HOST_LOG {
            eprintln!("{}", Reader::decode::<String>(bytes).unwrap());
            return Vec::new();
        }
        if id == mod_api::wire::HOST_RELEASE {
            // Decode checks the entire payload, as the real runtime does.
            let held: Id = Reader::decode(bytes).unwrap();
            // SAFETY: the caller lends the live context to this synchronous transport.
            let released = unsafe { (&mut *pointer).held.remove(held) }.unwrap();
            drop(released);
            return Vec::new();
        }
        let mut out = Writer::new();
        // SAFETY: the caller keeps the context live until dispatch returns.
        let _resources = unsafe { boundary::resource_scope(pointer, false) };
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        out.into_bytes()
    })
}
