//! One case per boundary rule, on small surfaces and call tables written out here. The calls'
//! hashes are what the generator would write; real trees are checked by replaying
//! `horizon-stage surface-diff --calls` on them.
use super::{Boundary, COMPILED_SCHEMA, Call, Compiled, CompiledCall, Owned, SCHEMA, apply};
use crate::diff::Class;
use crate::*;

fn hooks_trait(methods: &[&str]) -> Trait {
    Trait::new(
        "trait:Hooks",
        "",
        TraitContract {
            role: Role::Implement,
            side: Side::Both,
            generics: String::new(),
            bounds: vec![],
            required: vec![],
            refs: vec![],
        },
        methods
            .iter()
            .map(|name| {
                Method::new(
                    format!("method:Hooks::{name}"),
                    "",
                    MethodContract {
                        signature: format!("fn {name}(&self)"),
                        required: false,
                        wasm: true,
                        refs: vec![],
                    },
                )
            })
            .collect(),
        vec![],
    )
}

/// An owned enum that crosses as `Wire`, with its layout hash written by hand.
fn crossing_enum(name: &str, variants: &[&str], shape: &str) -> Type {
    let mut t = Type::new(
        format!("type:{name}"),
        "",
        None,
        TypeContract {
            kind: TypeKind::Enum,
            generics: String::new(),
            wire: true,
            wire_form: Some("derive".into()),
            constructible: false,
            non_exhaustive: true,
            target: None,
            members: vec![],
            refs: vec![],
        },
        vec![],
        variants
            .iter()
            .map(|v| {
                Variant::new(
                    format!("type:{name}.{v}"),
                    "",
                    VariantContract {
                        fields: vec![],
                        named: false,
                        refs: vec![],
                    },
                )
            })
            .collect(),
    );
    t.shape = Some(shape.into());
    t.paths = vec![name.into()];
    t
}

fn surface(methods: &[&str], types: Vec<Type>) -> Surface {
    Surface::new(
        vec![],
        vec![hooks_trait(methods)],
        vec![],
        vec![],
        vec![],
        types,
        vec![],
        vec![],
    )
}

/// An owned type with its shape; `""` for one without (a hand-written codec).
fn owned(list: &[(&str, &str)]) -> Vec<Owned> {
    list.iter()
        .map(|(path, shape)| Owned {
            path: (*path).into(),
            shape: (!shape.is_empty()).then(|| (*shape).into()),
        })
        .collect()
}

fn call(key: &str, signature: &str, export: bool, reached: &[(&str, &str)]) -> Call {
    Call {
        key: key.into(),
        signature: signature.into(),
        export,
        owned: owned(reached),
    }
}

fn boundary(core: &str, calls: Vec<Call>) -> Boundary {
    Boundary {
        schema: SCHEMA,
        core: core.into(),
        core_owned: owned(&[("mod_api::PackageInfo", "p0")]),
        calls,
    }
}

/// The surface diff with the boundary applied.
fn compared(old: (&Surface, &Boundary), new: (&Surface, &Boundary)) -> Diff {
    let mut d = crate::diff(old.0, new.0);
    apply(&mut d, old.0, new.0, old.1, new.1, None);
    d
}

/// A build's compiled hashes: its core layout and its calls' (key, layout).
fn built(core: &str, calls: &[(&str, &str)]) -> Compiled {
    Compiled {
        schema: COMPILED_SCHEMA,
        core: core.into(),
        calls: calls
            .iter()
            .map(|(key, layout)| CompiledCall {
                key: (*key).into(),
                layout: (*layout).into(),
                export: true,
            })
            .collect(),
    }
}

/// The surface diff with the boundary and both builds' compiled hashes applied.
fn compiled(old: (&Surface, &Boundary, &Compiled), new: (&Surface, &Boundary, &Compiled)) -> Diff {
    let mut d = crate::diff(old.0, new.0);
    apply(&mut d, old.0, new.0, old.1, new.1, Some((old.2, new.2)));
    d
}

fn undeclared(d: &Diff) -> Vec<&str> {
    d.undeclared().map(|c| c.id.as_str()).collect()
}

#[test]
fn a_changed_signature_of_an_existing_call_breaks_under_its_method() {
    // `TerrainUvShift<'a>`'s `scale` and `offset` swapped, or a field added to it: it is a
    // borrowing record reached only through `TerrainModelOptions<'a>`, so only the signature
    // hash of `terrain_model` changes.
    let s = surface(&["terrain_model"], vec![]);
    let old = boundary(
        "c0",
        vec![call("Hooks::terrain_model", "00000000000000aa", false, &[])],
    );
    let new = boundary(
        "c0",
        vec![call("Hooks::terrain_model", "00000000000000bb", false, &[])],
    );
    let d = compared((&s, &old), (&s, &new));
    assert_eq!(d.verdict, Verdict::Breaking);
    assert_eq!(undeclared(&d), ["method:Hooks::terrain_model"]);
    assert_eq!(
        d.changes[0].what,
        [
            "call `Hooks::terrain_model`: its signature hash changed (`00000000000000aa` → `00000000000000bb`), its arguments, answer or a borrowing record they reach: every wasm mod built before it is refused at load"
        ]
    );
}

#[test]
fn a_field_added_to_an_owned_event_breaks_the_hook_that_carries_it() {
    // #19020: `PlayerMoveEvent` gains `on_ground`, `has_position` and `has_rotation`. It is
    // owned, so `player_move`'s signature hash stays and the event's layout changes.
    let event = |shape: &str| {
        let mut t = crossing_enum("PlayerMoveEvent", &[], shape);
        t.contract.kind = TypeKind::Struct;
        t
    };
    let reaching = |shape| {
        boundary(
            "c0",
            vec![call(
                "Hooks::player_move",
                "01",
                true,
                &[("mod_api::PlayerMoveEvent", shape)],
            )],
        )
    };
    let old = surface(&["player_move"], vec![event("ec4f")]);
    let new = surface(&["player_move"], vec![event("ba66")]);
    let d = compared((&old, &reaching("ec4f")), (&new, &reaching("ba66")));
    assert_eq!(undeclared(&d), ["type:PlayerMoveEvent"]);
    assert_eq!(
        d.changes[0].what,
        [
            "wire layout: `ec4f` → `ba66`: reached by `Hooks::player_move`, so every wasm mod built before it is refused at load"
        ]
    );
    // Without a call that reaches it, the same change stays conditional.
    let none = boundary("c0", vec![call("Hooks::player_move", "01", true, &[])]);
    let d = compared((&old, &none), (&new, &none));
    assert_eq!(d.verdict, Verdict::Conditional);
}

#[test]
fn a_hidden_owned_type_a_call_reaches_breaks_under_its_path() {
    // `EffectTickChange` is `#[doc(hidden)]`: the surface has no record of it, the calls table
    // has its shape.
    let s = surface(&["apply_effect_tick"], vec![]);
    let reaching = |shape| {
        boundary(
            "c0",
            vec![call(
                "Hooks::apply_effect_tick",
                "01",
                true,
                &[("mod_api::EffectTickChange", shape)],
            )],
        )
    };
    let d = compared((&s, &reaching("e1")), (&s, &reaching("e2")));
    assert_eq!(undeclared(&d), ["type:EffectTickChange"]);
    assert_eq!(
        d.changes[0].what,
        [
            "wire layout: `e1` → `e2`: reached by `Hooks::apply_effect_tick`, so every wasm mod built before it is refused at load"
        ]
    );
}

#[test]
fn a_layout_change_of_an_owned_type_the_core_reaches_breaks() {
    let s = surface(&[], vec![]);
    let old = boundary("c0", vec![]);
    let mut new = boundary("c0", vec![]);
    new.core_owned = owned(&[("mod_api::PackageInfo", "p1")]);
    let d = compared((&s, &old), (&s, &new));
    assert_eq!(undeclared(&d), ["type:PackageInfo"]);
    assert_eq!(
        d.changes[0].what,
        [
            "wire layout: `p0` → `p1`: reached by the wire core, so every wasm mod built before it is refused at load"
        ]
    );
}

#[test]
fn a_call_with_no_trait_method_record_is_reported_by_key() {
    let s = surface(&[], vec![]);
    let old = boundary("c0", vec![call("held-Sink::write", "01", false, &[])]);
    let new = boundary("c0", vec![call("held-Sink::write", "02", false, &[])]);
    assert_eq!(
        undeclared(&compared((&s, &old), (&s, &new))),
        ["call:Sink::write"]
    );
}

#[test]
fn a_gone_call_or_a_changed_core_breaks_and_a_new_call_does_not() {
    let s = surface(&["tick"], vec![]);
    let old = boundary("c0", vec![call("Hooks::tick", "01", true, &[])]);
    let gone = boundary("c0", vec![]);
    assert_eq!(
        undeclared(&compared((&s, &old), (&s, &gone))),
        ["method:Hooks::tick"]
    );
    let core = boundary("c1", old.calls.clone());
    let d = compared((&s, &old), (&s, &core));
    assert_eq!(undeclared(&d), ["core:wire"]);
    assert!(d.changes[0].what[0].starts_with("the wire core changed (`c0` → `c1`)"));
    let more = boundary(
        "c0",
        vec![
            call("Hooks::tick", "01", true, &[]),
            call("Hooks::more", "02", true, &[("mod_api::New", "n0")]),
        ],
    );
    let d = compared((&s, &old), (&s, &more));
    assert_eq!(d.verdict, Verdict::Identical);
}

#[test]
fn an_owned_layout_change_breaks_only_where_an_existing_call_reaches_the_type() {
    // #18742: `StaleEntity` inserted before `OtherLevel`. Appending changes the layout too: the
    // game hashes every variant in order.
    let old_s = surface(
        &["place_block"],
        vec![
            crossing_enum("PlayerWorldRefusal", &["Stale", "OtherLevel"], "aaaa"),
            crossing_enum("GuestOnly", &["A"], "1111"),
        ],
    );
    let reaching = |shape| {
        boundary(
            "c0",
            vec![call(
                "Hooks::place_block",
                "01",
                true,
                &[("mod_api::PlayerWorldRefusal", shape)],
            )],
        )
    };
    for (variants, shape) in [
        (&["Stale", "StaleEntity", "OtherLevel"][..], "bbbb"),
        (&["Stale", "OtherLevel", "StaleEntity"][..], "cccc"),
    ] {
        let new_s = surface(
            &["place_block"],
            vec![
                crossing_enum("PlayerWorldRefusal", variants, shape),
                crossing_enum("GuestOnly", &["A", "B"], "2222"),
            ],
        );
        let d = compared((&old_s, &reaching("aaaa")), (&new_s, &reaching(shape)));
        assert_eq!(undeclared(&d), ["type:PlayerWorldRefusal"], "{variants:?}");
        let refusal = d
            .changes
            .iter()
            .find(|c| c.id == "type:PlayerWorldRefusal")
            .unwrap();
        assert!(
            refusal.what.contains(&format!(
                "wire layout: `aaaa` → `{shape}`: reached by `Hooks::place_block`, so every wasm mod built before it is refused at load"
            )),
            "{:?}",
            refusal.what
        );
        // No call reaches it: a mod that never names it is not refused.
        let local = d.changes.iter().find(|c| c.id == "type:GuestOnly").unwrap();
        assert_eq!(local.class, Class::Conditional);
    }
}

#[test]
fn a_changed_owned_type_list_breaks() {
    let s = surface(&["tick"], vec![]);
    let old = boundary(
        "c0",
        vec![call("Hooks::tick", "01", true, &[("mod_api::A", "a")])],
    );
    let new = boundary(
        "c0",
        vec![call(
            "Hooks::tick",
            "01",
            true,
            &[("mod_api::A", "a"), ("mod_api::B", "b")],
        )],
    );
    let d = compared((&s, &old), (&s, &new));
    assert_eq!(undeclared(&d), ["method:Hooks::tick"]);
    assert!(d.changes[0].what[0].contains("(gained: `mod_api::B`; lost: )"));
}

#[test]
fn another_schema_is_refused() {
    let mut b = boundary(
        "c0",
        vec![call("Hooks::tick", "01", true, &[("mod_api::A", "")])],
    );
    assert_eq!(Boundary::read(b.to_json().as_bytes()).unwrap(), b);
    b.schema = SCHEMA + 1;
    assert!(Boundary::read(b.to_json().as_bytes()).is_err());
}

#[test]
fn a_compiled_change_the_source_does_not_show_breaks_under_the_call() {
    // `PackageInfo`'s private `kind` stored as a new derived enum with the same variants: the
    // shape leaves type names out, the derive hashes them.
    let s = surface(&["tick"], vec![]);
    let b = boundary("c0", vec![call("Hooks::tick", "01", true, &[])]);
    let d = compiled(
        (&s, &b, &built("l0", &[("Hooks::tick", "a1")])),
        (&s, &b, &built("l0", &[("Hooks::tick", "a2")])),
    );
    assert_eq!(undeclared(&d), ["method:Hooks::tick"]);
    assert_eq!(
        d.changes[0].what,
        [
            "call `Hooks::tick`: its compiled layout changed (`a1` → `a2`): every wasm mod built before it is refused at load"
        ]
    );
    let d = compiled(
        (&s, &b, &built("l0", &[("Hooks::tick", "a1")])),
        (&s, &b, &built("l1", &[("Hooks::tick", "a1")])),
    );
    assert_eq!(undeclared(&d), ["core:wire"]);
    assert!(d.changes[0].what[0].starts_with("the compiled core layout changed (`l0` → `l1`)"));
    // Unchanged compiled hashes add nothing.
    let same = built("l0", &[("Hooks::tick", "a1")]);
    let d = compiled((&s, &b, &same), (&s, &b, &same));
    assert_eq!(d.verdict, Verdict::Identical);
}

#[test]
fn a_type_change_does_not_cover_a_second_compiled_change_of_the_core() {
    // One pull request swaps `EquipReadRow.health` and `max_health` (the source shows it) and
    // stores `PackageInfo.kind` as a new derived enum (only the compiled core shows it). A note
    // naming `type:EquipReadRow` alone must not pass it.
    let s = surface(&[], vec![]);
    let core = |equip: &str| {
        let mut b = boundary("c0", vec![]);
        b.core_owned = owned(&[
            ("mod_api::EquipReadRow", equip),
            ("mod_api::PackageInfo", "p0"),
        ]);
        b
    };
    let d = compiled(
        (&s, &core("e1"), &built("l0", &[])),
        (&s, &core("e2"), &built("l1", &[])),
    );
    assert_eq!(undeclared(&d), ["core:wire", "type:EquipReadRow"]);
    let wire = d.changes.iter().find(|c| c.id == "core:wire").unwrap();
    assert_eq!(
        wire.what,
        [
            "the compiled core layout changed (`l0` → `l1`); the source shows `mod_api::EquipReadRow` changed, which it reaches, and cannot tell whether that is all: every wasm mod built before it is refused at load"
        ]
    );
    // Without the compiled layouts, the source change alone.
    let d = compared((&s, &core("e1")), (&s, &core("e2")));
    assert_eq!(undeclared(&d), ["type:EquipReadRow"]);
}

#[test]
fn a_type_change_does_not_cover_a_second_compiled_change_of_a_call() {
    // #18742's `PlayerWorldRefusal` insert, with another hidden change on the same call: the call
    // is named too, so a note must list both ids.
    let s = surface(&["place_block"], vec![]);
    let reaching = |shape| {
        boundary(
            "c0",
            vec![call(
                "Hooks::place_block",
                "01",
                true,
                &[("mod_api::PlayerWorldRefusal", shape)],
            )],
        )
    };
    let d = compiled(
        (
            &s,
            &reaching("aaaa"),
            &built("l0", &[("Hooks::place_block", "a1")]),
        ),
        (
            &s,
            &reaching("bbbb"),
            &built("l0", &[("Hooks::place_block", "a2")]),
        ),
    );
    assert_eq!(
        undeclared(&d),
        ["method:Hooks::place_block", "type:PlayerWorldRefusal"]
    );
    // A signature change the source shows is under the call's own id: the compiled change adds a
    // line there, no second id.
    let signed = |signature| boundary("c0", vec![call("Hooks::place_block", signature, true, &[])]);
    let d = compiled(
        (
            &s,
            &signed("01"),
            &built("l0", &[("Hooks::place_block", "a1")]),
        ),
        (
            &s,
            &signed("02"),
            &built("l0", &[("Hooks::place_block", "a2")]),
        ),
    );
    assert_eq!(undeclared(&d), ["method:Hooks::place_block"]);
    assert_eq!(d.changes[0].what.len(), 2);
}

#[test]
fn compiled_layouts_of_another_schema_are_refused() {
    let mut b = built("l0", &[("Hooks::tick", "a1")]);
    let text = serde_json::to_string(&b).unwrap();
    assert_eq!(Compiled::read(text.as_bytes()).unwrap(), b);
    b.schema = COMPILED_SCHEMA + 1;
    assert!(Compiled::read(serde_json::to_string(&b).unwrap().as_bytes()).is_err());
}

#[test]
fn a_call_only_the_new_build_compiled_is_additive() {
    // The reviewer's case: the head's generator emits one more call for a method the base
    // already had, so its reading of both trees lists the same calls, and only the compiled
    // tables differ: 2315 calls in the base's, 2316 in the head's.
    let s = surface(&["tick"], vec![]);
    let b = boundary("c0", vec![call("Hooks::tick", "01", true, &[])]);
    let keys: Vec<String> = (0..2316).map(|i| format!("Hooks::k{i:04}")).collect();
    let table = |n: usize| {
        let calls: Vec<(&str, &str)> = keys[..n].iter().map(|k| (k.as_str(), "a1")).collect();
        built("l0", &calls)
    };
    let d = compiled((&s, &b, &table(2315)), (&s, &b, &table(2316)));
    assert_eq!(d.verdict, Verdict::Additive);
    assert!(undeclared(&d).is_empty());
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].id, "call:Hooks::k2315");
    assert_eq!(d.changes[0].class, Class::Additive);
    assert_eq!(
        d.changes[0].what,
        ["call `Hooks::k2315` is new in the compiled table"]
    );
}
