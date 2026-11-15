use super::*;

#[test]
fn native_build_catalog_requires_a_folder_and_refuses_build_or_sync_options() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from([&["horizon-launcher", "native-build", "--catalog"], args].concat())
    };
    assert!(matches!(
        parse(&["mod", "--trust-native-code"]).unwrap().command,
        Some(Cmd::NativeBuild {
            catalog: true,
            sync_internals: false,
            folder: Some(_),
            ..
        })
    ));
    assert!(parse(&[]).is_err());
    for args in [
        vec!["mod", "--sync-internals"],
        vec!["mod", "--instance", "A"],
        vec!["mod", "--server-output", "out"],
        vec!["mod", "--profile", "developer"],
        vec!["mod", "--version", "v1"],
        vec!["mod", "--update"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}
