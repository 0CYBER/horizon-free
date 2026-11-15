//! A mob's custom name, saved whole when it is styled.
use super::*;

/// `Bucketable.saveDefaultDataToBucketTag`: `copyFrom(CUSTOM_NAME)`, whole.
pub(super) fn bucket_custom_name(m: &Mob) -> Option<Tag> {
    m.custom_name_tag()
}

impl Mob {
    /// `Entity.getCustomName`: the whole text, style included, else the plain name.
    pub fn display_custom_name(&self) -> Option<crate::text::Text> {
        let plain = || self.custom_name.as_deref().map(crate::text::Text::literal);
        self.custom_name_text.clone().or_else(plain)
    }
}
