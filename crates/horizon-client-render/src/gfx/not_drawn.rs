//! Why the renderer's last window frame was not drawn (#19436): the control socket's `status`
//! and a control screenshot that waited too long name it, where nothing else says why frames
//! stopped.

/// The way out of a frame that was refused before it was drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotDrawn {
    /// The window is zero-sized (minimised) or too large to draw.
    Suspended,
    /// No room for a target the frame draws into (#13624).
    NoRoom,
    /// The surface could not be configured again.
    Unconfigured,
    /// The swapchain gave no image in time.
    AcquireTimeout,
    /// The window is occluded.
    Occluded,
    /// The swapchain was outdated and is configured again.
    Outdated,
    /// The surface was lost and is made again.
    Lost,
    /// Acquiring failed validation; the swapchain is configured or made again.
    Validation,
    /// Given up after it took its swapchain image (#13624, `abandon_frame`).
    GivenUp,
    /// A scaled screenshot's frame ran out of video memory; tried again smaller.
    ScreenshotNoMemory,
    /// A scaled screenshot's frame, drawn into a texture of its own: the window kept its last.
    ScreenshotOnly,
    /// A refusal without a reason of its own.
    Other,
}

impl NotDrawn {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suspended => "suspended",
            Self::NoRoom => "no_room",
            Self::Unconfigured => "surface_unconfigured",
            Self::AcquireTimeout => "acquire_timeout",
            Self::Occluded => "occluded",
            Self::Outdated => "outdated",
            Self::Lost => "lost",
            Self::Validation => "acquire_validation",
            Self::GivenUp => "given_up",
            Self::ScreenshotNoMemory => "screenshot_no_memory",
            Self::ScreenshotOnly => "screenshot_only",
            Self::Other => "other",
        }
    }

    /// The reason after a frame: none once one reached the window, the exit's when it was
    /// refused (an exit that named none reads `Other`, never a stale reason), `ScreenshotOnly`
    /// when a scaled screenshot drew its own texture instead, unchanged after an error.
    pub(super) fn settle<E>(
        last: Option<Self>,
        refusal: Option<Self>,
        rendered: &Result<bool, E>,
        scaled: bool,
    ) -> Option<Self> {
        match rendered {
            Ok(true) if scaled => Some(Self::ScreenshotOnly),
            Ok(true) => None,
            Ok(false) => Some(refusal.unwrap_or(Self::Other)),
            Err(_) => last,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NotDrawn;

    #[test]
    fn a_drawn_frame_clears_the_reason_a_refused_one_sets_it() {
        let refused: Result<bool, ()> = Ok(false);
        let drawn: Result<bool, ()> = Ok(true);
        let last = NotDrawn::settle(None, Some(NotDrawn::AcquireTimeout), &refused, false);
        assert_eq!(last, Some(NotDrawn::AcquireTimeout));
        // A refusal whose exit named nothing is not read as the previous one.
        assert_eq!(
            NotDrawn::settle(last, None, &refused, false),
            Some(NotDrawn::Other)
        );
        assert_eq!(NotDrawn::settle(last, None, &Err(()), false), last);
        // A scaled screenshot drawn into its own texture showed the window nothing.
        assert_eq!(
            NotDrawn::settle(last, None, &drawn, true),
            Some(NotDrawn::ScreenshotOnly)
        );
        assert_eq!(NotDrawn::settle(last, None, &drawn, false), None);
    }
}
