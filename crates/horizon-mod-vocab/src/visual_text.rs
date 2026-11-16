//! Styled characters obtained from the client's visual text sequence.
use crate::text::TextStyle;

/// One character accepted by vanilla's `FormattedCharSink` in visual order.
// exhaustive: the three values supplied by FormattedCharSink form this record.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct VisualTextCharacter {
    /// Unicode scalar value, including shaping and mirroring applied by the client.
    pub scalar: u32,
    /// Full effective style, including click, hover and font information.
    pub style: TextStyle,
    /// UTF-16 offset within the sink's current directional/style run.
    /// This is not a global component offset; it restarts for each run.
    pub source_index: u32,
}
