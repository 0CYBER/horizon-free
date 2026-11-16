use super::boundary_arguments;

#[test]
#[ignore = "daily"]
fn boolean_state_writes_refuse_before_mutation_without_changing_the_bool_contract() {
    let mut methods = String::new();
    for (name, ty, value) in [
        ("scalar", "BlockState", "Some(state)"),
        ("optional", "Option<BlockState>", "state"),
    ] {
        let method: syn::TraitItemFn =
            syn::parse_str(&format!("fn {name}(&mut self, state: {ty}) -> bool;")).unwrap();
        let (setup, _, checked_write) = boundary_arguments(&method.sig, name).unwrap();
        assert!(
            !checked_write,
            "bool-returning writes must not acquire a Result wrapper"
        );
        methods.push_str(&format!("fn {name}(&mut self, state: {ty}) -> bool {{ {setup} self.value = {value}; self.writes += 1; true }}"));
    }
    let source = format!(
        r#"
        struct BlockState(u8);
        trait TryIntoEngine {{ type Output; fn try_into_engine(self, _: &()) -> Result<Self::Output, ()>; }}
        impl TryIntoEngine for BlockState {{
            type Output = u8;
            fn try_into_engine(self, _: &()) -> Result<u8, ()> {{
                if self.0 == 255 {{ Err(()) }} else {{ Ok(self.0) }}
            }}
        }}
        impl<T: TryIntoEngine> TryIntoEngine for Option<T> {{
            type Output = Option<T::Output>;
            fn try_into_engine(self, ctx: &()) -> Result<Self::Output, ()> {{
                self.map(|value| value.try_into_engine(ctx)).transpose()
            }}
        }}
        struct Game {{ value: Option<u8>, writes: usize }}
        impl Game {{ fn items(&self) -> &() {{ &() }} {methods} }}
        fn main() {{
            let mut game = Game {{ value: Some(17), writes: 0 }};
            assert!(!game.scalar(BlockState(255)));
            assert_eq!((game.value, game.writes), (Some(17), 0));
            assert!(!game.optional(Some(BlockState(255))));
            assert_eq!((game.value, game.writes), (Some(17), 0));
            assert!(game.scalar(BlockState(23)));
            assert_eq!((game.value, game.writes), (Some(23), 1));
            assert!(game.optional(None));
            assert_eq!((game.value, game.writes), (None, 2));
            assert!(game.optional(Some(BlockState(31))));
            assert_eq!((game.value, game.writes), (Some(31), 3));
        }}
    "#
    );
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("state-bool-proof-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    let input = target.join("proof.rs");
    let binary = target.join("proof");
    std::fs::write(&input, source).unwrap();
    let compile = std::process::Command::new("rustc")
        .args(["--edition", "2024"])
        .arg(&input)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "generated bool crossing did not compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(binary).output().unwrap();
    assert!(
        run.status.success(),
        "generated refusal changed state or return contract: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}
