//! ItemParticleOption's stack template uses the ordinary inventory component codec.
use super::*;
pub(super) fn encode(
    hub: &Hub,
    id: i32,
    options: &crate::particles::ParticleOptions,
) -> Option<Particle> {
    if options.ty != crate::particles::ParticleType::Item {
        return None;
    }
    let stack = crate::lan::guest::stack_to_network(
        |name| {
            hub.sync
                .builtin
                .get("minecraft:item")
                .and_then(|r| r.id(name))
                .unwrap_or(0)
        },
        |registry, name| hub.sync.id(registry, name),
        &options.item_stack,
    )?;
    Some(Particle {
        id,
        data: Some(ParticleData::Item(stack)),
    })
}
