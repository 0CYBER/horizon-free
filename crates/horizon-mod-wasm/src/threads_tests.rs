//! Wasm threads are compiled into Wasmtime for hosts that embed their own engine; the engine every mod runs on has
//! them off. The first test is in the gate and spawns nothing; the second needs threads and is a daily test.
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use wasmtime::{Config, Engine, Linker, MemoryType, Module, SharedMemory, Store};

const STORE_AFTER: Duration = Duration::from_millis(50);
const BACKSTOP: Duration = Duration::from_secs(10);

const POLLS_HOST_FLAG: &str = r#"
(module
  (import "host" "flag" (memory 1 1 shared))
  (func (export "spin") (result i32)
    (local $n i32)
    (loop $again
      (local.set $n (i32.add (local.get $n) (i32.const 1)))
      (br_if $again (i32.eqz (i32.atomic.load (i32.const 0)))))
    (local.get $n)))
"#;

const DECLARES_SHARED: &str = r#"(module (memory 1 1 shared))"#;

const IMPORTS_SHARED: &str = r#"(module (import "env" "memory" (memory 1 1 shared)))"#;

const USES_ATOMICS: &str = r#"
(module
  (memory 1 1)
  (func (export "f") (result i32) (i32.atomic.load (i32.const 0))))
"#;

fn engine_with_threads() -> Engine {
    let mut config = Config::new();
    config
        .wasm_threads(true)
        .shared_memory(true)
        .epoch_interruption(true);
    let engine = Engine::new(&config).expect("an engine with wasm threads and shared memory");
    assert!(
        !engine.is_pulley(),
        "this host compiles to Pulley, which has no shared memory; run on a Cranelift host"
    );
    engine
}

fn chain(e: &wasmtime::Error) -> String {
    format!("{e:?}")
}

#[test]
#[ignore = "daily"]
fn guest_polling_a_host_shared_flag_returns_once_the_host_sets_it() {
    let engine = engine_with_threads();
    let flag = SharedMemory::new(&engine, MemoryType::shared(1, 1)).expect("a host shared memory");

    let mut store = Store::new(&engine, ());
    store.set_epoch_deadline(1);
    let mut linker = Linker::new(&engine);
    linker
        .define(&store, "host", "flag", flag.clone())
        .expect("the shared flag is defined for the guest");
    let wasm = wat::parse_str(POLLS_HOST_FLAG).expect("the polling module parses");
    let module = Module::new(&engine, &wasm).expect("the polling module compiles");
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("the polling module instantiates with the host flag");
    let spin = instance
        .get_typed_func::<(), i32>(&mut store, "spin")
        .expect("the module exports spin");

    let backstop = engine.weak();
    std::thread::spawn(move || {
        std::thread::sleep(BACKSTOP);
        if let Some(engine) = backstop.upgrade() {
            engine.increment_epoch();
        }
    });
    let setter_flag = flag.clone();
    let setter = std::thread::spawn(move || {
        std::thread::sleep(STORE_AFTER);
        let word = setter_flag.data()[0].get().cast::<u32>();
        let word = unsafe { AtomicU32::from_ptr(word) };
        word.store(1, Ordering::SeqCst);
    });

    spin.call(&mut store, ())
        .expect("the guest returns once the flag is set, not by the epoch backstop");
    setter.join().expect("the setter thread finishes");
}

#[test]
fn the_mod_engine_refuses_guest_shared_memory_and_atomics() {
    let engine = Engine::new(&crate::runtime::engine_config()).unwrap();

    for (what, text) in [("declares", DECLARES_SHARED), ("imports", IMPORTS_SHARED)] {
        let wasm = wat::parse_str(text).expect("the module parses");
        let err = Module::new(&engine, &wasm)
            .err()
            .unwrap_or_else(|| panic!("a module that {what} a shared memory was accepted"));
        let text = chain(&err);
        assert!(
            text.contains("threads must be enabled for shared memories"),
            "a module that {what} a shared memory was refused for another reason: {text}"
        );
    }

    let wasm = wat::parse_str(USES_ATOMICS).expect("the module parses");
    let err = Module::new(&engine, &wasm).expect_err("a module using an atomic load was accepted");
    let text = chain(&err);
    assert!(
        text.contains("threads support is not enabled"),
        "a module using an atomic load was refused for another reason: {text}"
    );

    let err = SharedMemory::new(&engine, MemoryType::shared(1, 1))
        .expect_err("the mod engine created a shared memory");
    let text = chain(&err);
    assert!(
        text.contains("shared memory support is disabled for this engine"),
        "the host shared memory was refused for another reason: {text}"
    );
}
