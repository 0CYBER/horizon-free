//! Work-site restocking and the trading-player refresh event.
use super::{Mob, MobEvent, MobWorld};

/// `WorkAtPoi.start`'s restock.
pub(in crate::mobs) fn restock_if_due(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if w.professions.state_index(&m.extra.profession).is_none() {
        return;
    }
    let day = w.day_time.div_euclid(24000);
    if let Some(t) = m.extra.trade.as_mut() {
        let before = t.offers.clone();
        let before_restock = (t.last_restock, t.restocks_today);
        let restock = t.should_restock(w.game_time, day);
        if restock {
            t.restock(w.game_time);
        }
        // shouldRestock also catches up missed demand on a new day.
        if restock || t.offers != before || (t.last_restock, t.restocks_today) != before_restock {
            events.push(MobEvent::VillagerRestocked { mob: m.id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merchant::Trader;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn work_site_refreshes_real_restock_and_counter_only_catchup() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |_, _| {},
            |world, types| {
                let mut world = world.clone();
                world.game_time = 6000;
                world.day_time = 2000;
                // Used stock restocks; next-day catch-up must still refresh the
                // trading player when only counters change; unused fresh stock does not.
                for (uses, last, restocks, demand, expected_last, expected_restocks, refresh) in [
                    (12, 0, 0, 12, 6000, 1, true),
                    (0, -24000, 2, 0, 6000, 0, true),
                    (0, 6000, 1, 0, 6000, 1, false),
                ] {
                    let mut mob = Mob::new(
                        77,
                        Arc::new(types.types["minecraft:villager"].clone()),
                        Vec3::ZERO,
                        0.0,
                    );
                    mob.extra.trade = Some(Box::new(Trader {
                        xp: 20,
                        offers: crate::entity_nbt::offers_from_nbt(&horizon_core::nbt::parse_snbt(
                            &format!(concat!("{{Recipes:[{{buy:{{id:\"minecraft:emerald\",count:4}},",
                                "sell:{{id:\"minecraft:glass\",count:3}},uses:{uses},maxUses:12,",
                                "rewardExp:0b,xp:1,priceMultiplier:0.05f,demand:0,specialPrice:0}}]}}"), uses = uses)
                        ).unwrap().0),
                        last_restock: last,
                        restocks_today: restocks,
                        ..Default::default()
                    }));
                    let mut events = Vec::new();
                    restock_if_due(&mut mob, &world, &mut events);
                    assert_eq!(events.len(), usize::from(refresh));
                    if refresh {
                        assert!(matches!(events[0], MobEvent::VillagerRestocked { mob: 77 }));
                    }
                    let trade = mob.extra.trade.as_ref().unwrap();
                    assert_eq!(trade.offers[0].uses, 0);
                    assert_eq!(trade.offers[0].demand, demand);
                    assert_eq!(trade.last_restock, expected_last);
                    assert_eq!(trade.restocks_today, expected_restocks);
                    assert_eq!(trade.xp, 20);
                    events.clear();
                    restock_if_due(&mut mob, &world, &mut events);
                    assert!(
                        events.is_empty(),
                        "the same work tick must not refresh twice"
                    );
                }
            },
        );
        assert!(
            ran,
            "the prepared resource fixture must execute all three controls"
        );
    }
}
