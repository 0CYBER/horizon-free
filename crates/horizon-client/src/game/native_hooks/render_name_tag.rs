use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Name tag visibility and optional replacement text. API 372.
    pub(in crate::game) fn native_render_name_tag(
        &self,
        entity_id: u32,
        kind: &str,
        vanilla_visible: bool,
        visible: &mut bool,
        text: &mut Option<String>,
    ) {
        self.fire_render_name_tag(entity_id, kind, vanilla_visible, visible, text);
    }
}
