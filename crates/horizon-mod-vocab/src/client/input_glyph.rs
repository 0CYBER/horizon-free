//! The active client's resolved control prompts.

/// A physical controller button, named by position rather than printed letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ControllerButton {
    /// Bottom face button.
    South,
    /// Right face button.
    East,
    /// Left face button.
    West,
    /// Top face button.
    North,
    /// Left shoulder bumper.
    LeftBumper,
    /// Right shoulder bumper.
    RightBumper,
    /// Left trigger.
    LeftTrigger,
    /// Right trigger.
    RightTrigger,
    /// Select, View, Create or Minus.
    Select,
    /// Start, Menu, Options or Plus.
    Start,
    /// Left stick click (not stick tilt).
    LeftStick,
    /// Right stick click (not stick tilt).
    RightStick,
    /// Up on the directional pad.
    DPadUp,
    /// Down on the directional pad.
    DPadDown,
    /// Left on the directional pad.
    DPadLeft,
    /// Right on the directional pad.
    DPadRight,
}

/// The artwork family selected for a controller prompt, including the player's override.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ControllerGlyphFamily {
    /// Xbox-style lettering (also the default for an unknown family).
    Xbox,
    /// PlayStation symbols and labels.
    PlayStation,
    /// Nintendo lettering.
    Nintendo,
}

/// A resolved control prompt. Read it again when drawing a new frame to follow settings, device,
/// language and resource changes. It contains no atlas names or coordinates.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct InputGlyph {
    pub(crate) label: String,
    pub(crate) size: [f32; 2],
    pub(crate) controller: Option<(ControllerGlyphFamily, ControllerButton)>,
}

impl InputGlyph {
    /// The localized key/chord label or the print on the controller button.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The prompt's layout width and height in GUI pixels, excluding transparent artwork margins.
    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    /// The artwork family and button position it depicts, or `None` for a keyboard/mouse prompt.
    pub fn controller(&self) -> Option<(ControllerGlyphFamily, ControllerButton)> {
        self.controller
    }
}
