//! Compile the production neighbor bridge against observable table/index seams.
use quote::ToTokens;
use syn::visit_mut::VisitMut;

#[test]
#[ignore = "daily"]
fn neighbor_source_identity_and_early_routing() {
    let source = include_str!("../../horizon-client/src/game/blocks/level_access.rs");
    let file = syn::parse_file(source).unwrap();
    let method = file
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) => Some(item.items),
            _ => None,
        })
        .flatten()
        .find_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == "observe_neighbor_changed" => {
                Some(method)
            }
            _ => None,
        })
        .unwrap();
    let mut wrong_source = method.clone();
    struct WrongSource;
    impl VisitMut for WrongSource {
        fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
            syn::visit_mut::visit_expr_method_call_mut(self, call);
            if call.method == "default_state" {
                call.args[0] = syn::parse_quote!(BlockId(1));
            }
        }
    }
    WrongSource.visit_impl_item_fn_mut(&mut wrong_source);
    assert_ne!(
        method.to_token_stream().to_string(),
        wrong_source.to_token_stream().to_string()
    );
    let mut unguarded = method.clone();
    struct Unguarded;
    impl VisitMut for Unguarded {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if matches!(expression, syn::Expr::Call(call)
                if call.func.to_token_stream().to_string().ends_with("block_listens"))
            {
                *expression = syn::parse_quote!(true);
            } else {
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
    }
    Unguarded.visit_impl_item_fn_mut(&mut unguarded);
    assert_ne!(
        method.to_token_stream().to_string(),
        unguarded.to_token_stream().to_string()
    );
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/neighbor-source-proof")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&target).unwrap();
    for (name, method, passes, failure) in [
        ("baseline", method, true, ""),
        (
            "wrong_source",
            wrong_source,
            false,
            "changed source identity",
        ),
        (
            "unguarded",
            unguarded,
            false,
            "secondary lookup before routing rejection",
        ),
    ] {
        let program = FIXTURE.replace("/* METHOD */", &method.to_token_stream().to_string());
        let input = target.join(format!("{name}.rs"));
        let binary = target.join(name);
        std::fs::write(&input, program).unwrap();
        let compile = std::process::Command::new("rustc")
            .args(["--edition=2024", "-Awarnings"])
            .arg(&input)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{name} failed to compile: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let result = std::process::Command::new(binary).output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.success(), passes, "{name}: {stderr}");
        if !passes {
            assert!(
                stderr.contains(failure),
                "{name} failed for the wrong reason: {stderr}"
            );
        }
        println!(
            "{name}: {}",
            if passes {
                "passed"
            } else {
                "assertion mutant killed"
            }
        );
    }
}

const FIXTURE: &str = r#"
use std::cell::{Cell, RefCell};
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct BlockStateId(u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct BlockId(u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct BlockKey(u32);
#[derive(Clone, Copy)] struct ApiState(BlockKey, u16);
impl ApiState { fn block(self) -> BlockKey { self.0 } }
struct Ticks;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct LevelDimension(u8);
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct BlockPos([i32; 3]);
mod mod_api { #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum HookId { NeighborChanged } }
thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static AVAILABLE: Cell<bool> = const { Cell::new(true) };
    static TABLE: Cell<usize> = const { Cell::new(0) };
    static INDEX: Cell<usize> = const { Cell::new(0) };
    static CALLS: RefCell<Vec<(LevelDimension, BlockPos, BlockStateId, BlockKey, bool)>> = const { RefCell::new(Vec::new()) };
}
struct Table;
impl Table {
    fn default_state(&self, block: BlockId) -> BlockStateId {
        TABLE.set(TABLE.get() + 1);
        BlockStateId(match block.0 { 1 => 5, 2 => 17, _ => 99 })
    }
}
struct Behaviours { table: Table }
mod horizon_world { pub(crate) mod level { pub(crate) use crate::BlockId; } }
mod game {
    use super::*;
    pub(super) mod api_values {
        use super::*;
        pub(crate) mod block_state {
            use super::*;
            pub(crate) fn block_listens(hook: mod_api::HookId, state: BlockStateId) -> bool {
                assert_eq!(hook, mod_api::HookId::NeighborChanged);
                ENABLED.get() && state == BlockStateId(5)
            }
        }
        pub(crate) trait TryIntoApi { fn try_into_api(&self) -> Result<ApiState, ()>; }
        impl TryIntoApi for BlockStateId {
            fn try_into_api(&self) -> Result<ApiState, ()> {
                INDEX.set(INDEX.get() + 1);
                if !AVAILABLE.get() { return Err(()); }
                match self.0 {
                    5 => Ok(ApiState(BlockKey(41), 0)),
                    17 => Ok(ApiState(BlockKey(93), 7)),
                    _ => Err(()),
                }
            }
        }
    }
    pub(super) struct GameLevel<'a> { behaviours: &'a Behaviours, native_authoritative: bool, dimension: LevelDimension }
    impl GameLevel<'_> {
        /* METHOD */
        fn native_neighbor_changed(&self, _: &mut Ticks, dimension: LevelDimension, pos: BlockPos, state: BlockStateId, changed: BlockKey, moved: bool) {
            CALLS.with_borrow_mut(|calls| calls.push((dimension, pos, state, changed, moved)));
        }
    }
    pub(super) fn run() {
        let behaviours = Behaviours { table: Table };
        let dimension = LevelDimension(3);
        let mut level = GameLevel { behaviours: &behaviours, native_authoritative: false, dimension };
        let mut ticks = Ticks;
        let pos = BlockPos([4, 65, -9]);
        level.observe_neighbor_changed(&mut ticks, pos, BlockStateId(5), BlockId(2), false);
        assert_eq!(TABLE.get(), 0, "non-authoritative observation must not resolve a source");
        assert_eq!(INDEX.get(), 0);
        assert!(CALLS.with_borrow(|calls| calls.is_empty()));
        level.native_authoritative = true;
        // A different primary block must not even consult the secondary table/index.
        level.observe_neighbor_changed(&mut ticks, pos, BlockStateId(8), BlockId(2), false);
        assert_eq!(TABLE.get(), 0, "secondary lookup before routing rejection");
        assert_eq!(INDEX.get(), 0);
        assert!(CALLS.with_borrow(|calls| calls.is_empty()));
        ENABLED.set(false);
        level.observe_neighbor_changed(&mut ticks, pos, BlockStateId(5), BlockId(2), false);
        assert_eq!(TABLE.get(), 0, "secondary lookup before routing rejection");
        assert_eq!(INDEX.get(), 0);
        assert!(CALLS.with_borrow(|calls| calls.is_empty()));
        ENABLED.set(true);
        level.observe_neighbor_changed(&mut ticks, pos, BlockStateId(5), BlockId(2), true);
        assert_eq!(TABLE.get(), 1);
        assert_eq!(INDEX.get(), 1);
        CALLS.with_borrow(|calls| assert_eq!(calls.as_slice(), &[(dimension, pos, BlockStateId(5), BlockKey(93), true)], "changed source identity"));
        // Unknown default state and unavailable index refuse without callbacks.
        level.observe_neighbor_changed(&mut ticks, pos, BlockStateId(5), BlockId(3), false);
        AVAILABLE.set(false);
        level.observe_neighbor_changed(&mut ticks, pos, BlockStateId(5), BlockId(2), false);
        assert_eq!(TABLE.get(), 3);
        assert_eq!(INDEX.get(), 3);
        CALLS.with_borrow(|calls| assert_eq!(calls.len(), 1));
    }
}
fn main() { game::run(); }
"#;
