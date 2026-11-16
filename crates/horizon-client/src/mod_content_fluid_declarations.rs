use super::*;

/// Joins the world fluids native mods declare in code to the content mods of their namespaces:
/// a fluid's file, if it has one, keeps saying how it looks.
pub fn accept_world_fluids(mods: &mut [ContentMod], fluids: &[FluidLook]) {
    for fluid in fluids {
        let namespace = fluid.id.split_once(':').map_or("", |(ns, _)| ns);
        let Some(owner) = mods
            .iter_mut()
            .find(|m| m.content.namespaces.iter().any(|owned| owned == namespace))
        else {
            continue;
        };
        match owner.content.fluids.iter_mut().find(|f| f.id == fluid.id) {
            Some(look) => look.world = fluid.world,
            None => owner.content.fluids.push(fluid.clone()),
        }
    }
}
