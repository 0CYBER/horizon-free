//! The `camera_type` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `key.togglePerspective` (F5) is cycling the window's camera type (`Minecraft.handleKeybinds`
    /// calling `Options.setCameraType(getCameraType().cycle())`): `previous` is the type it leaves,
    /// and `next` comes in as vanilla's next one (first person, third person behind, third person
    /// in front, then first person again); set `next` to another, to skip a type or keep one.
    /// Once a press, before the type changes; the post effect of the camera entity follows the
    /// answer as it follows vanilla's. `camera_type` and `set_camera_type` read and set the type
    /// at any other moment. Client-side. Beyond NeoForge: there is no counterpart. Diverges from
    /// upstream: new hook.
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "none (NeoForge has no camera type event)"
    )]
    fn camera_type_cycle();
}
