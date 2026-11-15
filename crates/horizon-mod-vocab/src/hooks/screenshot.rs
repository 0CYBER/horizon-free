//! The `screenshot` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An F2 screenshot has been read back, before either PNG is written
    /// (`ScreenshotEvent`). Runs on the client thread with a `ClientHost`.
    /// `image` is the in-memory frame (`getImage`): RGBA8, top to bottom,
    /// `width * height * 4` bytes. Change it and the normal PNG is that image.
    /// A length other than `width * height * 4` writes nothing and reports
    /// screenshot failure. The HDR copy is not this image. Change `file_name`,
    /// a relative path inside the player's screenshots folder, or set `message`
    /// to replace the success chat line. `None` uses the normal translated,
    /// clickable line for the final file name. Return `Flow::Cancel` to write
    /// no files and post `message`, or the translated cancellation line when
    /// `message` is `None`. Invalid paths (including symlinks) report screenshot
    /// failure; parent folders are created. HDR copies follow the final name
    /// with `-hdr.png`. Agent captures and world icons do not fire this hook.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "ScreenshotEvent",
        cancellable = true
    )]
    fn screenshot();
}
