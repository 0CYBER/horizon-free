//! The launcher look's part of `settings.json`: the accent kept per theme, the effects and reduced
//! motion, each dropped alone when it isn't one, and an old light appearance moved to the Taiga
//! theme.
use super::{Appearance, Settings};
use serde_json::{Map, Value};

/// Drops the look's fields that aren't what they say, each alone, before the rest is read.
pub(super) fn tidy(fields: &mut Map<String, Value>) {
    // An accent per theme that isn't one is dropped alone; a map that isn't one, whole.
    match fields.get_mut("theme_accents") {
        Some(Value::Object(accents)) => accents
            .retain(|_, a| serde_json::from_value::<crate::themes::Accent>(a.clone()).is_ok()),
        Some(_) => {
            fields.remove("theme_accents");
        }
        None => {}
    }
    // Effects keep their flags that are true or false; one that isn't stays on.
    match fields.get_mut("effects") {
        Some(Value::Object(flags)) => flags.retain(|_, v| v.is_boolean()),
        Some(_) => {
            fields.remove("effects");
        }
        None => {}
    }
    if fields
        .get("reduce_motion")
        .is_some_and(|v| !v.is_boolean() && !v.is_null())
    {
        fields.remove("reduce_motion");
    }
}

/// A theme is one look now: the light appearance became the Taiga theme, and dark or the system's
/// opens on the saved theme. Saved back as dark, so this runs once.
pub(super) fn settle(settings: &mut Settings) {
    if settings.appearance == Appearance::Light {
        settings.theme = Some(crate::themes::TAIGA.id.to_string());
        settings.accent =
            crate::themes::starting_accent(settings.theme.as_deref(), &settings.theme_accents);
    }
    settings.appearance = Appearance::Dark;
}

#[cfg(test)]
mod tests {
    use super::Settings;

    /// A wrong accent under one theme, or a motion choice that isn't one, loses only itself.
    #[test]
    fn theme_accents_and_motion_survive_bad_entries() {
        let s = Settings::parse(
            r##"{"theme_accents": {"sunrise": "#c2366f", "nord": 12, "horizon": "teal"},
                "reduce_motion": "yes", "game_args": "--x"}"##,
        );
        assert_eq!(s.game_args, "--x");
        assert_eq!(s.reduce_motion, None);
        assert_eq!(s.theme_accents.len(), 2);
        assert_eq!(s.theme_accents["horizon"], crate::themes::Accent::TEAL);
        let s = Settings::parse(r#"{"theme_accents": [1], "reduce_motion": true}"#);
        assert!(s.theme_accents.is_empty());
        assert_eq!(s.reduce_motion, Some(true));
        let saved = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!saved.contains("theme_accents") && !saved.contains("reduce_motion"));
    }

    #[test]
    fn effects_keep_their_good_flags_and_a_bad_effects_loses_nothing_else() {
        let s =
            Settings::parse(r#"{"effects": {"glow": false, "sheen": "no"}, "game_args": "--x"}"#);
        assert_eq!(s.game_args, "--x");
        assert!(!s.effects.glow && s.effects.gradients && s.effects.sheen);
        let s = Settings::parse(r#"{"effects": 3, "game_args": "--x"}"#);
        assert_eq!(
            (s.game_args.as_str(), s.effects),
            ("--x", crate::effects::Effects::ALL)
        );
    }
}
