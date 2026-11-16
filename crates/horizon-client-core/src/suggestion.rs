pub type Range = (usize, usize);
/// A completion: replace `range` of the input with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub range: Range,
    pub text: String,
    pub tooltip: Option<String>,
    /// IntegerSuggestion: sorted numerically against other integers.
    pub int_value: Option<i32>,
}
impl Suggestion {
    /// Brigadier's suggestion apply.
    pub fn apply(&self, input: &str) -> String {
        let (s, e) = (self.range.0.min(input.len()), self.range.1.min(input.len()));
        format!("{}{}{}", &input[..s], self.text, &input[e..])
    }
}
