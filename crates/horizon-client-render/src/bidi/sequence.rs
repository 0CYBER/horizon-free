#[cfg(test)]
#[path = "sequence_tests.rs"]
mod tests;
use super::*;
use crate::text::Style;

/// Characters, effective styles and run-local UTF-16 sink offsets, as accepted
/// by `Component.getVisualOrderText()`. Input must already resolve legacy codes.
/// Uses the same shaping and Unicode bidi pass as rendered text.
pub fn visual_characters(text: &Text) -> Vec<(char, &Style, u32)> {
    let chars: Vec<_> = text.chars().map(|(c, _)| c).collect();
    let styles: Vec<_> = text
        .chars()
        .flat_map(|(c, style)| std::iter::repeat_n(style, c.len_utf16()))
        .collect();
    let order = reorder(&chars, crate::lang::bidirectional());
    // Vanilla assigns pre-shaping UTF-16 styles to the shaped string. Shaping
    // may shorten it (lam-alef), but never lengthens it.
    let mut logical = vec![(0usize, 0u8); order.len()];
    for &(c, at, level) in &order {
        logical[at] = (c.len_utf16(), level);
    }
    let mut offsets = Vec::with_capacity(logical.len());
    let mut absolute = 0;
    let mut local = 0;
    let mut previous = None;
    for (length, level) in logical {
        let style = styles[absolute];
        if previous.is_some_and(|(old_style, old_level)| old_style != style || old_level != level) {
            local = 0;
        }
        offsets.push((style, local));
        previous = Some((style, level));
        absolute += length;
        local += length as u32;
    }
    order
        .into_iter()
        .map(|(c, at, _)| {
            let (style, index) = offsets[at];
            (c, style, index)
        })
        .collect()
}
