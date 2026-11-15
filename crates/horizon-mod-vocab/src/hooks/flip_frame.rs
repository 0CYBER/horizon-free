//! The `flip_frame` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once for each frame presented, in a world or not, after that frame's
    /// `render_frame_post` (NeoForge's `FlipFrameEvent`): the place to rotate
    /// per-frame GPU data. Not cancellable. A frame that was not presented does
    /// not fire it. Client-side.
    #[notes = r#"## Frame presentation

`render_frame_post` fires every frame once rendering returns, whatever became of
the frame. `flip_frame` fires after it, on the game's thread, and only when the
frame was presented. A frame presented on the game's thread (VSync, the default)
was already presented when `render_frame_post` fired: there the two differ only
on the frames that were skipped.

Nothing fires `flip_frame` for a frame that was skipped: the window is minimized
or has no drawable size, the swapchain gave no image (timeout, occluded, outdated
or lost), the device was lost, or the frame was a scaled screenshot that went to a
file. The closing save screen and the startup loading screens present frames of
their own, and those fire none of the frame hooks.

Where a frame goes decides what "presented" means when the hook runs:

- Presented on the game's thread: the present has returned.
- Presented on the submit thread: the game waits for that present before the hook
  runs. It waits only when a mod listens.
- Frame generation: the frame has been submitted and handed to the present thread,
  which shows it up to half a real frame later and can still fail to. The game does
  not wait, because that would hold back the next frame and undo frame generation.
  A made frame fires nothing of its own: the hook follows the real frames.
- Offscreen, with no swapchain (agent control, a splitscreen view): the frame has
  been submitted to its offscreen target.
"#]
    #[hook(side = "client", contract = "listen", neoforge = "FlipFrameEvent")]
    fn flip_frame();
}
