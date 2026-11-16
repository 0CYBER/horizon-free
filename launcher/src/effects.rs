//! The look's flourishes, each switched on or off on its own (Settings > Appearance, and the
//! first-run welcome). Plain data: the backend keeps it in `Settings`; the window reads it as
//! each frame starts and every drawing of a flourish asks for its own flag, so one can go
//! without touching the others.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    /// The accent's halo under primary buttons, chosen tabs and pills, and lifted cards.
    pub glow: bool,
    /// Two-colour fills on primary buttons, pills and tab bars; one flat colour when off.
    pub gradients: bool,
    /// The pass of light across a primary button under the pointer.
    pub sheen: bool,
}

impl Default for Effects {
    fn default() -> Self {
        Self::ALL
    }
}

impl Effects {
    /// Every flourish on: the look as designed.
    pub const ALL: Self = Self {
        glow: true,
        gradients: true,
        sheen: true,
    };
    /// None: flat, quiet controls.
    pub const CALM: Self = Self {
        glow: false,
        gradients: false,
        sheen: false,
    };

    /// The flags, each with its name and what it changes, in the order Settings lists them.
    pub const LIST: [(Flag, &'static str, &'static str); 3] = [
        (
            Flag::Glow,
            "Glow",
            "A soft light in the accent under Play, chosen tabs and lifted cards.",
        ),
        (
            Flag::Gradients,
            "Gradients",
            "Two-tone fills on Play and the chosen tab. Off, they are one flat colour.",
        ),
        (
            Flag::Sheen,
            "Shine on hover",
            "A pass of light across Play when the pointer arrives.",
        ),
    ];

    pub fn get(self, flag: Flag) -> bool {
        match flag {
            Flag::Glow => self.glow,
            Flag::Gradients => self.gradients,
            Flag::Sheen => self.sheen,
        }
    }

    pub fn set(&mut self, flag: Flag, on: bool) {
        match flag {
            Flag::Glow => self.glow = on,
            Flag::Gradients => self.gradients = on,
            Flag::Sheen => self.sheen = on,
        }
    }
}

/// One flourish of [`Effects`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Glow,
    Gradients,
    Sheen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_switch_alone_and_missing_ones_stay_on() {
        let mut e = Effects::ALL;
        e.set(Flag::Gradients, false);
        assert!(e.get(Flag::Glow) && !e.get(Flag::Gradients) && e.get(Flag::Sheen));
        let read: Effects = serde_json::from_str(r#"{"glow": false}"#).unwrap();
        assert_eq!(
            read,
            Effects {
                glow: false,
                ..Effects::ALL
            }
        );
        for (flag, _, _) in Effects::LIST {
            assert!(Effects::ALL.get(flag) && !Effects::CALM.get(flag));
        }
    }
}
