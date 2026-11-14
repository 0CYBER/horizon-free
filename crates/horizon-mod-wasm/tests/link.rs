//! The group linker on small shared modules written by hand: one std, one library and one mod,
//! the way `rust-lld --shared` lays them out.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "support/no_hooks.rs"]
mod no_hooks;
use horizon_mod_wasm::link::{Group, LinkError, Member};
use wasmtime::{Engine, Val};

/// std: one static counter, and the function that bumps it.
const STD: &str = r#"(module
  (@custom "dylink.0" "\01\04\04\02\00\00")
  (import "env" "memory" (memory 1))
  (import "env" "__memory_base" (global $base i32))
  (data (global.get $base) "\00\00\00\00")
  (func (export "bump") (result i32)
    (i32.store (global.get $base) (i32.add (i32.load (global.get $base)) (i32.const 1)))
    (i32.load (global.get $base)))
  (global (export "ALLOCS") i32 (i32.const 0)))"#;

/// A library: a function, and a static that starts at 40.
const LIB: &str = r#"(module
  (@custom "dylink.0" "\01\04\04\02\00\00")
  (import "env" "memory" (memory 1))
  (import "env" "__memory_base" (global $base i32))
  (import "env" "bump" (func $bump (result i32)))
  (data (global.get $base) "\28\00\00\00")
  (func (export "lib_add") (param i32 i32) (result i32)
    (drop (call $bump))
    (i32.add (local.get 0) (local.get 1)))
  (global (export "LIB_COUNTER") i32 (i32.const 0)))"#;

/// A mod calling the library's function and reading its static, and std's counter.
const MOD: &str = r#"(module
  (@custom "dylink.0" "\01\04\00\00\00\00")
  (import "env" "memory" (memory 1))
  (import "env" "lib_add" (func $add (param i32 i32) (result i32)))
  (import "env" "bump" (func $bump (result i32)))
  (import "GOT.mem" "LIB_COUNTER" (global $counter (mut i32)))
  (func (export "run") (result i32)
    (drop (call $bump))
    (i32.add (call $add (i32.const 2) (i32.const 3)) (i32.load (global.get $counter))))
  (func (export "bumps") (result i32) (call $bump)))"#;

/// Wasmtime's defaults with wasm threads off, as every Horizon engine has them.
fn engine() -> Engine {
    let mut config = wasmtime::Config::new();
    config.wasm_threads(false).shared_memory(false);
    Engine::new(&config).unwrap()
}

fn bytes(text: &str) -> Vec<u8> {
    wat::parse_str(text).unwrap()
}

fn call(group: &mut Group, member: &str, function: &str) -> i32 {
    let mut out = [Val::I32(0)];
    group.call(member, function, &[], &mut out).unwrap();
    out[0].unwrap_i32()
}

#[test]
fn a_mod_calls_its_library_and_reads_its_static_over_one_shared_std() {
    crate::test_stack::on_guest_stack(|| {
        let (std, lib, user) = (bytes(STD), bytes(LIB), bytes(MOD));
        let mut group = Group::link(
            &engine(),
            &[
                Member {
                    name: "std",
                    module: &std,
                },
                Member {
                    name: "lib",
                    module: &lib,
                },
                Member {
                    name: "mod",
                    module: &user,
                },
            ],
        )
        .unwrap();
        // The static's initial 40, and the call's 2 + 3.
        assert_eq!(call(&mut group, "mod", "run"), 45);
        // Three bumps of one counter: the mod's own, the library's, and this one. Two copies of std
        // would have counted separately.
        assert_eq!(call(&mut group, "mod", "bumps"), 3);
    })
}

#[test]
fn an_import_no_earlier_member_has_is_refused_naming_the_member() {
    crate::test_stack::on_guest_stack(|| {
        let (std, user) = (bytes(STD), bytes(MOD));
        let err = Group::link(
            &engine(),
            &[
                Member {
                    name: "std",
                    module: &std,
                },
                Member {
                    name: "mod",
                    module: &user,
                },
            ],
        )
        .err()
        .unwrap();
        assert_eq!(
            err,
            LinkError::Unresolved {
                member: "mod".into(),
                symbol: "env.lib_add".into()
            }
        );
    })
}

#[test]
fn two_members_exporting_one_name_are_refused() {
    crate::test_stack::on_guest_stack(|| {
        let std = bytes(STD);
        let err = Group::link(
            &engine(),
            &[
                Member {
                    name: "std",
                    module: &std,
                },
                Member {
                    name: "again",
                    module: &std,
                },
            ],
        )
        .err()
        .unwrap();
        assert!(matches!(err, LinkError::Duplicate { ref symbol, .. } if symbol == "bump"));
    })
}

#[test]
fn bytes_that_are_not_a_shared_module_are_refused_without_a_panic() {
    crate::test_stack::on_guest_stack(|| {
        let plain = bytes("(module)");
        let engine = engine();
        let err = Group::link(
            &engine,
            &[Member {
                name: "plain",
                module: &plain,
            }],
        )
        .err();
        assert_eq!(
            err,
            Some(LinkError::NoDylink {
                member: "plain".into()
            })
        );
        for junk in [
            &b""[..],
            b"\0asm",
            b"not wasm at all",
            b"\0asm\x01\0\0\0\x00\x05\x04dyli",
        ] {
            assert!(
                Group::link(
                    &engine,
                    &[Member {
                        name: "junk",
                        module: junk
                    }]
                )
                .is_err()
            );
        }
        // A member that asks for more memory than a group has.
        let huge = bytes(
            r#"(module (@custom "dylink.0" "\01\08\ff\ff\ff\ff\0f\02\00\00") (import "env" "memory" (memory 1)))"#,
        );
        let err = Group::link(
            &engine,
            &[Member {
                name: "huge",
                module: &huge,
            }],
        )
        .err();
        assert_eq!(
            err,
            Some(LinkError::TooLarge {
                member: "huge".into()
            })
        );
    })
}

const SELF_GOT: &str = r#"(module
  (@custom "dylink.0" "\01\04\04\02\00\00")
  (type $get (func (result i32)))
  (import "env" "memory" (memory 1))
  (import "env" "__indirect_function_table" (table 1 funcref))
  (import "env" "__memory_base" (global $base i32))
  (import "GOT.func" "answer" (global $answer_ptr (mut i32)))
  (import "GOT.mem" "SELF_DATA" (global $data (mut i32)))
  (data (global.get $base) "\28\00\00\00")
  (global (export "SELF_DATA") i32 (i32.const 0))
  (func $answer (export "answer") (result i32) (i32.const 2))
  (func $ctor (export "__wasm_call_ctors")
    (i32.store (global.get $data)
      (i32.add (i32.load (global.get $data))
        (call_indirect (type $get) (global.get $answer_ptr)))))
  (func (export "run") (result i32) (i32.load (global.get $data))))"#;

#[test]
fn self_got_function_and_data_relocations_are_ready_before_constructors() {
    crate::test_stack::on_guest_stack(|| {
        let module = bytes(SELF_GOT);
        let mut group = Group::link(
            &engine(),
            &[Member {
                name: "self",
                module: &module,
            }],
        )
        .unwrap();
        assert_eq!(call(&mut group, "self", "run"), 42);
    })
}

#[test]
fn self_got_requires_an_actual_export_of_the_right_kind() {
    crate::test_stack::on_guest_stack(|| {
        for source in [
            SELF_GOT.replace("(export \"answer\")", ""),
            SELF_GOT.replace(
                "(func $answer (export \"answer\")",
                "(global (export \"answer\") i32 (i32.const 0)) (func $answer",
            ),
        ] {
            let module = bytes(&source);
            let error = Group::link(
                &engine(),
                &[Member {
                    name: "self",
                    module: &module,
                }],
            )
            .err()
            .unwrap();
            assert_eq!(
                error,
                LinkError::Unresolved {
                    member: "self".into(),
                    symbol: "GOT.func.answer".into()
                }
            );
        }
        let module = bytes(&SELF_GOT.replace("(global (export \"SELF_DATA\")", "(global"));
        let error = Group::link(
            &engine(),
            &[Member {
                name: "self",
                module: &module,
            }],
        )
        .err()
        .unwrap();
        assert_eq!(
            error,
            LinkError::Unresolved {
                member: "self".into(),
                symbol: "GOT.mem.SELF_DATA".into()
            }
        );
    })
}

#[test]
fn self_got_does_not_allow_a_start_function_to_observe_unrelocated_state() {
    crate::test_stack::on_guest_stack(|| {
        let module = bytes(&SELF_GOT.replace("(func $ctor", "(start $ctor) (func $ctor"));
        let error = Group::link(
            &engine(),
            &[Member {
                name: "self",
                module: &module,
            }],
        )
        .err()
        .unwrap();
        assert!(matches!(error, LinkError::Malformed { member, .. } if member == "self"));
    })
}

#[test]
fn self_got_delayed_start_runs_after_relocation_and_before_constructors() {
    crate::test_stack::on_guest_stack(|| {
        let source = SELF_GOT.replace("(func $ctor",
        "(func (export \"__wasm_start\") (i32.store (global.get $data) (i32.const 50))) (func $ctor");
        let module = bytes(&source);
        let mut group = Group::link(
            &engine(),
            &[Member {
                name: "self",
                module: &module,
            }],
        )
        .unwrap();
        assert_eq!(call(&mut group, "self", "run"), 52);
    })
}

#[test]
fn self_got_private_helpers_and_data_belong_to_each_member() {
    crate::test_stack::on_guest_stack(|| {
        let source = SELF_GOT.replacen("(type $get", "(@custom \"horizon.private\" \"[\\22answer\\22,\\22SELF_DATA\\22,\\22run\\22]\") (type $get", 1);
        let first = bytes(&source);
        let second = bytes(&source.replace("(i32.const 2))", "(i32.const 3))"));
        let mut group = Group::link(
            &engine(),
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
        assert_eq!(call(&mut group, "first", "run"), 42);
        assert_eq!(call(&mut group, "second", "run"), 43);
    })
}

#[test]
fn self_got_private_function_does_not_bind_an_earlier_public_function() {
    crate::test_stack::on_guest_stack(|| {
        let first = bytes(&SELF_GOT.replacen(
            "(type $get",
            r#"(@custom "horizon.private" "[\22SELF_DATA\22,\22run\22]") (type $get"#,
            1,
        ));
        let second = bytes(&SELF_GOT.replacen("(type $get",
        r#"(@custom "horizon.private" "[\22answer\22,\22SELF_DATA\22,\22run\22]") (type $get"#, 1)
        .replace("(i32.const 2))", "(i32.const 3))"));
        let mut group = Group::link(
            &engine(),
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
        assert_eq!(call(&mut group, "first", "run"), 42);
        assert_eq!(call(&mut group, "second", "run"), 43);
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
