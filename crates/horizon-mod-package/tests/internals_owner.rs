//! The declaration owner is independent of a code mod's build target.
use horizon_mod_package::{Manifest, manifest::Internal};

const HEADER: &str =
    "id='owner'\nname='Owner'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n";

fn targets() -> [String; 2] {
    [
        format!(
            "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        ),
        format!("[wasm]\nsha512='{}'\ntoolchain='1.99.1'\n", "0".repeat(128)),
    ]
}

#[test]
fn both_targets_read_the_same_root_records_in_both_toml_forms() {
    let expected = vec![
        Internal {
            path: "Round::listening".into(),
            kind: Some("field".into()),
            hash: Some("field-hash".into()),
        },
        Internal {
            path: "Round::listening".into(),
            kind: Some("method".into()),
            hash: Some("method-hash".into()),
        },
        Internal {
            path: "Game".into(),
            kind: None,
            hash: Some("game-hash".into()),
        },
    ];
    for declarations in [
        "internals=[{path='Round::listening',kind='field',hash='field-hash'},{path='Round::listening',kind='method',hash='method-hash'},{path='Game',hash='game-hash'}]\n",
        "[[internals]]\npath='Round::listening'\nkind='field'\nhash='field-hash'\n[[internals]]\npath='Round::listening'\nkind='method'\nhash='method-hash'\n[[internals]]\npath='Game'\nhash='game-hash'\n",
    ] {
        for target in targets() {
            let manifest = Manifest::parse(&format!("{HEADER}{declarations}{target}")).unwrap();
            assert_eq!(manifest.internals, expected);
        }
    }
}

#[test]
fn native_table_declarations_are_unknown_fields_not_an_alternate_reader() {
    for declarations in [
        "internals=[{path='Game',hash='h'}]\n",
        "[[native.internals]]\npath='Game'\nhash='h'\n",
    ] {
        for (index, target) in targets().into_iter().enumerate() {
            let old = if index == 0 {
                format!("{HEADER}{target}{declarations}")
            } else {
                format!("{HEADER}{target}[native]\n{declarations}")
            };
            let error = Manifest::parse(&old).unwrap_err();
            // A wasm manifest is told it has no `[native]` at all before what the table held.
            let expected = if index == 0 {
                "unknown field `internals`"
            } else {
                "a mod is [native] or [wasm], not both"
            };
            assert!(error.contains(expected), "{error}");
        }
    }
}

#[test]
fn root_record_validation_is_identical_for_both_targets() {
    for declarations in [
        "[{path='',hash='h'}]",
        "[{path='Game',hash=''}]",
        "[{path='Game',kind='',hash='h'}]",
        "[{path='Game',hash='a'},{path='Game',hash='b'}]",
        "[{path='Game::field',kind='field',hash='a'},{path='Game::field',kind='field',hash='b'}]",
    ] {
        for target in targets() {
            let error = Manifest::parse(&format!("{HEADER}internals={declarations}\n{target}"))
                .unwrap_err();
            assert!(
                error.contains(
                    "empty internal path, kind or hash, or duplicate internal path and kind"
                ),
                "{error}"
            );
        }
    }
    let error =
        Manifest::parse(&format!("{HEADER}internals=[{{path='Game',hash='h'}}]\n")).unwrap_err();
    assert!(
        error.contains("internals need a [native] or [wasm] mod"),
        "{error}"
    );
}

#[test]
fn native_modules_stay_native() {
    let [native, _wasm] = targets();
    let manifest = Manifest::parse(&format!("{HEADER}{native}[[native.module]]\nid='support'\nfile='src/game.rs'\nname='support'\nsource='src/support.rs'\n")).unwrap();
    assert_eq!(manifest.native.unwrap().module[0].id, "support");
}
