//! Allocation regression coverage runs in client-lib's counting allocator.
use super::*;

#[test]
#[ignore = "daily"]
fn effective_damage_resistance_queries_allocate_nothing() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let damage = crate::health::HealthResources::load(&root).unwrap();
    let stacks = [
        r#"{id:"minecraft:netherite_sword",count:1}"#,
        r#"{id:"minecraft:netherite_sword",count:1,components:{"!minecraft:damage_resistant":{}}}"#,
        r##"{id:"minecraft:stone",count:1,components:{"minecraft:damage_resistant":{types:"#minecraft:panic_environmental_causes"}}}"##,
        r#"{id:"minecraft:nether_star",count:1}"#,
        r#"{id:"minecraft:nether_star",count:1,components:{"!minecraft:damage_resistant":{}}}"#,
        r##"{id:"minecraft:diamond",count:1,components:{"minecraft:damage_resistant":{types:"#minecraft:is_explosion"}}}"##,
    ].map(|input| crate::item_data::parse_stack(input).unwrap().unwrap());
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for stack in &stacks {
            let default = defaults
                .entries()
                .get(stack.item.as_ref())
                .and_then(|components| components.get("minecraft:damage_resistant"));
            for name in [
                "minecraft:in_fire",
                "minecraft:lava",
                "minecraft:cactus",
                "minecraft:explosion",
                "minecraft:lightning_bolt",
            ] {
                let _ = std::hint::black_box(stack_can_be_hurt_by(
                    default,
                    &damage.damage,
                    stack,
                    name,
                ));
            }
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
}
