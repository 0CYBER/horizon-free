use super::*;

use crate::test_log::{CAPTURE, capture_logs};

#[test]
fn released_queue_empty_fast_path_does_not_wait_for_its_mutex() {
    let releases = Arc::new(Releases::<Id>::default());
    let guard = releases.entries.lock().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = {
        let releases = releases.clone();
        std::thread::spawn(move || send.send(releases.take()).unwrap())
    };
    let answer = receive.recv_timeout(Duration::from_secs(2));
    drop(guard);
    worker.join().unwrap();
    assert!(answer.unwrap().is_empty());
}

#[test]
fn released_queue_concurrent_pushes_are_delivered_once() {
    let releases = Arc::new(Releases::default());
    let (send, receive) = std::sync::mpsc::channel();
    let workers: Vec<_> = (0..4u64)
        .map(|producer| {
            let releases = releases.clone();
            let send = send.clone();
            std::thread::spawn(move || {
                for batch in 0..32u64 {
                    for item in 0..8u64 {
                        releases.push(Id(producer * 256 + batch * 8 + item));
                    }
                    send.send(()).unwrap();
                }
            })
        })
        .collect();
    drop(send);
    let mut observed = Vec::new();
    for () in receive {
        observed.extend(releases.take().into_iter().map(|id| id.0));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    observed.extend(releases.take().into_iter().map(|id| id.0));
    observed.sort_unstable();
    assert_eq!(observed, (0..1024u64).collect::<Vec<_>>());
    assert!(releases.take().is_empty());
}

struct Random<F>(F);
impl<F: FnMut() -> i64> mod_api::StructureRandom for Random<F> {
    fn next_int_bound(&mut self, _: i32) -> i32 {
        0
    }
    fn next_float(&mut self) -> f32 {
        0.0
    }
    fn next_double(&mut self) -> f64 {
        0.0
    }
    fn next_boolean(&mut self) -> bool {
        false
    }
    fn next_long(&mut self) -> i64 {
        (self.0)()
    }
}

fn guest(layout: u64, body: &str) -> Vec<u8> {
    guest_with_exports(layout, body, "")
}
fn guest_with_exports(layout: u64, body: &str, exports: &str) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
      (@custom "dylink.0" "\01\04\00\00\00\00")
      (import "env" "memory" (memory 1))
      (import "env" "__indirect_function_table" (table 1 funcref))
      (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
      (import "horizon" "host_reply" (func $reply (param i32)))
      (func (export "horizon_layout") (result i64) i64.const {layout})
      (func $recurse (param $depth i32) (result i32)
        local.get $depth i32.eqz
        if (result i32) i32.const 0
        else local.get $depth i32.const 1 i32.sub call $recurse i32.const 1 i32.add end)
      (global $in_len (mut i32) (i32.const 0))
      (func (export "horizon_in") (param i32) (result i32) local.get 0 global.set $in_len i32.const 512)
      {exports}
      (func $hook (export "horizon_hook") (param i32) (result i64) (local i32 i32)
        i32.const 512 local.set 1 global.get $in_len local.set 2 {body})
    )"#
    ))
    .unwrap()
}
fn module(body: &str) -> WasmMod {
    let engine = engine().unwrap();
    let bytes = guest(*mod_api::wire::LAYOUT, body);
    WasmMod::instantiate(
        &engine,
        DEFAULT_GROUP_MEMORY_MIB,
        &[Member {
            name: "test",
            module: &bytes,
        }],
        None,
    )
    .unwrap()
    .remove(0)
}
fn call(module: &WasmMod) -> Option<u32> {
    call_at(module, 0)
}
fn call_at(module: &WasmMod, id: u32) -> Option<u32> {
    module.call(
        CallClass::Hook,
        Frame::default(),
        id,
        None,
        |w| w.put(&42u32),
        |r| r.take(),
    )
}
#[test]
fn hook_round_trip_and_malformed_reply_refusal() {
    crate::test_stack::on_guest_stack(|| {
        // Echo the four-byte input from horizon_in.
        let module = module("i64.const 2199023255556"); // 512 << 32 | 4
        assert_eq!(call(&module), Some(42));
        assert!(!module.running.poisoned.load(Ordering::Relaxed));
        let module = super::runtime_tests::module("i64.const -4294967292"); // ptr = u32::MAX
        assert_eq!(call(&module), None);
        assert!(!module.running.poisoned.load(Ordering::Relaxed));
        assert_eq!(call(&module), None);
    })
}
/// The reply a hook names is read only where it lies inside guest memory.
#[test]
fn a_reply_reaching_past_guest_memory_is_refused_and_one_ending_at_its_edge_is_read() {
    crate::test_stack::on_guest_stack(|| {
        fn invoke(reply: impl Fn(u64) -> u64) -> (u64, Result<usize, String>) {
            let size = {
                let module = module("i64.const 0");
                let group = module.running.group.lock().unwrap();
                group.store.data().memory.unwrap().data_size(&group.store) as u64
            };
            let module = module(&format!("i64.const {}", reply(size) as i64));
            let mut group = module.running.group.lock().unwrap();
            group.store.set_epoch_deadline(30_000);
            let mut input = Writer::new();
            input.put(&42u32);
            let mut output = Vec::new();
            let result = module
                .invoke(&mut group.store.as_context_mut(), 0, &input, &mut output)
                .map(|()| output.len())
                .map_err(|error| error.to_string());
            (size, result)
        }
        // The reply starts at 512 and ends exactly at the end of memory.
        let (size, result) = invoke(|size| 512 << 32 | (size - 512));
        assert_eq!(result, Ok(size as usize - 512));
        // One byte longer reaches past the end.
        let (_, result) = invoke(|size| 512 << 32 | (size - 511));
        assert_eq!(result, Err("reply outside memory".into()));
        // An error reply (length u32::MAX) that starts past the end.
        let (_, result) = invoke(|size| (size + 1) << 32 | u64::from(u32::MAX));
        assert_eq!(result, Err("error reply outside memory".into()));
    })
}
#[test]
fn encoding_failure_refuses_partial_arguments() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 2199023255556");
        assert_eq!(
            module.call(
                CallClass::Hook,
                Frame::default(),
                0,
                None,
                |w| {
                    w.put(&42u32);
                    w.fail(WireError::Invalid("resource refused"));
                },
                |r| r.take::<u32>()
            ),
            None
        );
        assert!(!module.is_stopped());
    })
}

#[test]
fn nested_handler_decodes_in_scope_and_remains_callable_after_return() {
    crate::test_stack::on_guest_stack(|| {
        let bounds = crate::boundary::export_id("StructurePiece", "bounding_box").unwrap();
        let random_id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        for handler in [123, 999] {
            let module = module(&format!(
                r#"
        local.get 0 i32.const {bounds} i32.eq if (result i64)
          local.get 1 i64.load i64.const 123 i64.ne if unreachable end
          i32.const 1024 i32.const 1 i32.store
          i32.const 1028 i32.const 2 i32.store
          i32.const 1032 i32.const 3 i32.store
          i32.const 1036 i32.const 4 i32.store
          i32.const 1040 i32.const 5 i32.store
          i32.const 1044 i32.const 6 i32.store
          i64.const 4398046511128
        else local.get 0 i32.const 1 i32.eq if (result i64)
          i32.const {random_id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host
          i32.const 8 i32.ne if unreachable end
          i64.const 8796093022216
        else
          i64.const 2199023255552 local.get 2 i64.extend_i32_u i64.or
        end end"#
            ));
            let found: mod_api::FoundStructure = module
                .call(
                    CallClass::Hook,
                    Frame::default(),
                    0,
                    None,
                    |w| {
                        w.put(&[9i32, 10, 11]);
                        w.put(&vec![Id(handler)]);
                    },
                    |r| r.take(),
                )
                .unwrap();
            assert_eq!(found.position, [9, 10, 11]);
            if handler == 123 {
                let bounds = found.pieces[0].bounding_box();
                assert_eq!(bounds.min, [1, 2, 3]);
                assert_eq!(bounds.max, [4, 5, 6]);
            }
            let mut random = Random(|| i64::from(found.pieces[0].bounding_box().max[2]));
            let mut frame = Frame::default();
            frame.set_structure_random_mut(&mut random);
            let answer = module.call(CallClass::Load, frame, 1, None, |_| {}, |r| r.take::<i64>());
            if handler == 123 {
                assert_eq!(answer, Some(6));
            }
            assert_eq!(module.is_stopped(), handler != 123);
        }
    })
}

#[test]
fn imports_inherit_the_root_borrow_token_and_expire_with_the_call() {
    crate::test_stack::on_guest_stack(|| {
        let id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let module = module(&format!(
            "i32.const {id} i32.const 0 i32.const 0 i32.const 1024 i32.const 8 call $host drop i32.const {id} i32.const 0 i32.const 0 i32.const 1024 i32.const 8 call $host drop i64.const 4398046511112"
        ));
        let saved = Cell::new(None::<mod_api::wire::ScopeToken>);
        let calls = Cell::new(0);
        let mut random = Random(|| {
            calls.set(calls.get() + 1);
            if let Some(token) = saved.get() {
                token.check().unwrap();
            } else {
                saved.set(Some(mod_api::wire::borrow_token().unwrap()));
            }
            7
        });
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        assert_eq!(
            module.call(CallClass::Hook, frame, 0, None, |_| {}, |r| r.take::<i64>()),
            Some(7)
        );
        assert_eq!(calls.get(), 2);
        assert!(matches!(
            saved.get().unwrap().check(),
            Err(mod_api::wire::ScopeError::Stale)
        ));
    })
}

#[test]
fn guest_panic_text_is_logged_with_mod_id_and_no_reply() {
    crate::test_stack::on_guest_stack(|| {
        capture_logs();
        for trailing in [false, true] {
            let module = module(&format!(
                "i32.const {} local.get 1 local.get 2 i32.const -1 i32.const 0 call $host i32.const 0 i32.ne if unreachable end i64.const 0",
                mod_api::wire::HOST_LOG
            ));
            let answer = module.call(
                CallClass::Hook,
                Frame::default(),
                0,
                None,
                |w| {
                    w.put(&"guest panic at register");
                    if trailing {
                        w.put(&1u8);
                    }
                },
                |r| r.take::<()>(),
            );
            assert_eq!(answer, if trailing { None } else { Some(()) });
            assert_eq!(module.is_stopped(), trailing);
        }
        let messages = CAPTURE.0.lock().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.as_str() == "wasm mod test: guest panic at register")
                .count(),
            1
        );
    })
}

#[test]
fn calls_do_not_leave_an_active_module_ownership_cycle() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 2199023255556");
        let running = Arc::downgrade(&module.running);
        assert_eq!(call(&module), Some(42));
        drop(module);
        assert!(running.upgrade().is_none());
    })
}

#[test]
fn trap_and_deadline_stop_the_group() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("unreachable");
        assert_eq!(call(&module), None);
        assert!(module.running.poisoned.load(Ordering::Relaxed));
        let module = super::runtime_tests::module("(loop br 0) unreachable");
        let started = Instant::now();
        assert_eq!(call(&module), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(module.running.poisoned.load(Ordering::Relaxed));
    })
}
#[test]
fn a_member_cannot_bypass_group_limits_with_private_memories() {
    crate::test_stack::on_guest_stack(|| {
        let bytes =
            wat::parse_str(r#"(module (@custom "dylink.0" "\01\04\00\00\00\00") (memory 1))"#)
                .unwrap();
        let refusal = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "private",
                module: &bytes,
            }],
            None,
        )
        .err()
        .unwrap();
        assert!(refusal.to_string().contains("must import the group memory"));
    })
}

#[test]
#[ignore = "requires shared std and the real noop fixture"]
fn guest_input_pool_stays_flat_for_100k_no_import_calls() {
    crate::test_stack::on_guest_stack(|| {
        let std = std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap();
        let fixture = horizon_mod_package::manifest::Package::read_wasm(std::path::Path::new(
            &std::env::var("HORIZON_WASM_FIXTURE").unwrap(),
        ))
        .unwrap();
        let module = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[
                Member {
                    name: "std",
                    module: &std,
                },
                Member {
                    name: "fixture",
                    module: &fixture,
                },
            ],
            None,
        )
        .unwrap()
        .pop()
        .unwrap();
        let subscriptions: Vec<(String, i32, bool)> = module
            .call(
                CallClass::Load,
                Frame::default(),
                crate::boundary::SUBSCRIBED,
                None,
                |_| {},
                |r| r.take(),
            )
            .unwrap();
        assert!(
            subscriptions
                .iter()
                .any(|(hook, _, _)| hook == "server_stopped")
        );
        let id = crate::boundary::export_id("Hooks", "server_stopped").unwrap();
        let mut group = module.running.group.lock().unwrap();
        group.store.set_epoch_deadline(30_000);
        fn step(module: &WasmMod, store: &mut Store<State>, id: u32) {
            let memory = store.data().memory.unwrap();
            let input = module.input.call(&mut *store, 1).unwrap();
            memory.write(&mut *store, input as usize, &[0]).unwrap();
            let reply = module.hook.call(&mut *store, id).unwrap();
            assert_eq!(reply as u32, 1);
            let mut bytes = [0xff];
            memory
                .read(&*store, (reply >> 32) as usize, &mut bytes)
                .unwrap();
            assert_eq!(bytes, [0]);
        }
        for _ in 0..1000 {
            step(&module, &mut group.store, id);
        }
        let memory = group.store.data().memory.unwrap();
        let before = memory.size(&group.store);
        for _ in 0..100_000 {
            step(&module, &mut group.store, id);
        }
        let after = memory.size(&group.store);
        assert_eq!(
            after, before,
            "warmed no-import calls must not retain inputs"
        );
    })
}

#[test]
fn real_members_link_when_built() {
    crate::test_stack::on_guest_stack(|| {
        let Some(dir) = std::env::var_os("HORIZON_WASM_TEST_MEMBERS") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let (Ok(std), Ok(fixture)) = (
            std::fs::read(dir.join("std.wasm")),
            std::fs::read(dir.join("fixture.wasm")),
        ) else {
            return;
        };
        let engine = engine().unwrap();
        let modules = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[
                Member {
                    name: "std",
                    module: &std,
                },
                Member {
                    name: "fixture",
                    module: &fixture,
                },
            ],
            None,
        );
        let modules =
            modules.unwrap_or_else(|why| panic!("real shared members did not load: {why}"));
        assert_eq!(modules.len(), 1);
        let subscriptions: Vec<(String, i32, bool)> = modules[0]
            .call(
                CallClass::Load,
                Frame::default(),
                crate::boundary::SUBSCRIBED,
                None,
                |_| {},
                |r| r.take(),
            )
            .expect("real guest subscription hook failed");
        assert!(
            subscriptions
                .iter()
                .all(|(name, _, _)| crate::boundary::hook_id(name).is_some())
        );
        assert!(subscriptions.contains(&("server_stopped".into(), 37, false)));
        assert!(subscriptions.contains(&("render_gui_pre".into(), -12, true)));
        // A cancelled event enters the real guest; its receive-cancelled listener clears it.
        let cancelled: mod_api::RenderGuiPreEvent = Reader::new(&[1]).take().unwrap();
        assert!(cancelled.cancelled);
        let pre = crate::boundary::export_id("Hooks", "render_gui_pre").unwrap();
        let cleared = modules[0]
            .call(
                CallClass::Hook,
                Frame::default(),
                pre,
                None,
                |w| w.put(&cancelled),
                |r| {
                    assert_eq!(r.take::<mod_api::Flow>()?, mod_api::Flow::Continue);
                    r.take::<mod_api::RenderGuiPreEvent>()
                },
            )
            .expect("real guest cancellation writeback failed");
        assert!(!cleared.cancelled);
        let event: mod_api::ClientTickPostEvent = Reader::new(&[]).take().unwrap();
        let tick = crate::boundary::export_id("Hooks", "client_tick_post").unwrap();
        assert_eq!(
            modules[0].call(
                CallClass::Hook,
                Frame::default(),
                tick,
                None,
                |w| w.put(&event),
                |r| {
                    r.take::<()>()?;
                    r.take::<mod_api::ClientTickPostEvent>()?;
                    Ok(())
                }
            ),
            Some(())
        );
    })
}

#[test]
fn unresolved_namespaced_functions_are_not_stubbed() {
    crate::test_stack::on_guest_stack(|| {
        let bytes = wat::parse_str(r#"(module (@custom "dylink.0" "\01\04\00\00\00\00") (import "env" "fixture:missing" (func)))"#).unwrap();
        let error = Group::link(
            &crate::runtime::test_engine(),
            &[Member {
                name: "test",
                module: &bytes,
            }],
        )
        .err()
        .unwrap();
        assert!(
            matches!(error, crate::link::LinkError::Unresolved { symbol, .. } if symbol == "env.fixture:missing")
        );
    })
}

#[test]
fn linker_heap_symbols_follow_all_members_static_data() {
    crate::test_stack::on_guest_stack(|| {
        let first =
            wat::parse_str(r#"(module (@custom "dylink.0" "\01\04\10\00\00\00"))"#).unwrap();
        let second = wat::parse_str(
            r#"(module (@custom "dylink.0" "\01\04\20\00\00\00")
        (import "GOT.mem" "__heap_base" (global $heap (mut i32)))
        (import "GOT.mem" "__data_end" (global $end (mut i32)))
        (import "GOT.mem" "__heap_end" (global $limit (mut i32)))
        (func (export "heap") (result i32) global.get $heap)
        (func (export "data") (result i32) global.get $end)
        (func (export "limit") (result i32) global.get $limit))"#,
        )
        .unwrap();
        let engine = crate::runtime::test_engine();
        let mut group = Group::link(
            &engine,
            &[
                Member {
                    name: "first",
                    module: &first,
                },
                Member {
                    name: "second",
                    module: &second,
                },
            ],
        )
        .unwrap();
        for symbol in ["heap", "data"] {
            let mut output = [wasmtime::Val::I32(0)];
            group.call("second", symbol, &[], &mut output).unwrap();
            assert_eq!(output[0].i32(), Some(262192)); // 256 KiB stack + 16 + 32 bytes.
        }
        let mut output = [wasmtime::Val::I32(0)];
        group.call("second", "limit", &[], &mut output).unwrap();
        assert_eq!(output[0].i32(), Some(327680)); // Five initial memory pages.
    })
}

#[test]
fn boundary_exports_are_per_member_but_other_horizon_symbols_are_not() {
    crate::test_stack::on_guest_stack(|| {
        let bytes = guest_with_exports(
            *mod_api::wire::LAYOUT,
            "i64.const 0",
            r#"
            (func (export "prepare_can_equip") (result i64) i64.const 2199023255560)
            (func (export "ask_can_equip") (result i64) i64.const 0)
            (func (export "release_can_equip") (result i64) i64.const 0)
        "#,
        );
        let engine = engine().unwrap();
        let modules = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[
                Member {
                    name: "first",
                    module: &bytes,
                },
                Member {
                    name: "second",
                    module: &bytes,
                },
            ],
            None,
        )
        .unwrap();
        assert_eq!(modules.len(), 2);
        for i in 0..2 {
            let owner = modules[i].downgrade();
            let other = modules[1 - i].downgrade();
            let token = owner.prepare(&[]).unwrap();
            assert!(owner.check(token.0, token.1));
            assert!(!other.check(token.0, token.1));
            owner.release(token.0, token.1);
        }

        let bytes = wat::parse_str(
        r#"(module (@custom "dylink.0" "\01\04\00\00\00\00") (func (export "horizon_shared")))"#,
    )
    .unwrap();
        let refusal = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[
                Member {
                    name: "first",
                    module: &bytes,
                },
                Member {
                    name: "second",
                    module: &bytes,
                },
            ],
            None,
        )
        .err()
        .unwrap();
        assert!(
            refusal
                .to_string()
                .contains("exports horizon_shared, which first already does")
        );
    })
}

#[test]
fn loader_refuses_poi_registration_before_unsupported_metadata_can_run() {
    crate::test_stack::on_guest_stack(|| {
        let std = wat::parse_str(
        r#"(module (@custom "dylink.0" "\01\04\00\00\00\00") (import "env" "memory" (memory 1)))"#,
    )
    .unwrap();
        let mut subscriptions = Writer::new();
        subscriptions.put(&vec![("extend_poi_types".to_owned(), 0_i32, false)]);
        let bytes = subscriptions.into_bytes();
        let mut body = String::new();
        for (offset, byte) in bytes.iter().enumerate() {
            body.push_str(&format!(
                "i32.const {} i32.const {byte} i32.store8 ",
                2048 + offset
            ));
        }
        body.push_str(&format!(
            "i64.const {}",
            (2048_u64 << 32) | bytes.len() as u64
        ));
        let module = guest(*mod_api::wire::LAYOUT, &body);
        let manifest = horizon_mod_package::manifest::Manifest::parse(&format!("id = 'poi_probe'\nname = 'POI probe'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[wasm]\nsha512 = '{}'\ntoolchain = 'test'\nbuilt_with = 'test'\n", "a".repeat(128))).unwrap();
        let loaded = crate::load(&std, vec![crate::WasmPackage { manifest, module }]);
        assert!(loaded.mods.is_empty());
        assert_eq!(loaded.refused.len(), 1);
        assert!(
            loaded.refused[0]
                .1
                .to_string()
                .contains("PoiTypeExtensions::metadata"),
            "{:?}",
            loaded.refused
        );
    })
}

#[test]
fn loader_initializes_and_installs_a_wasm_entry() {
    crate::test_stack::on_guest_stack(|| {
        use mod_api::__private::game::Fields;
        let std = wat::parse_str(
        r#"(module (@custom "dylink.0" "\01\04\00\00\00\00") (import "env" "memory" (memory 1)))"#,
    )
    .unwrap();
        let module = guest(
            *mod_api::wire::LAYOUT,
            r#"local.get 0 i32.const -256 i32.eq if (result i64)
            i32.const 4096 local.get 2 i32.store
            i32.const 2048 local.get 1 local.get 2 memory.copy
            i64.const 0
          else local.get 0 i32.const 2 i32.eq if (result i64)
            i64.const 8796093022208 i32.const 4096 i32.load i64.extend_i32_u i64.or
          else i64.const 4398046511108 end end"#,
        );
        let manifest = horizon_mod_package::manifest::Manifest::parse(&format!("id = 'roundtrip'\nname = 'Roundtrip'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[wasm]\nsha512 = '{}'\ntoolchain = 'test'\nbuilt_with = 'test'\n", "a".repeat(128))).unwrap();
        let loaded = crate::load(&std, vec![crate::WasmPackage { manifest, module }]);
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        assert_eq!(loaded.mods[0].fields().id, "roundtrip");
        assert_eq!(loaded.manifests[0].id, "roundtrip");
        let inventory: Vec<_> = ["native", "roundtrip", "content"]
            .into_iter()
            .map(|id| {
                mod_api::PackageInfo::new(
                    id.into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec![id.into()],
                    Vec::new(),
                )
            })
            .collect();
        crate::initialize_modules(&loaded.modules, &inventory);
        let echoed: Vec<mod_api::PackageInfo> = loaded.modules[0]
            .call(
                CallClass::Hook,
                Frame::default(),
                2,
                None,
                |_| {},
                |r| r.take(),
            )
            .unwrap();
        assert_eq!(
            echoed.iter().map(|p| p.id()).collect::<Vec<_>>(),
            ["native", "roundtrip", "content"]
        );
    })
}

#[test]
fn invalid_import_input_is_a_trap_not_a_host_panic() {
    crate::test_stack::on_guest_stack(|| {
        let module = module(
            "i32.const -1 i32.const -1 i32.const 16 i32.const 0 i32.const 0 call $host drop i64.const 0",
        );
        assert_eq!(call(&module), None);
        assert!(module.running.poisoned.load(Ordering::Relaxed));
    })
}

#[test]
fn layout_mismatch_is_a_mods_page_refusal() {
    crate::test_stack::on_guest_stack(|| {
        let engine = engine().unwrap();
        let bytes = guest(*mod_api::wire::LAYOUT ^ 1, "i64.const 0");
        let refusal = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "test",
                module: &bytes,
            }],
            None,
        )
        .err()
        .unwrap();
        assert!(matches!(refusal, Refusal::Layout { .. }));
        assert_eq!(
            refusal.to_string(),
            "built for another version of the mod API (including the prepared can_equip ABI)"
        );
    })
}
#[test]
fn point_record_layout_mismatch_is_refused() {
    crate::test_stack::on_guest_stack(|| {
        let bytes = wat::parse_str(format!(
            r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
        (func (export "horizon_layout") (result i64) i64.const {})
        (func (export "horizon_layout_ext") (result i64) i64.const {})
        (func (export "horizon_in") (param i32) (result i32) i32.const 0)
        (func (export "horizon_hook") (param i32) (result i64) i64.const 0))"#,
            *mod_api::wire::LAYOUT,
            mod_points::POINT_LAYOUT ^ 1
        ))
        .unwrap();
        let refusal = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "points",
                module: &bytes,
            }],
            None,
        )
        .err()
        .unwrap();
        assert!(matches!(refusal, Refusal::PointLayout { .. }));
    })
}

#[test]
fn callback_borrows_only_the_lent_context_and_restores_depth() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 2199023255556");
        let mut group = module.running.group.lock().unwrap();
        group.store.set_epoch_deadline(100);
        group.store.data_mut().depth = 1;
        group.store.data_mut().cx.frames.push(Frame::default());
        let mut context = group.store.as_context_mut();
        let lending = context.data().lending.clone();
        lending.fetch_add(1, Ordering::Relaxed);
        let loan = Loan {
            lending,
            group: Arc::as_ptr(&module.running) as usize,
            context: Cell::new((&mut context as *mut StoreContextMut<'_, State>).cast()),
            previous: LENT.get(),
        };
        LENT.set(&loan);
        assert_eq!(
            module.call(
                CallClass::Callback,
                Frame::default(),
                0,
                None,
                |w| w.put(&99u32),
                |r| r.take::<u32>()
            ),
            Some(99)
        );
        drop(loan);
        assert_eq!(context.data().depth, 1);
        assert_eq!(context.data().cx.frames.len(), 1);
    })
}

#[test]
fn import_reply_retry_survives_a_nested_callback() {
    crate::test_stack::on_guest_stack(|| {
        let id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let module = module(&format!(
            r#"
        local.get 0 i32.const 1 i32.eq
        if (result i64)
          i64.const 2199023255560
        else
          i32.const {id} i32.const 0 i32.const 0 i32.const 0 i32.const 0 call $host
          i32.const 8 i32.ne if unreachable end
          i32.const 0 i64.load i64.const 0 i64.ne if unreachable end
          i32.const 1024 call $reply
          i64.const 4398046511112
        end"#
        ));
        let mut random = Random(|| {
            module
                .call(
                    CallClass::Hook,
                    Frame::default(),
                    1,
                    None,
                    |w| w.put(&70i64),
                    |r| r.take::<i64>(),
                )
                .unwrap_or(-1)
                + 1
        });
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        assert_eq!(
            module.call(CallClass::Hook, frame, 0, None, |_| {}, |r| r.take::<i64>()),
            Some(71)
        );
    })
}

#[test]
fn another_threads_hook_completes_after_an_import_releases_the_group() {
    crate::test_stack::on_guest_stack(|| {
        let id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let module = module(&format!(
            r#"
            local.get 0 i32.eqz
            if (result i64)
                i32.const {id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host
                drop i64.const 8796093022216
            else
                i64.const 2199023255556
            end"#
        ));
        let (started, ready) = std::sync::mpsc::channel();
        let (send, receive) = std::sync::mpsc::channel();
        let mut contender = None;
        let mut early = None;
        let mut random = Random(|| {
            let module = module.clone();
            let started = started.clone();
            let send = send.clone();
            contender = Some(std::thread::spawn(move || {
                crate::test_stack::on_guest_stack(|| {
                    started.send(()).unwrap();
                    send.send(call_at(&module, 1)).unwrap();
                })
            }));
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            early = Some(receive.recv_timeout(Duration::from_millis(100)));
            7
        });
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        assert_eq!(
            module.call(CallClass::Load, frame, 0, None, |_| {}, |r| r.take::<i64>()),
            Some(7)
        );
        contender.unwrap().join().unwrap();
        assert_eq!(
            early.unwrap(),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        assert_eq!(receive.try_recv().unwrap(), Some(42));
        assert!(!module.is_stopped());
    })
}

#[test]
fn contended_calls_wait_for_the_group_and_workers_ignore_lending() {
    crate::test_stack::on_guest_stack(|| {
        for (class, lending) in [(CallClass::Hook, false), (CallClass::Worker, true)] {
            let module = module("i64.const 2199023255556");
            let group = module.running.group.lock().unwrap();
            if lending {
                module.running.lending.store(1, Ordering::Relaxed);
            }
            let (started, ready) = std::sync::mpsc::channel();
            let (send, receive) = std::sync::mpsc::channel();
            let other = module.clone();
            let thread = std::thread::spawn(move || {
                crate::test_stack::on_guest_stack(|| {
                    started.send(()).unwrap();
                    let result = other.call(
                        class,
                        Frame::default(),
                        0,
                        None,
                        |w| w.put(&42u32),
                        |r| r.take::<u32>(),
                    );
                    send.send(result).unwrap();
                })
            });
            ready.recv().unwrap();
            let early = receive.recv_timeout(Duration::from_millis(100));
            let waited = matches!(&early, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
            module.running.lending.store(0, Ordering::Relaxed);
            drop(group);
            let result = match early {
                Ok(value) => value,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    receive.recv_timeout(Duration::from_secs(5)).unwrap()
                }
                Err(error) => panic!("call thread failed: {error}"),
            };
            thread.join().unwrap();
            assert!(
                waited,
                "a contended call returned before the lock was released"
            );
            assert_eq!(result, Some(42));
            assert!(!module.is_stopped());
        }
    })
}

#[test]
#[ignore = "daily"]
fn contended_hook_times_out_without_poisoning_the_group() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 2199023255556");
        let group = module.running.group.lock().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let other = module.clone();
        let thread = std::thread::spawn(move || {
            crate::test_stack::on_guest_stack(|| {
                let start = Instant::now();
                let result = call(&other);
                send.send((result, start.elapsed())).unwrap();
            })
        });
        let result = receive.recv_timeout(Duration::from_secs(10));
        drop(group);
        thread.join().unwrap();
        let (answer, elapsed) = result.expect("contended hook did not time out");
        assert_eq!(answer, None);
        assert!(elapsed >= Duration::from_secs(5));
        assert!(!module.is_stopped());
        assert_eq!(call(&module), Some(42));
    })
}

#[test]
fn nested_load_calls_do_not_extend_the_outer_deadline() {
    crate::test_stack::on_guest_stack(|| {
        let id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let module = module(&format!(
            "local.get 0 i32.const 1 i32.eq if (result i64) i64.const 0 else i32.const {id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host drop i64.const 8796093022216 end"
        ));
        let epoch = module.running.group.lock().unwrap().store.engine().clone();
        let entered = Cell::new(false);
        let mut random = Random(|| {
            entered.set(true);
            for _ in 0..1000 {
                epoch.increment_epoch();
            }
            assert_eq!(
                module.call(
                    CallClass::Load,
                    Frame::default(),
                    1,
                    None,
                    |_| {},
                    |r| r.take::<()>()
                ),
                None
            );
            7
        });
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        let _ = module.call(CallClass::Hook, frame, 0, None, |_| {}, |r| r.take::<i64>());
        assert!(entered.get());
        assert!(module.is_stopped());
    })
}

#[test]
fn dropping_a_handler_releases_its_id_on_the_next_call() {
    crate::test_stack::on_guest_stack(|| {
        let module = module(
            r#"
        local.get 0 i32.const -255 i32.eq
        if (result i64)
          local.get 1 i32.load i32.const 1 i32.ne if unreachable end
          local.get 1 i64.load offset=4 i64.const 123 i64.ne if unreachable end
          i32.const 1024 i32.const 1 i32.store
          i64.const 0
        else
          i64.const 4398046511108
        end"#,
        );
        assert_eq!(call(&module), Some(0));
        drop(WasmHandler {
            module: module.downgrade(),
            index: Id(123),
        });
        assert_eq!(call(&module), Some(1));
        assert!(!module.running.poisoned.load(Ordering::Relaxed));
    })
}

struct StopProbe {
    module: WeakWasmMod,
    dropped: Arc<AtomicBool>,
}
impl Drop for StopProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.module.call(
                CallClass::Callback,
                Frame::default(),
                0,
                None,
                |_| {},
                |r| r.take::<u32>()
            ),
            None
        );
        self.dropped.store(true, Ordering::Relaxed);
    }
}
fn hold_stop_probe(module: &WasmMod) -> Arc<AtomicBool> {
    let dropped = Arc::new(AtomicBool::new(false));
    module
        .running
        .group
        .lock()
        .unwrap()
        .store
        .data_mut()
        .cx
        .held
        .insert_long(Box::new(StopProbe {
            module: module.downgrade(),
            dropped: dropped.clone(),
        }));
    dropped
}

#[test]
fn retained_proxy_does_not_own_running_and_unload_drops_native_resources() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 4398046511108");
        let proxy = WasmHandler::new(&module, Id(123));
        let running = Arc::downgrade(&module.running);
        let dropped = hold_stop_probe(&module);
        drop(module);
        assert_eq!(running.strong_count(), 0);
        assert!(dropped.load(Ordering::Relaxed));
        assert_eq!(
            proxy.module.call(
                CallClass::Callback,
                Frame::default(),
                0,
                None,
                |_| {},
                |r| r.take::<u32>()
            ),
            None
        );
    })
}

#[test]
fn disable_clears_held_resources_before_retained_proxy_can_call() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 4398046511108");
        let proxy = WasmHandler::new(&module, Id(123));
        let dropped = hold_stop_probe(&module);
        module.disable();
        assert!(dropped.load(Ordering::Relaxed));
        assert_eq!(
            proxy.module.call(
                CallClass::Callback,
                Frame::default(),
                0,
                None,
                |_| {},
                |r| r.take::<u32>()
            ),
            None
        );
    })
}

#[test]
fn disable_during_native_import_defers_resource_drop_until_call_returns() {
    crate::test_stack::on_guest_stack(|| {
        let id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let module = module(&format!(
            "i32.const {id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host drop i64.const 8796093022216"
        ));
        let dropped = hold_stop_probe(&module);
        let mut random = Random(|| {
            module.disable();
            assert!(!dropped.load(Ordering::Relaxed));
            7
        });
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        assert_eq!(
            module.call(CallClass::Hook, frame, 0, None, |_| {}, |r| r.take::<i64>()),
            Some(7)
        );
        assert!(dropped.load(Ordering::Relaxed));
    })
}

#[test]
fn calling_module_borrow_is_cleared_after_success_and_trap() {
    crate::test_stack::on_guest_stack(|| {
        for (body, expected) in [("i64.const 0", Some(())), ("unreachable", None)] {
            let module = module(body);
            assert_eq!(
                module.call(
                    CallClass::Hook,
                    Frame::default(),
                    0,
                    None,
                    |_| {},
                    |r| r.take::<()>()
                ),
                expected
            );
            let group = module.running.group.lock().unwrap();
            assert_eq!(group.store.data().depth, 0);
            assert!(group.store.data().cx.frames.is_empty());
            assert!(group.store.data().cx.module.is_null());
        }
    })
}

#[test]
fn native_panic_stops_group_before_held_resources_drop() {
    crate::test_stack::on_guest_stack(|| {
        let id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let module = module(&format!(
            "i32.const {id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host drop i64.const 8796093022216"
        ));
        let dropped = hold_stop_probe(&module);
        let mut random = Random(|| panic!("native callback"));
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            module.call(CallClass::Hook, frame, 0, None, |_| {}, |r| r.take::<i64>())
        }));
        assert!(result.is_err());
        assert!(dropped.load(Ordering::Relaxed));
        assert_eq!(call(&module), None);
        let group = module
            .running
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(group.store.data().depth, 0);
        assert!(group.store.data().cx.frames.is_empty());
        assert!(group.store.data().cx.module.is_null());
    })
}

#[test]
fn idle_proxy_called_during_unwind_stops_and_clears_resources() {
    crate::test_stack::on_guest_stack(|| {
        struct OnUnwind(WeakWasmMod);
        impl Drop for OnUnwind {
            fn drop(&mut self) {
                assert_eq!(
                    self.0.call(
                        CallClass::Callback,
                        Frame::default(),
                        0,
                        None,
                        |_| {},
                        |r| r.take::<u32>()
                    ),
                    None
                );
            }
        }
        let module = module("i64.const 4398046511108");
        let dropped = hold_stop_probe(&module);
        let weak = module.downgrade();
        let result = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                let _callback = OnUnwind(weak);
                panic!("unrelated native panic");
            })
            .unwrap()
            .join();
        assert!(result.is_err());
        assert!(module.is_stopped());
        assert!(dropped.load(Ordering::Relaxed));
    })
}

#[test]
fn released_dispatch_receiver_cannot_reenter_guest_while_unwinding() {
    crate::test_stack::on_guest_stack(|| {
        struct Piece {
            module: WeakWasmMod,
            refused: Arc<AtomicBool>,
        }
        impl std::fmt::Debug for Piece {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("Piece")
            }
        }
        impl Drop for Piece {
            fn drop(&mut self) {
                let result = self.module.call(
                    CallClass::Callback,
                    Frame::default(),
                    2,
                    None,
                    |_| {},
                    |r| r.take::<u32>(),
                );
                self.refused.store(result.is_none(), Ordering::Relaxed);
            }
        }
        impl mod_api::EnergyHandler for Piece {
            fn amount(&self, _: &mut dyn mod_api::ServerHost) -> u32 {
                assert_eq!(
                    self.module.call(
                        CallClass::Callback,
                        Frame::default(),
                        1,
                        None,
                        |_| {},
                        |r| r.take::<()>()
                    ),
                    Some(())
                );
                panic!("last dispatch receiver unwinds");
            }
            fn capacity(&self, _: &mut dyn mod_api::ServerHost) -> u32 {
                0
            }
            fn insert(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::Transaction,
                _: u32,
            ) -> u32 {
                0
            }
            fn extract(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::Transaction,
                _: u32,
            ) -> u32 {
                0
            }
        }
        let bounds = crate::boundary::import_id("EnergyHandler", "amount").unwrap();
        let release = mod_api::wire::HOST_RELEASE;
        let module = module(&format!(
            r#"
        local.get 0 i32.eqz if (result i64)
          i32.const 0 local.get 1 i64.load i64.store
          i32.const {bounds} i32.const 0 i32.const 8 i32.const 2048 i32.const 24 call $host drop
          i64.const 0
        else local.get 0 i32.const 1 i32.eq if (result i64)
          i32.const {release} i32.const 0 i32.const 8 i32.const 0 i32.const 0 call $host drop
          i64.const 0
        else
          i64.const 4398046511108
        end end"#
        ));
        let refused = Arc::new(AtomicBool::new(false));
        let piece: Arc<dyn mod_api::EnergyHandler> = Arc::new(Piece {
            module: module.downgrade(),
            refused: refused.clone(),
        });
        let mut host = mod_api::testing::FakeHost::new();
        let mut frame = Frame::default();
        frame.set_server_host_mut(&mut host);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            module.call(
                CallClass::Hook,
                frame,
                0,
                None,
                |w| w.put_owned(piece),
                |r| r.take::<()>(),
            )
        }));
        assert!(result.is_err());
        assert!(refused.load(Ordering::Relaxed));
        assert!(module.is_stopped());
    })
}

#[test]
fn trap_clears_held_native_resources() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("unreachable");
        let dropped = hold_stop_probe(&module);
        assert_eq!(call(&module), None);
        assert!(dropped.load(Ordering::Relaxed));
    })
}

#[test]
fn returned_builtin_proxy_does_not_keep_its_group_alive() {
    crate::test_stack::on_guest_stack(|| {
        use mod_api::EnergyHandler;
        struct World;
        impl mod_api::Host for World {}
        mod_api::impl_host_areas!(World);
        impl mod_api::ServerHost for World {}
        mod_api::impl_server_host_areas!(World);
        let amount = crate::boundary::export_id("EnergyHandler", "amount").unwrap();
        let module = module(&format!(
            r#"
        local.get 0 i32.const {amount} i32.eq if (result i64)
          i32.const 1024 i32.const 17 i32.store
          i64.const 4398046511108
        else
          i64.const 2199023255552 local.get 2 i64.extend_i32_u i64.or
        end"#
        ));
        let proxy: Arc<dyn EnergyHandler> = module
            .call(
                CallClass::Hook,
                Frame::default(),
                0,
                None,
                |w| w.put(&Id(123)),
                |r| r.take(),
            )
            .unwrap();
        assert_eq!(proxy.amount(&mut World), 17);
        // Sending this proxy back makes Cx.held retain it. The retained guest proxy
        // must not create Running -> Cx.held -> WasmHandler -> Running ownership.
        let _: Id = module
            .call(
                CallClass::Hook,
                Frame::default(),
                0,
                None,
                |w| w.put(&proxy),
                |r| r.take(),
            )
            .unwrap();
        let running = Arc::downgrade(&module.running);
        drop(module);
        assert_eq!(
            running.strong_count(),
            0,
            "a retained guest proxy kept its own group alive"
        );
        assert_eq!(
            proxy.amount(&mut World),
            0,
            "an unavailable proxy did not take the call-refused path"
        );
    })
}

#[test]
fn growth_limits_return_failure_without_stopping_the_group() {
    crate::test_stack::on_guest_stack(|| {
        for body in [
            // Past the default 1 GiB (16,384 pages) on top of what the guest already has.
            "i32.const 16384 memory.grow",
            "ref.null func i32.const 1048576 table.grow",
        ] {
            let module = module(&format!(
                "{body} i32.const -1 i32.ne if unreachable end i64.const 2199023255556"
            ));
            let other = module.clone();
            assert_eq!(call(&module), Some(42));
            assert!(!other.is_stopped());
            assert_eq!(call(&other), Some(42));
        }
    })
}

/// A refused growth the guest handled is forgotten when its call ends: a later trap's reason
/// does not blame it.
#[test]
fn a_handled_refused_growth_does_not_explain_a_later_trap() {
    crate::test_stack::on_guest_stack(|| {
        // First call: memory.grow past 1 GiB answers -1, the guest drops it and replies 42.
        // Second call: an unrelated trap.
        let module = module(
            r#"
            i32.const 100 i32.const 100 i32.load i32.const 1 i32.add i32.store
            i32.const 100 i32.load i32.const 1 i32.eq
            if (result i64)
                i32.const 16384 memory.grow i32.const -1 i32.ne if unreachable end
                i64.const 2199023255556
            else unreachable end"#,
        );
        assert_eq!(call(&module), Some(42));
        assert_eq!(call(&module), None);
        assert_eq!(
            take_refusal(),
            "wasm trap: wasm `unreachable` instruction executed"
        );
    })
}

/// Normal guest returns cannot poison shared memory, even with unusable replies.
#[test]
fn refusal_replies_survive_and_report_once() {
    crate::test_stack::on_guest_stack(|| {
        capture_logs();
        for (index, bad) in [
            "i64.const 2203318222847", // error reply: empty string at 512
            "i64.const 2199023255553", // too short for u32
            "i64.const 2199023255557", // trailing byte
            "i32.const 100 i32.load i32.const 1 i32.eq if (result i64) i64.const 2199023255553 else i64.const 2199023255557 end", // changing reasons
        ]
        .into_iter()
        .enumerate()
        {
            let mut module = module(&format!(
                r#"
            i32.const 100 i32.const 100 i32.load i32.const 1 i32.add i32.store
            i32.const 100 i32.load i32.const 2 i32.le_u
            if (result i64)
                i32.const 512 i32.const 0 i32.store
                {bad}
            else i64.const 2199023255556 end"#
            ));
            module.id = format!("refusal_case_{index}").into();
            for _ in 0..2 {
                assert_eq!(call(&module), None);
                assert!(!module.is_stopped());
            }
            assert_eq!(
                module
                    .running
                    .group
                    .lock()
                    .unwrap()
                    .store
                    .data()
                    .refusals
                    .len(),
                1
            );
            assert_eq!(call(&module), Some(42));
            let prefix = format!("wasm refusal: {} hook 0:", module.id);
            assert_eq!(
                CAPTURE
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|m| m.starts_with(&prefix))
                    .count(),
                1
            );
        }
    })
}

/// A depth refusal never enters guest code and is deduplicated by call path.
#[test]
fn depth_refusal_survives() {
    crate::test_stack::on_guest_stack(|| {
        capture_logs();
        let mut module = module("i64.const 2199023255556");
        module.id = "depth_refusal_case".into();
        module.running.group.lock().unwrap().store.data_mut().depth = 64;
        for _ in 0..2 {
            assert_eq!(call(&module), None);
            assert!(!module.is_stopped());
        }
        let mut group = module.running.group.lock().unwrap();
        assert_eq!(group.store.data().refusals.len(), 1);
        assert_eq!(
            CAPTURE
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.starts_with("wasm depth refusal:")
                    && m.contains("depth_refusal_case")
                    && m.contains("limit 64"))
                .count(),
            1
        );
        group.store.data_mut().depth = 0;
        drop(group);
        assert_eq!(call(&module), Some(42));
    })
}

/// A nested call must avoid a lock cycle while the target group lends its store.
#[test]
fn nested_busy_refusal_survives() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 2199023255556");
        let guard = module.running.group.lock().unwrap();
        module.running.lending.store(1, Ordering::Relaxed);
        let loan = Loan {
            group: 0,
            lending: Arc::new(AtomicUsize::new(1)),
            context: Cell::new(std::ptr::null_mut()),
            previous: LENT.get(),
        };
        LENT.set(&loan);
        // The early nested refusal returns at once; without it the call would wait the full
        // five seconds for the lock and then give the same refusal.
        let started = Instant::now();
        assert_eq!(call(&module), None);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(take_refusal().starts_with(&Refusal::Busy.to_string()));
        // A refusal names the call holding the group, so a busy failure says who it waited on.
        *module.running.holder.lock().unwrap() = Some(Holder {
            module: "holder_mod".into(),
            export: 7,
            thread: std::thread::current(),
            since: Instant::now() - Duration::from_secs(3),
        });
        assert_eq!(call(&module), None);
        let refusal = take_refusal();
        let thread = std::thread::current();
        let thread = thread
            .name()
            .map_or_else(|| format!("{:?}", thread.id()), |name| format!("'{name}'"));
        assert!(
            refusal.starts_with(&format!(
                "{}: held by mod holder_mod export 7 on thread {thread} for 3",
                Refusal::Busy
            )),
            "{refusal}"
        );
        assert!(refusal.contains("; this call waited "), "{refusal}");
        assert!(!module.is_stopped());
        drop(loan);
        module.running.lending.store(0, Ordering::Relaxed);
        drop(guard);
        assert_eq!(call(&module), Some(42));
    })
}

/// A cut-off guest invalidates every member sharing memory, but no other group.
#[test]
fn trap_stops_group_not_independent_group() {
    crate::test_stack::on_guest_stack(|| {
        let engine = engine().unwrap();
        let trap = guest(*mod_api::wire::LAYOUT, "unreachable");
        let echo = guest(*mod_api::wire::LAYOUT, "i64.const 2199023255556");
        let members = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[
                Member {
                    name: "trap",
                    module: &trap,
                },
                Member {
                    name: "echo",
                    module: &echo,
                },
            ],
            None,
        )
        .unwrap();
        let independent = module("i64.const 2199023255556");
        assert_eq!(call(&members[1]), Some(42));
        assert_eq!(call(&members[0]), None);
        assert!(members.iter().all(WasmMod::is_stopped));
        assert_eq!(call(&members[1]), None);
        assert_eq!(call(&independent), Some(42));
    })
}

/// One member's refusal leaves every member of its group answering: the next call of the
/// refusing listener succeeds, and the other member never noticed.
#[test]
fn a_refusal_keeps_every_member_of_the_group_answering() {
    crate::test_stack::on_guest_stack(|| {
        let engine = engine().unwrap();
        // Refuses the first call with an empty error reply at 512, then echoes; the flag lives in
        // guest memory at 600, which the input write at 512 never touches.
        let flaky = guest(
            *mod_api::wire::LAYOUT,
            r#"i32.const 600 i32.load if (result i64) i64.const 2199023255556 else
            i32.const 600 i32.const 1 i32.store i32.const 512 i32.const 0 i32.store
            i64.const 2203318222847 end"#,
        );
        let echo = guest(*mod_api::wire::LAYOUT, "i64.const 2199023255556");
        let members = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[
                Member {
                    name: "flaky",
                    module: &flaky,
                },
                Member {
                    name: "echo",
                    module: &echo,
                },
            ],
            None,
        )
        .unwrap();
        assert_eq!(call(&members[0]), None);
        assert!(members.iter().all(|member| !member.is_stopped()));
        assert_eq!(call(&members[1]), Some(42));
        assert_eq!(call(&members[0]), Some(42));
    })
}

/// The refusal log names each (member, hook) once, not once per call.
#[test]
fn refusal_log_names_each_member_and_hook_once() {
    crate::test_stack::on_guest_stack(|| {
        capture_logs();
        // Hook 0 reports its empty error reply at 512, hook 7 at 516.
        let mut module = module(
            r#"local.get 0 i32.eqz if (result i64) i32.const 512 i32.const 0 i32.store
           i64.const 2203318222847 else i32.const 516 i32.const 0 i32.store
           i64.const 2220498092031 end"#,
        );
        module.id = "hook_log_case".into();
        for _ in 0..2 {
            assert_eq!(call_at(&module, 0), None);
            assert_eq!(call_at(&module, 7), None);
        }
        assert!(!module.is_stopped());
        for hook in [0, 7] {
            assert_eq!(
                CAPTURE
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|m| **m == format!("wasm refusal: hook_log_case hook {hook}: "))
                    .count(),
                1,
                "hook {hook} logs exactly once"
            );
        }
    })
}

/// A refusal during a reply's writeback applies none of it: the record a point listener would
/// commit keeps its old value, and the same call path commits once the guest answers.
#[test]
fn refusal_during_writeback_leaves_the_record_unchanged() {
    crate::test_stack::on_guest_stack(|| {
        let write = |record: &mut (u32, u32), module: &WasmMod| {
            module.call(
                CallClass::Hook,
                Frame::default(),
                0,
                None,
                |w| w.put(&42u32),
                |input| {
                    input.take::<()>()?;
                    let bytes = input.take_bytes(input.rest().len())?;
                    mod_api::wire::Record::apply(record, bytes)
                },
            )
        };
        // A reply of (1, 2) plus one trailing byte: decoded whole, refused, nothing assigned.
        let refused = module(
            "i32.const 512 i32.const 1 i32.store i32.const 516 i32.const 2 i32.store \
         i32.const 520 i32.const -1 i32.store i64.const 2199023255561",
        );
        let mut record = (7u32, 9u32);
        assert_eq!(write(&mut record, &refused), None);
        assert!(!refused.is_stopped());
        assert_eq!(record, (7, 9));
        let answered = module(
            "i32.const 512 i32.const 1 i32.store i32.const 516 i32.const 2 i32.store \
         i64.const 2199023255560",
        );
        assert_eq!(write(&mut record, &answered), Some(()));
        assert_eq!(record, (1, 2));
    })
}

#[test]
fn deep_guest_stack_and_overflow_attribution() {
    capture_logs();
    crate::test_stack::on_guest_stack(|| {
        let before = rss();
        let module = module(
            "i32.const 1024 i32.const 60000 call $recurse i32.store i64.const 4398046511108",
        );
        let loaded = rss();
        for hook in ["render_frame_pre", "server_tick"] {
            let id = crate::boundary::export_id("Hooks", hook).unwrap();
            assert_eq!(call_at(&module, id), Some(60000), "{hook}");
        }
        eprintln!(
            "guest stack VmRSS KiB: unloaded={before:?}, loaded={loaded:?}, deep={:?}",
            rss()
        );
        let (send, receive) = std::sync::mpsc::channel();
        let pool = horizon_threads::jobs::Pool::new(1, "guest-recursion-test");
        let job = Arc::new(RecursionJob {
            module,
            send: Mutex::new(Some(send)),
        });
        let registration = pool.register(job);
        registration.notify();
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(10)).unwrap(),
            Some(60000)
        );
        drop(registration);
        let runaway = module_for_runaway();
        assert_eq!(
            runaway.call(
                CallClass::Load,
                Frame::default(),
                0,
                None,
                |_| {},
                |r| r.take::<u32>()
            ),
            None
        );
        assert!(runaway.is_stopped());
    });
    assert!(
        CAPTURE
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("wasm mod test stopped its group") && line.contains("stack"))
    );
}

fn module_for_runaway() -> WasmMod {
    module("i32.const -1 call $recurse drop i64.const 0")
}

fn rss() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[test]
fn sixty_thousand_frames_exceed_wasmtime_default() {
    crate::test_stack::on_guest_stack(|| {
        let engine = crate::runtime::test_engine();
        let bytes = wat::parse_str(
            r#"(module
          (func $recurse (export "recurse") (param i32) (result i32)
            local.get 0 i32.eqz if (result i32) i32.const 0 else
            local.get 0 i32.const 1 i32.sub call $recurse i32.const 1 i32.add end))"#,
        )
        .unwrap();
        let module = wasmtime::Module::new(&engine, bytes).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let recurse = instance
            .get_typed_func::<i32, i32>(&mut store, "recurse")
            .unwrap();
        let error = recurse.call(&mut store, 60000).unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::StackOverflow)
        );
    });
}

struct RecursionJob {
    module: WasmMod,
    send: Mutex<Option<std::sync::mpsc::Sender<Option<u32>>>>,
}
impl horizon_threads::jobs::Source for RecursionJob {
    fn next(&self) -> Option<horizon_threads::jobs::Rank> {
        self.send
            .lock()
            .unwrap()
            .is_some()
            .then_some(horizon_threads::jobs::Rank::new(
                horizon_threads::jobs::Class::Near,
                0,
            ))
    }
    fn run(&self, _: usize) -> bool {
        let Some(send) = self.send.lock().unwrap().take() else {
            return false;
        };
        send.send(call(&self.module)).unwrap();
        true
    }
}

#[test]
fn small_calling_thread_refuses_without_poisoning_the_mod() {
    crate::test_stack::on_guest_stack(|| {
        capture_logs();
        let module = module("i64.const 2199023255556");
        let module = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                assert!(
                    horizon_guest_stack::check_current()
                        .unwrap_err()
                        .starts_with("GuestStackUnavailable")
                );
                assert_eq!(call(&module), None);
                assert!(!module.is_stopped());
                module
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(call(&module), Some(42));
        assert!(
            CAPTURE
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("wasm mod test: GuestStackUnavailable"))
        );
    })
}

#[test]
fn generated_client_and_server_hooks_complete_sixty_thousand_frames() {
    use mod_api::Hooks;
    struct Host;
    impl mod_api::Host for Host {}
    mod_api::impl_host_areas!(Host);
    impl mod_api::ClientHost for Host {}
    mod_api::impl_client_host_areas!(Host);
    impl mod_api::ServerHost for Host {}
    mod_api::impl_server_host_areas!(Host);

    // A hook writes a known reply only after recursion returns. Call id 0 reads
    // the saved depth, independently proving that the real generated proxy ran it.
    fn recursive_hook(reply: &Writer) -> WasmMod {
        let stores: String = reply
            .bytes()
            .iter()
            .enumerate()
            .map(|(i, byte)| format!("i32.const {} i32.const {byte} i32.store8 ", 2048 + i))
            .collect();
        module(&format!(
            "local.get 0 i32.eqz if (result i64)
            i64.const 4398046511108
            else i32.const 1024 i32.const 60000 call $recurse i32.store
            {stores} i64.const {} end",
            (2048u64 << 32) | reply.bytes().len() as u64
        ))
    }
    crate::test_stack::on_guest_stack(|| {
        let mut host = Host;
        let mut reply = Writer::default();
        reply.put(&Duration::from_secs(60000));
        reply.put(&0.25f32);
        reply.put(&0.5f32);
        let client = recursive_hook(&reply);
        let mut render = mod_api::RenderFramePreEvent {
            elapsed: Duration::from_secs(1),
            world_partial_tick: 0.25,
            partial_tick: 0.5,
        };
        client.render_frame_pre(&mut host, &mut render);
        assert_eq!(render.elapsed, Duration::from_secs(60000));
        assert_eq!(
            (render.world_partial_tick, render.partial_tick),
            (0.25, 0.5)
        );
        assert_eq!(call(&client), Some(60000));

        let mut reply = Writer::default();
        reply.put(&mod_api::ServerTickPhase::ServerTickEnd {
            world_running: true,
        });
        let server = recursive_hook(&reply);
        let mut context_bytes = Writer::default();
        context_bytes.put(&mod_api::Authority::DedicatedServer);
        context_bytes.put(&mod_api::WorldEpoch::new(1));
        context_bytes.put(&mod_api::TickId::new(1));
        let context: mod_api::LifecycleContext = Reader::new(context_bytes.bytes()).take().unwrap();
        let mut tick = mod_api::ServerTickEvent {
            context: &context,
            phase: mod_api::ServerTickPhase::ServerTickStart {
                world_running: true,
            },
        };
        server.server_tick(&mut host, &mut tick);
        assert!(matches!(
            tick.phase,
            mod_api::ServerTickPhase::ServerTickEnd {
                world_running: true
            }
        ));
        assert_eq!(call(&server), Some(60000));
    });
}

#[test]
fn plain_spawn_refuses_guest_entry() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("i64.const 2199023255556");
        let error = std::thread::spawn(move || {
            assert_eq!(call(&module), None);
            horizon_guest_stack::check_current().unwrap_err()
        })
        .join()
        .unwrap();
        assert!(error.starts_with("GuestStackUnavailable"));
    });
}

#[test]
fn subscription_stack_refusal_is_not_cached() {
    crate::test_stack::on_guest_stack(|| {
        let mut names = Writer::new();
        names.put(&vec!["server_tick".to_owned()]);
        let stores: String = names
            .bytes()
            .iter()
            .enumerate()
            .map(|(i, b)| format!("i32.const {} i32.const {b} i32.store8 ", 1024 + i))
            .collect();
        let module = module(&format!(
            "{stores} i64.const {}",
            (1024u64 << 32) | names.bytes().len() as u64
        ));
        let export = crate::boundary::export_id("Hooks", "subscribed")
            .unwrap_or(crate::boundary::SUBSCRIBED);
        let handler = WasmHandler::new(&module, Id(1));
        let small = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                assert!(handler.subscriptions("Hooks", export).is_empty());
                handler
            })
            .unwrap()
            .join()
            .unwrap();
        let subscriptions = small.subscriptions("Hooks", export);
        assert!(matches!(subscriptions, [Some(mod_api::HookId::ServerTick)]));
    });
}

#[test]
fn a_guest_call_that_answers_nothing_names_why() {
    assert_eq!(
        fault_reason(
            &wasmtime::Error::new(wasmtime::Trap::Interrupt),
            100,
            &mut Limits::new("a+b".into(), 64)
        ),
        "epoch deadline: the call ran past its budget of 100 ticks (about 100 ms)"
    );
    assert_eq!(
        fault_reason(
            &wasmtime::Error::new(wasmtime::Trap::StackOverflow),
            100,
            &mut Limits::new("a+b".into(), 64)
        ),
        "wasm trap: call stack exhausted"
    );
    // A trap after a refused memory.grow names the group and its limit, once.
    let mut limits = Limits::new("a+b".into(), 64);
    assert!(
        !wasmtime::ResourceLimiter::memory_growing(&mut limits, 64 << 20, 65 << 20, None).unwrap()
    );
    assert_eq!(
        fault_reason(
            &wasmtime::Error::new(wasmtime::Trap::UnreachableCodeReached),
            100,
            &mut limits
        ),
        "wasm trap: wasm `unreachable` instruction executed, after memory.grow to 65 MiB was refused at group a+b's limit of 64 MiB"
    );
    assert_eq!(
        fault_reason(
            &wasmtime::Error::new(wasmtime::Trap::UnreachableCodeReached),
            100,
            &mut limits
        ),
        "wasm trap: wasm `unreachable` instruction executed"
    );
    // Allowed by the limit but refused by the system: the reason says so, with the size asked.
    let mut limits = Limits::new("a+b".into(), 64);
    assert!(
        wasmtime::ResourceLimiter::memory_growing(&mut limits, 32 << 20, 48 << 20, None).unwrap()
    );
    wasmtime::ResourceLimiter::memory_grow_failed(
        &mut limits,
        wasmtime::Error::msg("out of memory"),
    )
    .unwrap();
    assert_eq!(
        fault_reason(
            &wasmtime::Error::new(wasmtime::Trap::UnreachableCodeReached),
            100,
            &mut limits
        ),
        "wasm trap: wasm `unreachable` instruction executed, after memory.grow to 48 MiB was refused by the system below group a+b's limit of 64 MiB"
    );
    note("its group has stopped after an earlier fault".into());
    assert_eq!(
        take_refusal(),
        "its group has stopped after an earlier fault"
    );
    // Taken once: a later call that answers nothing without a reason does not reuse it.
    assert_eq!(take_refusal(), "refused");
}

#[cfg(feature = "test-copy-probe")]
#[test]
fn prepared_runtime_writes_payload_once_for_eight_scalar_asks() {
    crate::test_stack::on_guest_stack(|| {
        // Protocol-only guest: slice B supplies the owned-stack slot dispatcher.
        let bytes = guest_with_exports(
            *mod_api::wire::LAYOUT,
            "i64.const 0",
            r#"
            (func (export "prepare_can_equip") (result i64) i64.const 2199023255560)
            (func (export "ask_can_equip") (result i64) i64.const 0)
            (func (export "release_can_equip") (result i64) i64.const 0)
        "#,
        );
        let engine = engine().unwrap();
        let module = WasmMod::instantiate(
            &engine,
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "test",
                module: &bytes,
            }],
            None,
        )
        .unwrap()
        .remove(0);
        let weak = module.downgrade();
        let other_module = self::module("i64.const 0");
        let other = other_module.downgrade();
        for size in [16, 4112] {
            let bytes = vec![42u8; size];
            let (token, setup) = crate::copy_probe::measure(|| weak.prepare(&bytes).unwrap());
            assert_eq!(setup.to_guest_bytes, size + 8);
            assert_eq!(setup.from_guest_bytes, 8);
            assert!(weak.check(token.0, token.1));
            assert!(!weak.check(token.0, token.1 + 1));
            assert!(!other.check(token.0, token.1));
            let (_, asks) = crate::copy_probe::measure(|| {
                for _ in 0..8 {
                    assert_eq!(
                        module.call(
                            CallClass::Hook,
                            Frame::default(),
                            0,
                            None,
                            |w| {
                                w.put(&token.0);
                                w.put(&token.1);
                            },
                            |r| r.take::<()>()
                        ),
                        Some(())
                    );
                }
            });
            assert_eq!(asks.to_guest_bytes, 8 * 8);
            assert_eq!(asks.from_guest_bytes, 0);
            weak.release(token.0, token.1);
            assert!(!weak.check(token.0, token.1));
        }
    });
}

/// Grows the group's memory a page at a time until `memory.grow` answers -1, then writes and reads
/// its last byte: the guest goes on at the limit. Answers the pages it reached.
fn grow_until_refused(memory_mib: u32) -> i32 {
    let engine = engine().unwrap();
    let mut store = Store::new(
        &engine,
        State::new(Limits::new("grower".into(), memory_mib)),
    );
    store.limiter(|state| &mut state.limits);
    store.set_epoch_deadline(30_000);
    let memory = Memory::new(&mut store, wasmtime::MemoryType::new(1, None)).unwrap();
    let module = wasmtime::Module::new(
        &engine,
        wat::parse_str(
            r#"(module
            (import "env" "memory" (memory 1))
            (func (export "grow") (result i32)
                (local $pages i32)
                (block $refused
                    (loop $more
                        (br_if $refused (i32.eq (memory.grow (i32.const 1)) (i32.const -1)))
                        (br $more)))
                (local.set $pages (memory.size))
                (i32.store8 (i32.sub (i32.mul (local.get $pages) (i32.const 65536)) (i32.const 1)) (i32.const 7))
                (if (i32.ne (i32.load8_u (i32.sub (i32.mul (local.get $pages) (i32.const 65536)) (i32.const 1))) (i32.const 7))
                    (then unreachable))
                (local.get $pages)))"#,
        )
        .unwrap(),
    )
    .unwrap();
    let instance = wasmtime::Instance::new(&mut store, &module, &[memory.into()]).unwrap();
    let grow = instance
        .get_typed_func::<(), i32>(&mut store, "grow")
        .unwrap();
    let pages = grow.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().limits.refused_growth,
        Some((((pages as usize) + 1) << 16, false))
    );
    pages
}

#[test]
fn a_group_grows_past_256_mib_to_its_default_limit_and_goes_on_there() {
    // 1 GiB of 64 KiB pages: four times the old 256 MiB.
    assert_eq!(grow_until_refused(DEFAULT_GROUP_MEMORY_MIB), 16_384);
}

#[test]
fn a_lowered_limit_refuses_growth_inside_the_guest() {
    assert_eq!(grow_until_refused(64), 1_024);
}

#[test]
fn old_unprepared_can_equip_fingerprint_is_refused_at_load() {
    crate::test_stack::on_guest_stack(|| {
        let old = mod_api::wire_core_layout();
        assert_ne!(old, *mod_api::wire::LAYOUT);
        let bytes = guest(old, "unreachable");
        let refusal = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "old_can_equip",
                module: &bytes,
            }],
            None,
        )
        .err()
        .unwrap();
        assert!(
            refusal
                .to_string()
                .contains("prepared can_equip ABI layout mismatch"),
            "{refusal}"
        );
        assert!(refusal.to_string().contains("old unprepared layout"));
    });
}

#[test]
fn refused_prepared_release_stops_group_before_another_stack_can_be_prepared() {
    crate::test_stack::on_guest_stack(|| {
        let refusal = (1024u64 << 32) | u64::from(u32::MAX);
        let exports = format!(
            r#"
            (func (export "prepare_can_equip") (result i64)
                (memory.copy (i32.const 2048) (i32.const 512) (global.get $in_len))
                (i32.store (i32.const 4000) (i32.add (i32.load (i32.const 4000)) (i32.const 1)))
                i64.const 2199023255560)
            (func (export "ask_can_equip") (result i64)
                (i32.store (i32.const 4004) (i32.const 1)) i64.const 0)
            (func (export "release_can_equip") (result i64)
                (i32.store (i32.const 1024) (i32.const 0)) i64.const {refusal})
        "#
        );
        let bytes = guest_with_exports(*mod_api::wire::LAYOUT, "unreachable", &exports);
        let module = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "refusing_release",
                module: &bytes,
            }],
            None,
        )
        .unwrap()
        .remove(0);
        let owner = module.downgrade();
        let token = owner.prepare(b"first-stack-payload").unwrap();
        owner.release(token.0, token.1);
        assert!(module.is_stopped());
        assert!(!owner.check(token.0, token.1));
        assert!(owner.prepare(b"second-stack-payload").is_none());
        let ask = crate::boundary::export_id("ItemPart", "ask_can_equip").unwrap();
        assert_eq!(
            module.call(
                CallClass::Hook,
                Frame::default(),
                ask,
                None,
                |_| {},
                |r| r.take::<()>()
            ),
            None
        );
        let group = module.running.group.lock().unwrap();
        let memory = group.memory.data(&group.store);
        assert_eq!(
            &memory[2056..2056 + b"first-stack-payload".len()],
            b"first-stack-payload"
        );
        assert_eq!(
            u32::from_le_bytes(memory[4000..4004].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(memory[4004..4008].try_into().unwrap()),
            0
        );
    });
}

#[test]
fn contended_prepared_release_at_capacity_cleans_up_before_slot_reuse() {
    crate::test_stack::on_guest_stack(|| {
        let bytes = guest_with_exports(
            *mod_api::wire::LAYOUT,
            "i64.const 2199023255556",
            r#"
            (func (export "prepare_can_equip") (result i64)
                (if (i32.ge_u (i32.load (i32.const 512)) (i32.const 4096))
                    (then unreachable))
                i64.const 2199023255560)
            (func (export "ask_can_equip") (result i64) i64.const 0)
            (func (export "release_can_equip") (result i64)
                (i32.store (i32.const 4004) (i32.load (i32.const 516)))
                i64.const 0)
            "#,
        );
        let module = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "busy_release",
                module: &bytes,
            }],
            None,
        )
        .unwrap()
        .remove(0);
        let owner = module.downgrade();
        let token = owner.prepare(&[]).unwrap();
        for index in 1..4096 {
            assert_eq!(owner.prepare(&[]).unwrap().0, index);
        }

        let guard = module.running.group.lock().unwrap();
        owner.release(token.0, token.1);
        assert!(!owner.check(token.0, token.1));
        assert!(!module.is_stopped());
        drop(guard);

        assert_eq!(owner.prepare(&[]), None);
        assert!(!module.is_stopped());
        assert_eq!(call(&module), Some(42));
        let reused = owner.prepare(&[]).unwrap();
        assert_eq!(reused.0, token.0);
        assert_ne!(reused.1, token.1);
        let group = module.running.group.lock().unwrap();
        let memory = group.memory.data(&group.store);
        assert_eq!(
            u32::from_le_bytes(memory[4004..4008].try_into().unwrap()),
            token.1
        );
    });
}

/// A member as the production engine links it, alone in its group; the refusal if there is one.
fn link_alone(text: &str) -> Option<crate::link::LinkError> {
    let bytes = wat::parse_str(text).unwrap();
    let engine = crate::runtime::engine().unwrap();
    crate::test_stack::on_guest_stack(|| {
        Group::link(
            &engine,
            &[Member {
                name: "threads",
                module: &bytes,
            }],
        )
        .err()
    })
}

fn refused_with(text: &str, fragment: &str) {
    match link_alone(text) {
        Some(crate::link::LinkError::Malformed { member, why }) => {
            assert_eq!(member, "threads");
            assert!(why.contains(fragment), "{why}");
        }
        other => panic!("expected a malformed refusal containing {fragment:?}, got {other:?}"),
    }
}

#[test]
#[ignore = "daily"]
fn shared_memory_and_atomics_are_refused_and_an_ordinary_member_links() {
    refused_with(
        r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
        (import "env" "memory" (memory 1 1 shared)))"#,
        "threads must be enabled for shared memories",
    );
    refused_with(
        r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
        (import "env" "memory" (memory 1))
        (func (export "read") (result i32) i32.const 0 i32.atomic.load))"#,
        "threads support is not enabled",
    );
    assert!(
        link_alone(
            r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
            (import "env" "memory" (memory 1))
            (func (export "read") (result i32) i32.const 0 i32.load))"#
        )
        .is_none()
    );
}
