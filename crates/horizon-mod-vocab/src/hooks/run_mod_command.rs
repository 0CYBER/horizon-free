//! The `run_mod_command` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0033

pub trait Hooks {
    /// Execute a parsed mod command after permission checks and argument resolution.
    /// Return `Flow::Cancel` when handled. `source` identifies origin and permission,
    /// independent of `execute as`. Feedback honours silence, track-output and the swapped-in player.
    #[notes = r#"## Values of the position, rotation, dimension and tag kinds

- `vec3` and `vec3_uncentered` are `CommandArgumentValue::Vec3`, `column_pos` is `ColumnPos`, `rotation` is `Rotation` (yaw, pitch), `dimension` is `Dimension(LevelRef)` and `nbt` is `Nbt(Compound)`. `~` and `^` resolve against the source's position and rotation. A `dimension` the world has not loaded is refused with `argument.dimension.invalid` before a listener runs."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "ran",
        neoforge = "none (a command's executor is part of the Brigadier node added in RegisterCommandsEvent; CommandEvent only watches executions)"
    )]
    fn run_mod_command();
}
