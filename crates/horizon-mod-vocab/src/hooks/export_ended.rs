//! The `export_ended` hook declaration read by the mod API generator.

pub trait Hooks {
    /// A mod-started video export ended. `frames` counts complete output frames.
    /// Heard once after `export_status` is terminal and `export.json` has been published
    /// or its publication has failed, been cancelled, or exceeded its deadline. A timed-out
    /// rename may still complete. A graceful stop reports `Finished`; an interrupted export
    /// on world exit or disconnect reports `Failed`.
    /// Starting another export from this hook returns `Busy`.
    ///
    /// The event's fields are fixed on the wire (docs/modding/src/concepts/wire-types.md).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon's video export)"
    )]
    fn export_ended();
}
