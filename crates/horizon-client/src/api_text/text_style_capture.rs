use super::*;

/// Capture a standalone engine style through the same text boundary.
pub fn style_to_api(style: &engine::Style) -> Result<TextStyle, TextFormError> {
    let span = engine::Span::new("", style.clone());
    Ok(capture(Source::Span(&span))?.style)
}
