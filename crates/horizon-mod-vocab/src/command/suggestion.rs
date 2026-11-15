/// A command completion replacing a byte range of the input.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Suggestion {
    /// Start (inclusive) and end (exclusive) byte offsets.
    pub range: (usize, usize),
    /// Replacement text.
    pub text: String,
    /// Optional explanation of the completion.
    pub tooltip: Option<String>,
    /// Numeric sort value for integer completions.
    pub int_value: Option<i32>,
}
impl Suggestion {
    /// Construct a completion without a tooltip or numeric sort value.
    pub fn new(range: (usize, usize), text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
            tooltip: None,
            int_value: None,
        }
    }
}
