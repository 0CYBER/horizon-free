//! Components cross as the API's validated, flat node table, not Rust recursion.
use super::{Reader, Wire, WireError, Writer};
use crate::text::{Text, TextForm};

impl Wire for Text {
    const MIN: usize = TextForm::MIN;
    const LAYOUT_SEED: u64 = TextForm::LAYOUT_SEED;
    fn layout(visiting: &mut Vec<&'static str>) -> u64 {
        TextForm::layout(visiting)
    }
    fn put(&self, out: &mut Writer) {
        match TextForm::try_from(self) {
            Ok(form) => out.put(&form),
            Err(_) => out.fail(WireError::Depth),
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Text::try_from(input.take::<TextForm>()?).map_err(|_| WireError::Invalid("text node table"))
    }
}
