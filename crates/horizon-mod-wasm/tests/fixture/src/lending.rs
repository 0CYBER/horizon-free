use mod_api::{Hooks, MenuLinksEvent, RecipeSourcesEvent, ServerHost};

pub struct Lender;

#[mod_api::__subscriptions]
impl Hooks for Lender {
    fn living_death(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity != 0xffff_ff01 {
            return mod_api::Flow::Continue;
        }
        assert_eq!(event.kind, "minecraft:zombie");
        assert_eq!(event.source.damage_type, "minecraft:arrow");
        assert_eq!(event.source.causing, 41);
        assert_eq!(event.source.direct, 73);
        assert_eq!(event.source.position, Some([-17.25, 64.5, 32.75]));
        if let Some(weapon) = event.weapon {
            assert_eq!(weapon.count(), 1);
            assert_eq!(weapon.damage(), 19);
            assert_eq!(
                Some(weapon.item()),
                host.item_key(&mod_api::Identifier::parse("minecraft:bow").unwrap())
            );
            assert_eq!(weapon, &death_weapon(weapon.item()));
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }

    fn recipe_sources(&self, _: &mut dyn ServerHost, event: &mut RecipeSourcesEvent<'_>) {
        assert_eq!(event.player, 0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        assert_eq!(event.menu, "wasm_fixture:machine");
        assert_eq!(event.pos, [-17, 64, 32]);
        assert_eq!(event.sources.as_slice(), [[-17, 63, 32]]);
        event.sources.push([-16, 64, 32]);
        event.sources.push([-18, 64, 32]);
    }

    fn menu_links(&self, _: &mut dyn ServerHost, event: &mut MenuLinksEvent<'_>) {
        assert_eq!(event.player, 0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        assert_eq!(event.menu, "wasm_fixture:machine");
        assert_eq!(event.pos, [-17, 64, 32]);
        assert_eq!(event.links.as_slice(), [[-17, 63, 32]]);
        event.links.push([-16, 64, 32]);
        event.links.push([-18, 64, 32]);
    }
}

pub fn check(host: &mut dyn ServerHost) {
    let mut sources = vec![[-17, 63, 32]];
    mod_api::dispatch_recipe_sources(
        host,
        0x1234_5678_9abc_def0_1234_5678_9abc_def0,
        "wasm_fixture:machine",
        [-17, 64, 32],
        &mut sources,
    );
    assert_eq!(sources, [[-17, 63, 32], [-16, 64, 32], [-18, 64, 32]]);
    let mut links = vec![[-17, 63, 32]];
    mod_api::dispatch_menu_links(
        host,
        0x1234_5678_9abc_def0_1234_5678_9abc_def0,
        "wasm_fixture:machine",
        [-17, 64, 32],
        &mut links,
    );
    assert_eq!(links, sources);
}

fn death_weapon(item: mod_api::ItemKey) -> mod_api::ItemStack {
    let mut weapon = mod_api::ItemStack::new(item, 1).unwrap();
    weapon.set_damage(19);
    weapon
        .set_components(
            [(
                "minecraft:custom_data".into(),
                mod_api::Tag::Compound(
                    [(
                        "marker".into(),
                        mod_api::Tag::String("death: café 水".into()),
                    )]
                    .into(),
                ),
            )]
            .into(),
        )
        .unwrap();
    weapon
}

/// The native and compiled-guest runners invoke this same portable boundary proof.
/// Death animation, health and loot behavior belong to the actual Game regressions.
pub fn check_death(host: &mut dyn ServerHost, bow: mod_api::ItemKey) {
    let weapon = death_weapon(bow);
    let source = mod_api::HurtSource {
        damage_type: "minecraft:arrow",
        causing: 41,
        direct: 73,
        position: Some([-17.25, 64.5, 32.75]),
    };
    for (weapon, expected_cancel) in [(Some(&weapon), true), (None, false)] {
        let mut cancel = false;
        mod_api::dispatch_living_death(
            host,
            0xffff_ff01,
            "minecraft:zombie",
            &source,
            weapon,
            &mut cancel,
        );
        assert_eq!(cancel, expected_cancel);
    }
    assert_eq!(
        weapon,
        death_weapon(bow),
        "the borrowed killing stack is unchanged"
    );
}
