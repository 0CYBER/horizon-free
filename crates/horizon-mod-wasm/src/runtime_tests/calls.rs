//! Calls cross by name (#18253): a mod links each call it makes by its name and signature, and
//! the game compares the mod's table of calls with its own when it loads.
use super::*;
use horizon_mod_runtime::dispatch_failures::dispatch_failure_counter as failure_counter;

const NEXT_LONG: (&str, &str) = ("StructureRandom", "next_long");

/// A guest importing `imports` (wasm import names) and answering `CALLS` with `table`; every
/// other export runs `body`.
fn member(imports: &[&str], table: &[(String, u64, bool)], body: &str) -> Vec<u8> {
    let declared: String = imports
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(r#"(import "horizon" "{name}" (func $call{i} (param i32 i32 i32 i32) (result i32)))"#)
        })
        .collect();
    let mut bytes = Writer::new();
    bytes.put(&table.to_vec());
    let stores: String = bytes
        .bytes()
        .iter()
        .enumerate()
        .map(|(offset, byte)| format!("i32.const {} i32.const {byte} i32.store8 ", 4096 + offset))
        .collect();
    wat::parse_str(format!(
        r#"(module
      (@custom "dylink.0" "\01\04\00\00\00\00")
      (import "env" "memory" (memory 1))
      {declared}
      (func (export "horizon_layout") (result i64) i64.const {layout})
      (func (export "horizon_in") (param i32) (result i32) i32.const 512)
      (global $calls (mut i32) (i32.const 0))
      (func (export "horizon_hook") (param i32) (result i64)
        local.get 0 i32.const {calls} i32.eq if (result i64)
          {stores} i64.const {answer}
        else
          global.get $calls i32.const 1 i32.add global.set $calls {body}
        end))"#,
        layout = *mod_api::wire::LAYOUT,
        calls = mod_api::wire::CALLS as i32,
        answer = (4096u64 << 32) | bytes.bytes().len() as u64,
    ))
    .unwrap()
}

fn load(members: &[(&str, &[u8])]) -> Result<Vec<WasmMod>, Refusal> {
    let members: Vec<_> = members
        .iter()
        .map(|(name, module)| Member { name, module })
        .collect();
    WasmMod::instantiate(&engine().unwrap(), DEFAULT_GROUP_MEMORY_MIB, &members, None)
}

/// The game's own entry for `key`, as a mod built against this game answers it.
fn entry(key: &str) -> (String, u64, bool) {
    let (_, layout, export) = mod_api::call_layouts()
        .into_iter()
        .find(|(k, ..)| *k == key)
        .unwrap_or_else(|| panic!("the game has no call {key}"));
    (key.to_owned(), layout, export)
}

fn import(call: (&str, &str)) -> &'static str {
    crate::calls::import_name(call.0, call.1).unwrap()
}

#[test]
fn a_call_linked_by_name_reaches_the_games_method() {
    crate::test_stack::on_guest_stack(|| {
        let name = import(NEXT_LONG);
        let bytes = member(
            &[name],
            &[entry(crate::calls::key(name))],
            "i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $call0 drop i64.const 8796093022216",
        );
        let module = load(&[("test", &bytes)]).unwrap().remove(0);
        let mut random = Random(|| 41);
        let mut frame = Frame::default();
        frame.set_structure_random_mut(&mut random);
        assert_eq!(
            module.call(CallClass::Hook, frame, 7, None, |_| {}, |r| r.take::<i64>()),
            Some(41)
        );
    })
}

#[test]
fn a_call_whose_signature_changed_is_refused_naming_it() {
    crate::test_stack::on_guest_stack(|| {
        let bytes = member(
            &["StructureRandom::next_long#0000000000000000"],
            &[],
            "unreachable",
        );
        let refusal = load(&[("test", &bytes)]).err().unwrap();
        assert_eq!(
            refusal,
            Refusal::Call {
                call: "StructureRandom::next_long".into(),
                changed: true
            }
        );
        assert!(refusal.to_string().contains("`StructureRandom::next_long`"));
    })
}

#[test]
fn a_call_the_game_no_longer_has_is_refused_naming_it() {
    crate::test_stack::on_guest_stack(|| {
        let bytes = member(
            &["StructureRandom::next_gone#0000000000000000"],
            &[],
            "unreachable",
        );
        assert_eq!(
            load(&[("test", &bytes)]).err().unwrap(),
            Refusal::Call {
                call: "StructureRandom::next_gone".into(),
                changed: false
            }
        );
    })
}

/// Linking matches the signature; a type the call reaches whose wire layout changed is caught by
/// the table, before the mod runs anything.
#[test]
fn a_call_reaching_a_type_whose_layout_changed_is_refused_naming_it() {
    crate::test_stack::on_guest_stack(|| {
        let name = import(NEXT_LONG);
        let (key, layout, export) = entry(crate::calls::key(name));
        let bytes = member(&[name], &[(key, layout ^ 1, export)], "unreachable");
        assert_eq!(
            load(&[("test", &bytes)]).err().unwrap(),
            Refusal::Call {
                call: "StructureRandom::next_long".into(),
                changed: true
            }
        );
    })
}

/// A mod built when `data_map_value` had another access level answers another hash for it (the
/// generator's `a_calls_access_is_part_of_its_hash`): it is refused, naming the call, before it
/// could call into a phase that now refuses it. A mod that doesn't import that call loads.
#[test]
fn a_call_whose_access_changed_is_refused_and_other_mods_load() {
    crate::test_stack::on_guest_stack(|| {
        let name = import(("Backend", "data_map_value"));
        let (key, layout, export) = entry(crate::calls::key(name));
        let built_before = member(&[name], &[(key, layout ^ 1, export)], "unreachable");
        assert_eq!(
            load(&[("test", &built_before)]).err().unwrap(),
            Refusal::Call {
                call: "Backend::data_map_value".into(),
                changed: true
            }
        );
        let other = import(NEXT_LONG);
        let unaffected = member(&[other], &[entry(crate::calls::key(other))], "unreachable");
        assert!(load(&[("test", &unaffected)]).is_ok());
    })
}

#[test]
fn an_export_whose_hash_changed_is_refused_before_it_is_called() {
    crate::test_stack::on_guest_stack(|| {
        let (key, layout, export) = entry("Hooks::server_stopped");
        let bytes = member(&[], &[(key, layout ^ 1, export)], "unreachable");
        assert_eq!(
            load(&[("test", &bytes)]).err().unwrap(),
            Refusal::Call {
                call: "Hooks::server_stopped".into(),
                changed: true
            }
        );
    })
}

#[test]
fn an_export_the_game_dropped_is_refused_naming_it() {
    crate::test_stack::on_guest_stack(|| {
        // Its trait's only export: the game has nothing left of the trait to compare it with.
        for gone in ["Hooks::gone_hook", "GoneTrait::only_export"] {
            let bytes = member(&[], &[(gone.into(), 1, true)], "unreachable");
            assert_eq!(
                load(&[("test", &bytes)]).err().unwrap(),
                Refusal::Call {
                    call: gone.into(),
                    changed: false
                }
            );
        }
    })
}

/// A mod built before an export existed loads; a call of that export answers that the mod has
/// none, without stopping it or counting as a failed dispatch.
#[test]
fn an_export_added_after_the_mod_was_built_is_answered_as_absent() {
    crate::test_stack::on_guest_stack(|| {
        let refusal = (512u64 << 32) | u64::from(u32::MAX);
        let bytes = member(&[], &[], &format!("i64.const {refusal}"));
        let module = load(&[("test", &bytes)]).unwrap().remove(0);
        let id = crate::boundary::export_id("Hooks", "server_stopped").unwrap();
        let failures = failure_counter();
        let before = failures.get();
        assert_eq!(
            module.call(
                CallClass::Hook,
                Frame::default(),
                id,
                None,
                |_| {},
                |r| r.take::<()>()
            ),
            None
        );
        assert_eq!(failures.get(), before);
        assert!(!module.is_stopped());
    })
}

#[test]
fn a_linked_group_built_against_different_apis_is_refused() {
    crate::test_stack::on_guest_stack(|| {
        let older = member(&[], &[], "unreachable");
        let newer = member(&[], &[entry("Hooks::server_stopped")], "unreachable");
        assert_eq!(
            load(&[("liba.so", &older), ("libb.so", &newer)])
                .err()
                .unwrap(),
            Refusal::MixedGroup {
                first: "liba.so".into(),
                second: "libb.so".into()
            }
        );
        assert!(load(&[("liba.so", &older), ("libb.so", &older)]).is_ok());
    })
}

/// A change to any value the transport uses (a reserved id, the group state's size or a word's
/// place) changes the core layout, so a mod built before it is refused at load.
#[test]
fn a_changed_transport_value_refuses_the_mod() {
    crate::test_stack::on_guest_stack(|| {
        let complete = |core: u64| mod_api::wire::layout_mix(core, mod_api::prepared_abi_layout());
        assert_eq!(
            complete(mod_api::wire_core_layout_with(mod_api::wire::PROTOCOL)),
            *mod_api::wire::LAYOUT
        );
        let changed = |name: &str| {
            let protocol: Vec<(&str, u64)> = mod_api::wire::PROTOCOL
                .iter()
                .map(|&(n, value)| (n, if n == name { value + 1 } else { value }))
                .collect();
            complete(mod_api::wire_core_layout_with(&protocol))
        };
        for &(name, _) in mod_api::wire::PROTOCOL {
            assert_ne!(changed(name), *mod_api::wire::LAYOUT, "{name}");
        }
        let older = guest_with_exports(changed("HOST_RELEASE"), "i64.const 0", "");
        assert!(matches!(
            load(&[("test", &older)]).err().unwrap(),
            Refusal::Layout { .. }
        ));
    })
}

/// A shared std holding another number of group-state words than the game's is refused.
#[test]
fn a_std_with_another_group_state_is_refused() {
    crate::test_stack::on_guest_stack(|| {
        // A refused std's constructor traps: the refusal must come before it runs.
        let std = |ctor: &str, words: &str| {
            wat::parse_str(format!(
                r#"(module
              (@custom "dylink.0" "\01\04\00\00\00\00")
              (import "env" "memory" (memory 1))
              (global (export "horizon_group_state") i32 (i32.const 0))
              (func (export "__wasm_call_ctors") {ctor})
              {words})"#
            ))
            .unwrap()
        };
        let words = |count: usize| {
            format!(r#"(func (export "horizon_group_words") (result i32) i32.const {count})"#)
        };
        let mod_ = member(&[], &[], "unreachable");
        let refused = |std: &[u8]| match load(&[("libstd.so", std), ("test", &mod_)]) {
            Err(Refusal::Runtime(why)) => why,
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("loaded"),
        };
        let right = mod_api::wire::GROUP_WORDS;
        let stores = "i32.const 0 i32.const 7 i32.store";
        assert!(load(&[("libstd.so", &std(stores, &words(right))), ("test", &mod_)]).is_ok());
        assert!(
            refused(&std("unreachable", &words(right + 1)))
                .contains(&format!("holds {} group-state words", right + 1))
        );
        assert!(refused(&std("unreachable", "")).contains("doesn't say how much group state"));
    })
}
