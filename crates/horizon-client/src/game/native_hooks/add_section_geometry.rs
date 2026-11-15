use super::*;

impl Game<'_> {
    /// Extra geometry in chunk section meshes (NeoForge's `AddSectionGeometryEvent`):
    /// `register(renderer)` adds the mod's own `SectionGeometry`, which the mesher asks to add its
    /// quads every time it meshes a section, on its worker threads. Asked once at start on the
    /// client, before resources load.
    pub(in crate::game) fn native_add_section_geometry(
        context: &mod_api::RegistrationContext,
        renderers: &mut dyn mod_api::SectionGeometryRenderers,
    ) {
        Self::fire_add_section_geometry(context, renderers);
    }
}
