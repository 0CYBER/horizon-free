use super::*;

fn fixture() -> (Catalog, Manifest) {
    let catalog = Catalog {
        points: serde_json::from_value(serde_json::json!([{
            "name": "tick", "target": "game::tick", "at_return": true,
            "returns": "cancel", "return_ty": "()", "host": "host"
        }]))
        .unwrap(),
        internals: serde_json::from_value(serde_json::json!([{
            "path": "game::tick", "kind": "fn", "signature": "fn tick()", "hash": "pin"
        }]))
        .unwrap(),
        ..Default::default()
    };
    let manifest = Manifest::parse("id = 'addon'\nname = 'Addon'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '0000000000000000000000000000000000000000'\nmixins = '0000000000000000000000000000000000000000'\n[[inject]]\nid = 'before'\nat = 'game::tick'\nkind = 'head'\nwith = 'before'\nhash = 'pin'\n").unwrap();
    (catalog, manifest)
}

#[test]
fn validates_api_places_and_signature_before_codegen() {
    let (mut catalog, mut manifest) = fixture();
    check(&catalog, &manifest).unwrap();
    manifest.inject[0].kind = Kind::Replace;
    check(&catalog, &manifest).unwrap();
    catalog.points[0].return_ty = Some("Result<(), Error>".into());
    assert!(check(&catalog, &manifest)
        .unwrap_err()
        .contains("Result<(), Error>"));
    manifest.inject[0].kind = Kind::Return;
    catalog.points[0].at_return = false;
    assert!(check(&catalog, &manifest)
        .unwrap_err()
        .contains("no API return"));
    manifest.inject[0].hash = Some("old".into());
    let error = check(&catalog, &manifest).unwrap_err();
    assert!(error.contains("addon:before") && error.contains("old") && error.contains("pin"));
    catalog.points.clear();
    assert!(check(&catalog, &manifest)
        .unwrap_err()
        .contains("E_INJECT_MISSING"));
}

#[test]
fn native_registration_refusal_names_mod_target_and_type() {
    let (mut catalog, mut manifest) = fixture();
    catalog.points[0].target = "listener_fixture::no_wire".into();
    catalog.points[0].api_refusal =
        Some("argument 1 `value` (`&RegistrationData`) has no API type yet".into());
    catalog.internals[0].path = "listener_fixture::no_wire".into();
    manifest.inject[0].at = "listener_fixture::no_wire".into();
    assert_eq!(check(&catalog, &manifest).unwrap_err(), "addon:before: no API listener at listener_fixture::no_wire: argument 1 `value` (`&RegistrationData`) has no API type yet");
}
