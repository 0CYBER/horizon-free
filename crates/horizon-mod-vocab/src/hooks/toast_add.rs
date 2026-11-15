//! Client toast admission (`ToastAddEvent`).

pub trait Hooks {
    /// A client toast is about to enter its queue (`ToastAddEvent`). Return
    /// `Flow::Cancel` to discard only this candidate, before slot allocation,
    /// animation or sound. `kind` identifies its family; music is `Other`.
    /// Replace `title` and `description` to change the displayed components.
    /// For advancements the title is the frame label and the description is
    /// the advancement name. Existing recipe/system toasts updated in place
    /// do not fire another add event. Client-lib and title producers prepare
    /// candidates for the game to admit with a `ClientHost` before applying.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "ToastAddEvent",
        cancellable = true
    )]
    fn toast_add();
}
