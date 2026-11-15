//! Settings a mod declares for its page in Options > Mods (NeoForge's `ModConfigSpec`): the
//! game draws them with its own widgets and keeps the player's values; the mod reads them typed
//! (`ConfigClientHost::setting_bool` and its siblings), never as text it has to parse.

/// Longest setting key, in bytes.
pub const SETTING_KEY_MAX: usize = 64;

/// What a setting is, with its default.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SettingKind {
    /// On or off.
    Toggle {
        /// The value before the player changes it.
        default: bool,
    },
    /// One of `options`, by index; each option is a translation key.
    Choice {
        /// The options' translation keys, in the order the button cycles through them.
        options: Vec<String>,
        /// The index picked before the player changes it.
        default: usize,
    },
    /// A number from `min` to `max` in steps of `step`, drawn as a slider.
    Range {
        /// The smallest value.
        min: f64,
        /// The largest value.
        max: f64,
        /// The distance between two values the slider stops at.
        step: f64,
        /// The value before the player changes it.
        default: f64,
    },
    /// A colour, `0xRRGGBB`, without alpha.
    Color {
        /// The colour before the player changes it.
        default: u32,
    },
}

/// A setting's value, as the game keeps it and the mod reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SettingValue {
    /// A toggle's state.
    Toggle(bool),
    /// A choice's index.
    Choice(usize),
    /// A range's number.
    Range(f64),
    /// A colour, `0xRRGGBB`.
    Color(u32),
}

/// A setting a mod registers with `Registrar::register_setting`: its key (unique within the
/// mod), the translation keys of its label and tooltip, and what it is.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SettingSpec {
    /// The setting's key within its mod (`zoom_speed`): `a-z`, `0-9`, `_`, `-` and `.`, at most
    /// [`SETTING_KEY_MAX`] bytes, not starting with `.`.
    pub key: String,
    /// The translation key of the label its row shows.
    pub label: String,
    /// The translation key of the tooltip its row shows, if any.
    pub tooltip: Option<String>,
    /// What it is, with its default.
    pub kind: SettingKind,
}

/// Why a setting was refused at registration.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SettingError {
    /// The key is empty, too long, starts with `.` or holds another character than `a-z`,
    /// `0-9`, `_`, `-` and `.`.
    InvalidKey(String),
    /// The mod registered a setting with this key already.
    Duplicate(String),
    /// The label (or an option of a choice) is empty.
    EmptyLabel,
    /// A choice has no options, or its default is not one of them.
    InvalidChoice,
    /// A range's `min` is not below its `max`, or a bound is not finite.
    EmptyRange,
    /// A range's `step` is zero, negative, not finite or wider than the range.
    InvalidStep,
    /// A range's default is outside `min..=max`.
    DefaultOutOfRange,
    /// A colour holds alpha or more than 24 bits.
    ColorAlpha,
}

impl std::fmt::Display for SettingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(key) => write!(f, "invalid setting key {key:?}"),
            Self::Duplicate(key) => write!(f, "setting {key:?} is registered twice"),
            Self::EmptyLabel => f.write_str("a setting's label or option is empty"),
            Self::InvalidChoice => f.write_str("a choice needs options and a default among them"),
            Self::EmptyRange => f.write_str("a range needs finite bounds with min below max"),
            Self::InvalidStep => f.write_str("a range's step must be above zero and fit in it"),
            Self::DefaultOutOfRange => f.write_str("a range's default is outside min..=max"),
            Self::ColorAlpha => f.write_str("a colour is 0xRRGGBB, without alpha"),
        }
    }
}

impl std::error::Error for SettingError {}

impl SettingSpec {
    fn new(key: impl Into<String>, label: impl Into<String>, kind: SettingKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            tooltip: None,
            kind,
        }
    }

    /// An on/off setting.
    pub fn toggle(key: impl Into<String>, label: impl Into<String>, default: bool) -> Self {
        Self::new(key, label, SettingKind::Toggle { default })
    }

    /// One of `options` (translation keys), `default` being an index into them.
    pub fn choice<S: Into<String>>(
        key: impl Into<String>,
        label: impl Into<String>,
        options: impl IntoIterator<Item = S>,
        default: usize,
    ) -> Self {
        let options = options.into_iter().map(Into::into).collect();
        Self::new(key, label, SettingKind::Choice { options, default })
    }

    /// A number from `min` to `max` in steps of `step`.
    pub fn range(
        key: impl Into<String>,
        label: impl Into<String>,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
    ) -> Self {
        let kind = SettingKind::Range {
            min,
            max,
            step,
            default,
        };
        Self::new(key, label, kind)
    }

    /// A colour, `0xRRGGBB`.
    pub fn color(key: impl Into<String>, label: impl Into<String>, default: u32) -> Self {
        Self::new(key, label, SettingKind::Color { default })
    }

    /// Sets the tooltip's translation key.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Whether the registrar takes it: the key's form, a label, and a kind whose default fits.
    pub fn check(&self) -> Result<(), SettingError> {
        let key_ok = !self.key.is_empty()
            && self.key.len() <= SETTING_KEY_MAX
            && !self.key.starts_with('.')
            && self
                .key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b));
        if !key_ok {
            return Err(SettingError::InvalidKey(self.key.clone()));
        }
        if self.label.is_empty() {
            return Err(SettingError::EmptyLabel);
        }
        match &self.kind {
            SettingKind::Toggle { .. } => Ok(()),
            SettingKind::Choice { options, default } => {
                if options.iter().any(String::is_empty) {
                    Err(SettingError::EmptyLabel)
                } else if *default >= options.len() {
                    Err(SettingError::InvalidChoice)
                } else {
                    Ok(())
                }
            }
            SettingKind::Range {
                min,
                max,
                step,
                default,
            } => {
                if !(min.is_finite() && max.is_finite() && min < max) {
                    Err(SettingError::EmptyRange)
                } else if !(step.is_finite() && *step > 0.0 && *step <= max - min) {
                    Err(SettingError::InvalidStep)
                } else if !(min..=max).contains(&default) {
                    Err(SettingError::DefaultOutOfRange)
                } else {
                    Ok(())
                }
            }
            SettingKind::Color { default } if *default > 0xFF_FFFF => Err(SettingError::ColorAlpha),
            SettingKind::Color { .. } => Ok(()),
        }
    }

    /// The default value.
    pub fn default_value(&self) -> SettingValue {
        match &self.kind {
            SettingKind::Toggle { default } => SettingValue::Toggle(*default),
            SettingKind::Choice { default, .. } => SettingValue::Choice(*default),
            SettingKind::Range { default, .. } => SettingValue::Range(*default),
            SettingKind::Color { default } => SettingValue::Color(*default),
        }
    }

    /// The value kept as `stored` (see [`SettingSpec::store`]), or the default when nothing is
    /// kept, or when what is kept isn't a value of this setting (another kind, out of range).
    pub fn read(&self, stored: Option<&str>) -> SettingValue {
        let stored = stored.map(str::trim);
        let value = match (&self.kind, stored) {
            (SettingKind::Toggle { .. }, Some("true")) => Some(SettingValue::Toggle(true)),
            (SettingKind::Toggle { .. }, Some("false")) => Some(SettingValue::Toggle(false)),
            (SettingKind::Choice { options, .. }, Some(text)) => text
                .parse::<usize>()
                .ok()
                .filter(|index| *index < options.len())
                .map(SettingValue::Choice),
            (SettingKind::Range { min, max, .. }, Some(text)) => text
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && (*min..=*max).contains(v))
                .map(SettingValue::Range),
            (SettingKind::Color { .. }, Some(text)) => text
                .strip_prefix('#')
                .filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .map(SettingValue::Color),
            _ => None,
        };
        value.unwrap_or_else(|| self.default_value())
    }

    /// How `value` is kept: the text [`SettingSpec::read`] gives back. None when `value` isn't
    /// one of this setting's (another kind, out of range).
    pub fn store(&self, value: SettingValue) -> Option<String> {
        let text = match (&self.kind, value) {
            (SettingKind::Toggle { .. }, SettingValue::Toggle(on)) => on.to_string(),
            (SettingKind::Choice { options, .. }, SettingValue::Choice(index))
                if index < options.len() =>
            {
                index.to_string()
            }
            (SettingKind::Range { min, max, .. }, SettingValue::Range(v))
                if v.is_finite() && (*min..=*max).contains(&v) =>
            {
                v.to_string()
            }
            (SettingKind::Color { .. }, SettingValue::Color(rgb)) if rgb <= 0xFF_FFFF => {
                format!("#{rgb:06x}")
            }
            _ => return None,
        };
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_that_cannot_work_is_refused_with_its_reason() {
        let range =
            |min, max, step, default| SettingSpec::range("speed", "l", min, max, step, default);
        assert_eq!(
            range(1.0, 1.0, 0.1, 1.0).check(),
            Err(SettingError::EmptyRange)
        );
        assert_eq!(
            range(2.0, 1.0, 0.1, 1.5).check(),
            Err(SettingError::EmptyRange)
        );
        assert_eq!(
            range(0.0, f64::INFINITY, 1.0, 0.0).check(),
            Err(SettingError::EmptyRange)
        );
        assert_eq!(
            range(0.0, 1.0, 0.0, 0.5).check(),
            Err(SettingError::InvalidStep)
        );
        assert_eq!(
            range(0.0, 1.0, -0.5, 0.5).check(),
            Err(SettingError::InvalidStep)
        );
        assert_eq!(
            range(0.0, 1.0, 5.0, 0.5).check(),
            Err(SettingError::InvalidStep)
        );
        assert_eq!(range(0.0, 1.0, 1.0, 0.5).check(), Ok(()));
        assert_eq!(
            range(0.0, 1.0, 0.1, 1.5).check(),
            Err(SettingError::DefaultOutOfRange)
        );
        assert_eq!(range(0.0, 1.0, 0.1, 1.0).check(), Ok(()));
        assert_eq!(
            SettingSpec::color("c", "l", 0xFF00_0000).check(),
            Err(SettingError::ColorAlpha)
        );
        assert_eq!(SettingSpec::color("c", "l", 0xFF_FFFF).check(), Ok(()));
        assert_eq!(
            SettingSpec::choice("m", "l", ["a", "b"], 2).check(),
            Err(SettingError::InvalidChoice)
        );
        assert_eq!(
            SettingSpec::choice("m", "l", Vec::<String>::new(), 0).check(),
            Err(SettingError::InvalidChoice)
        );
        assert_eq!(
            SettingSpec::choice("m", "l", ["a", ""], 0).check(),
            Err(SettingError::EmptyLabel)
        );
        assert_eq!(
            SettingSpec::toggle("on", "", true).check(),
            Err(SettingError::EmptyLabel)
        );
        for key in [
            "",
            ".hidden",
            "../up",
            "Zoom",
            "a/b",
            &"k".repeat(SETTING_KEY_MAX + 1),
        ] {
            let refused = SettingSpec::toggle(key, "l", true).check();
            assert_eq!(
                refused,
                Err(SettingError::InvalidKey(key.to_string())),
                "{key:?}"
            );
        }
        assert_eq!(
            SettingSpec::toggle("zoom_speed-2.x", "l", true).check(),
            Ok(())
        );
    }

    #[test]
    fn a_kept_value_reads_back_and_anything_else_reads_as_the_default() {
        let toggle = SettingSpec::toggle("on", "l", true);
        let choice = SettingSpec::choice("mode", "l", ["a", "b", "c"], 1);
        let range = SettingSpec::range("speed", "l", 0.5, 4.0, 0.5, 1.0);
        let color = SettingSpec::color("tint", "l", 0x33_66CC);
        let values = [
            (&toggle, SettingValue::Toggle(false)),
            (&choice, SettingValue::Choice(2)),
            (&range, SettingValue::Range(2.5)),
            (&color, SettingValue::Color(0xAB_CDEF)),
        ];
        for (spec, value) in values {
            let kept = spec.store(value).expect("a value of the setting is kept");
            assert_eq!(spec.read(Some(&kept)), value);
            assert_eq!(spec.read(None), spec.default_value());
            assert_eq!(spec.read(Some("garbage")), spec.default_value());
        }
        assert_eq!(choice.read(Some("3")), SettingValue::Choice(1));
        assert_eq!(range.read(Some("9.0")), SettingValue::Range(1.0));
        assert_eq!(range.read(Some("NaN")), SettingValue::Range(1.0));
        assert_eq!(color.read(Some("#12345")), SettingValue::Color(0x33_66CC));
        assert_eq!(color.read(Some("#+12345")), SettingValue::Color(0x33_66CC));
        assert_eq!(toggle.store(SettingValue::Choice(0)), None);
        assert_eq!(range.store(SettingValue::Range(4.5)), None);
        assert_eq!(color.store(SettingValue::Color(0x0100_0000)), None);
    }
}
