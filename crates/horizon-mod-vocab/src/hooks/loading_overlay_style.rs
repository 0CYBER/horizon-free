//! The loading overlay hook declaration read by the mod API generator.

pub trait Hooks {
    /// Edit `style` before the branded loading overlay paints. `phase` distinguishes startup,
    /// resource reload and the preview requested by a client mod; `progress` is in `0..=1`.
    /// The same painter draws the background, progress bar and two logo halves in all three
    /// phases. The earliest unbranded boot frame and world/chunk loading screens are separate.
    /// Not on wasm: the branded boot renderer and preview are native client services.
    #[hook(side = "client", contract = "listen")]
    fn loading_overlay_style();
}
