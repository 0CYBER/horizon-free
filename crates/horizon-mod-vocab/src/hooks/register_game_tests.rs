//! Registration of in-world test functions.
pub trait Hooks {
    /// Register namespaced test functions before the world's test definitions load.
    /// Each function runs with a ServerHost once per attempt in the real test level.
    /// The runner retains ownership of retries, timeout and completion.
    #[notes = r#"## In-world game tests

`register_game_tests` runs once per world open before test-instance definitions resolve their functions.
Register an `Arc<dyn GameTestFunction>` through `event.functions.register("example:check", callback)`.
Add a test with `event.functions.register_test(id, GameTestDefinition::new(structure, function))`,
or use a data-pack `test_instance` with type `minecraft:function` and function `example:check` selects it.
The callback receives a `ServerHost` and `GameTestContext`: use `context.level` for level handles and
`context.absolute_pos(relative)` for rotated structure coordinates. Return `Pass`, `Fail(message)` or
`Pending`; pending attempts keep the ordinary timeout. Each retry invokes the function again.
Definitions retain resolved callbacks only for their world snapshot. Unknown functions fail normally;
`minecraft:always_pass` remains built in. Data-pack test definitions override code-registered defaults. Registration refuses duplicates, invalid names and the vanilla
namespace. This adds no sequence engine and no per-tick registry lookup."#]
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "RegisterGameTestsEvent"
    )]
    fn register_game_tests();
}
