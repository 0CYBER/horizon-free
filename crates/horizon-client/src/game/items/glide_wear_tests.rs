use super::*;

/// Glides a survival or creative guest for 20 ticks with `components` on a stick in `hand`
/// (4 main, 5 off), or an elytra in the chest slot when `components` is `None`, and returns
/// the worn stack's damage.
fn glide(hand: usize, creative: bool, components: Option<&str>) -> u32 {
    mod_platform::testing::with_mods(Default::default(), || {
        let mut damage = None;
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let elytra = components.is_none();
            let mut stack = ItemStack::new(
                if elytra {
                    "minecraft:elytra"
                } else {
                    "minecraft:stick"
                },
                1,
            );
            if let Some(components) = components {
                let tag = horizon_client::item_data::command_components(components).unwrap();
                let (_, extra) = horizon_client::item_data::decode_components(Some(&tag));
                stack.extra = extra;
            }
            let body = horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(3.0, 80.0, 5.0),
            )
            .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.game_mode = if creative { 1 } else { 0 };
                    g.on_ground = false;
                    g.fall_flying = true;
                    g.motion = Some(Box::new(body));
                    g.armor = Default::default();
                    if elytra {
                        g.armor[1] = Some(stack);
                    } else if hand == 4 {
                        g.inventory[g.selected] = Some(stack);
                    } else {
                        g.offhand = Some(stack);
                    }
                })
                .unwrap();
            for _ in 0..20 {
                game.guest_glide_tick(12, -1.0);
            }
            let g = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            let stack = if elytra {
                g.armor[1].as_ref()
            } else if hand == 4 {
                g.inventory[g.selected].as_ref()
            } else {
                g.offhand.as_ref()
            }
            .unwrap();
            damage = Some(stack.damage);
        });
        damage.unwrap()
    })
}

const MAIN_HAND: &str = "[minecraft:glider={},minecraft:equippable={slot:\"mainhand\"},minecraft:max_damage=10,minecraft:damage=0,minecraft:max_stack_size=1]";
const OFF_HAND: &str = "[minecraft:glider={},minecraft:equippable={slot:\"offhand\"},minecraft:max_damage=10,minecraft:damage=0,minecraft:max_stack_size=1]";
/// `ItemStack.isDamageableItem` needs a `damage` component too: this one glides but never wears.
const NO_DAMAGE: &str = "[minecraft:glider={},minecraft:equippable={slot:\"mainhand\"},minecraft:max_damage=10,minecraft:max_stack_size=1]";

#[test]
#[ignore = "daily"]
fn main_hand_glide_wear() {
    assert_eq!(glide(4, false, Some(MAIN_HAND)), 1);
}
#[test]
#[ignore = "daily"]
fn off_hand_glide_wear() {
    assert_eq!(glide(5, false, Some(OFF_HAND)), 1);
}
#[test]
#[ignore = "daily"]
fn creative_glide_wear() {
    assert_eq!(glide(4, true, Some(MAIN_HAND)), 0);
}
#[test]
#[ignore = "daily"]
fn chest_glide_wear() {
    assert_eq!(glide(1, false, None), 1);
}
#[test]
#[ignore = "daily"]
fn a_glider_without_a_damage_component_glides_but_never_wears() {
    assert_eq!(glide(4, false, Some(NO_DAMAGE)), 0);
}
