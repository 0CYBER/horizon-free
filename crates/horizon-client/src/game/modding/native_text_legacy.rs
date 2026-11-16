//! `text_from_legacy`: legacy `§` formatting codes read as the font draws them.
use super::Game;
use horizon_client::text::Text;

impl Game<'_> {
    /// Legacy formatting codes read as the font draws them.
    pub(super) fn text_from_legacy(&self, legacy: &str) -> Text {
        Text::legacy(legacy)
    }
}

#[cfg(test)]
mod tests {
    use super::Text;
    use crate::game::api_values::TryIntoApi;

    /// Colour codes reset decorations, `§r` resets everything, unknown codes are dropped.
    #[test]
    fn text_from_legacy_reads_section_codes() {
        let text = Text::legacy("§cRed §lbold§r plain§zx");
        assert_eq!(text.plain(), "Red bold plainx");
        let spans: Vec<_> = text.chars().map(|(c, style)| (c, style.clone())).collect();
        assert_eq!(spans[0].1.color, Some(0xFF5555));
        assert!(!spans[0].1.is_bold());
        assert!(spans[4].1.is_bold() && spans[4].1.color == Some(0xFF5555));
        assert_eq!(spans[9].1.color, None);
        let api: mod_api::Text = text.try_into_api().unwrap();
        assert!(api.validate().is_ok());
    }
}
