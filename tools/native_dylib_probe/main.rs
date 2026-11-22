//! Dev-only trusted exact-build Rust ABI probe, not a production loader.
use std::sync::atomic::Ordering;
fn main() {
    // Ensure the actual game (including its global allocator) is a linked dependency.
    std::hint::black_box(horizon_mod_host::main_with as *const ());
    let libraries: Vec<_> = std::env::args_os()
        .skip(1)
        .map(|path| {
            // SAFETY: harness verifies exact SDK/compiler artifacts before starting us.
            // Retain handles until process exit; no unloading Rust implementations.
            Box::leak(Box::new(unsafe { libloading::Library::new(path).unwrap() }))
        })
        .collect();
    assert_eq!(libraries.len(), 2);
    for (index, library) in libraries.iter().enumerate() {
        // SAFETY: these signatures are compiled from the adjacent fixtures with this compiler.
        unsafe {
            let identity = library.get::<fn() -> usize>(b"probe_identity").unwrap();
            assert_eq!(identity(), probe_shared::identity());
            let bump = library.get::<fn() -> usize>(b"probe_bump").unwrap();
            assert_eq!(bump(), index);
            let exchange = library
                .get::<fn(Vec<u8>) -> Vec<u8>>(b"probe_exchange")
                .unwrap();
            assert_eq!(exchange(vec![1, 2, 3]), [4, 5, 6]);
            assert!(
                mod_api::content::crash::details()
                    .iter()
                    .any(|(key, value)| *key == "linkage probe" && value == "shared dependency")
            );
            let tls = *library.get::<fn(Option<&str>)>(b"probe_tls").unwrap();
            tls(None);
            let guard =
                mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "probe",
                    hook: "test",
                    part: None,
                });
            tls(Some("probe"));
            // Global registry shared across threads, but native call attribution is TLS.
            std::thread::spawn(move || {
                assert_eq!(mod_api::content::crash::current_native_mod(), None);
                tls(None);
            })
            .join()
            .unwrap();
            drop(guard);
            tls(None);
            let enter = library
                .get::<fn() -> mod_api::content::crash::ModGuard>(b"probe_enter")
                .unwrap();
            let from_mod = enter();
            assert_eq!(mod_api::content::crash::current_native_mod(), Some("mod"));
            drop(from_mod);
            assert_eq!(mod_api::content::crash::current_native_mod(), None);
        }
    }
    assert_eq!(probe_shared::STATE.load(Ordering::SeqCst), 2);
    println!(
        "PASS: two independently loaded mods share game API, TLS and dependency state; cross-image allocation/drop completed"
    );
}
