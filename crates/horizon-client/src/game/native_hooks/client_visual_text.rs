#[cfg(test)]
#[path = "client_visual_text/tests.rs"]
mod tests;
use crate::game::{Game, api_values::TryIntoApi};

impl Game<'_> {
    pub(in crate::game) fn text_visual_order(
        &self,
        text: &horizon_client::text::Text,
    ) -> Result<Vec<mod_api::VisualTextCharacter>, mod_api::TextFormError> {
        let resolved = text.resolved();
        horizon_client::bidi::visual_characters(&resolved)
            .into_iter()
            .map(|(scalar, style, source_index)| {
                let style = style.try_into_api()?;
                Ok(mod_api::VisualTextCharacter {
                    scalar: scalar as u32,
                    style,
                    source_index,
                })
            })
            .collect()
    }
}
