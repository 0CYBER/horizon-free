use super::*;
use mod_api::{
    Identifier, PoiKey, ProfessionDefinition, ProfessionKey, ProfessionRegistrationError as E,
    TradeSetKey,
};
fn engineer() -> ProfessionDefinition {
    let id = |s| Identifier::parse(s).unwrap();
    ProfessionDefinition {
        workstations: vec![],
        key: ProfessionKey(id("test:engineer")),
        held_poi: vec![PoiKey(id("test:assembler"))],
        acquirable_poi: vec![PoiKey(id("test:assembler"))],
        requested_items: vec![],
        secondary_blocks: vec![],
        work_sound: None,
        trades: vec![(1, TradeSetKey(id("test:engineer")))],
    }
}
#[test]
fn candidate_retains_explicit_identity_and_vanilla_order() {
    let table =
        ResolvedProfessions::prepare(vec![("test".into(), engineer())], |_, _| true).unwrap();
    assert_eq!(table.entries().len(), 16);
    assert_eq!(table.resolve("minecraft:none"), Some(0));
    assert_eq!(table.resolve("minecraft:farmer"), Some(5));
    assert_eq!(table.resolve("minecraft:weaponsmith"), Some(14));
    assert_eq!(table.resolve("test:engineer"), Some(15));
    assert_eq!(table.acquiring("test:assembler"), Some(15));
    assert_eq!(
        table.entry(15).unwrap().definition.as_ref().unwrap().trades[0]
            .1
            .0
            .to_string(),
        "test:engineer"
    );
}
#[test]
fn refused_candidate_keeps_active_table() {
    let mut registry = ProfessionRegistry::default();
    let old = registry.active().clone();
    let candidate = ResolvedProfessions::prepare(
        vec![("z".into(), engineer()), ("a".into(), engineer())],
        |_, _| true,
    );
    assert!(
        matches!(&candidate, Err(errors) if errors.iter().any(|e| matches!(e, E::DuplicateProfession { owners, .. } if owners == &["a", "z"])))
    );
    assert!(registry.publish(candidate).is_err());
    assert!(std::sync::Arc::ptr_eq(&old, registry.active()));
    assert_eq!(registry.active().entries().len(), 15);
    assert_eq!(registry.active().acquiring("test:assembler"), None);
}
#[test]
fn reserved_overlap_and_missing_references_refuse() {
    let mut reserved = engineer();
    reserved.key = ProfessionKey(Identifier::parse("minecraft:farmer").unwrap());
    assert!(
        matches!(ResolvedProfessions::prepare(vec![("a".into(), reserved)], |_, _| true), Err(e) if e.iter().any(|e| matches!(e, E::ReservedProfession { .. })))
    );
    let mut second = engineer();
    second.key = ProfessionKey(Identifier::parse("test:mechanic").unwrap());
    assert!(
        matches!(ResolvedProfessions::prepare(vec![("a".into(), engineer()), ("b".into(), second)], |_, _| true), Err(e) if e.iter().any(|e| matches!(e, E::OverlappingPoi { .. })))
    );
    assert!(
        matches!(ResolvedProfessions::prepare(vec![("a".into(), engineer())], |_, _| false), Err(e) if e.iter().any(|e| matches!(e, E::UnknownPoi { .. })) && e.iter().any(|e| matches!(e, E::UnknownTradeSet { .. })))
    );
}

#[test]
fn villager_vanilla_fifteen_rows_and_farmer_keys_are_unchanged() {
    let table = ResolvedProfessions::vanilla();
    let expected = [
        "minecraft:none",
        "minecraft:armorer",
        "minecraft:butcher",
        "minecraft:cartographer",
        "minecraft:cleric",
        "minecraft:farmer",
        "minecraft:fisherman",
        "minecraft:fletcher",
        "minecraft:leatherworker",
        "minecraft:librarian",
        "minecraft:mason",
        "minecraft:nitwit",
        "minecraft:shepherd",
        "minecraft:toolsmith",
        "minecraft:weaponsmith",
    ];
    assert_eq!(table.entries().len(), 15);
    for (index, name) in expected.into_iter().enumerate() {
        assert_eq!(table.resolve(name), Some(index), "{name}");
    }
    assert_eq!(
        table.job(horizon_world::poi::PoiType::Farmer.into()),
        Some(5)
    );
    assert_eq!(
        table.trade_key("minecraft:farmer", 1),
        Some("minecraft:farmer/level_1")
    );
    assert_eq!(
        table.trade_key("minecraft:farmer", 2),
        Some("minecraft:farmer/level_2")
    );
    assert_eq!(
        table.trade_key("minecraft:farmer", 3),
        Some("minecraft:farmer/level_3")
    );
    assert_eq!(
        table.trade_key("minecraft:farmer", 4),
        Some("minecraft:farmer/level_4")
    );
    assert_eq!(
        table.trade_key("minecraft:farmer", 5),
        Some("minecraft:farmer/level_5")
    );
    let composter = horizon_world::chunk::BlockState {
        name: "minecraft:composter".into(),
        properties: Default::default(),
    };
    assert_eq!(
        table.pois.classify(&composter),
        Some(horizon_world::poi::PoiIdentity::Vanilla(
            horizon_world::poi::PoiType::Farmer
        ))
    );
}

fn saved_mob(
    species: crate::mobs::Species,
    id: &str,
) -> (
    crate::mobs::Mob,
    std::collections::HashMap<String, std::sync::Arc<crate::mobs::MobType>>,
) {
    use crate::{
        items::ItemStack,
        merchant::{ItemCost, MerchantOffer, Trader},
    };
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let data = std::sync::Arc::make_mut(&mut kind);
    data.id = id.into();
    data.species = species;
    let mut mob = crate::mobs::Mob::new(
        71,
        kind.clone(),
        horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        0.0,
    );
    mob.extra.profession =
        crate::mobs::villager::ProfessionState::Unresolved("test:engineer".into());
    mob.extra.villager_level = 3;
    mob.extra.trade = Some(Box::new(Trader {
        xp: 37,
        last_restock: 12345,
        restocks_today: 1,
        check_day: 0,
        unset: false,
        offers: vec![MerchantOffer {
            base_cost_a: ItemCost {
                stack: ItemStack::new("minecraft:emerald", 3),
            },
            cost_b: None,
            result: ItemStack::new("minecraft:iron_ingot", 2),
            uses: 2,
            max_uses: 12,
            reward_exp: true,
            special_price_diff: -1,
            demand: 4,
            price_multiplier: 0.2,
            xp: 7,
        }],
    }));
    (mob, [(id.into(), kind)].into())
}

fn save_cycle(species: crate::mobs::Species, id: &str, missing: bool) {
    use crate::{
        entity_nbt::{mob_from_nbt, mob_to_nbt},
        mobs::villager::ProfessionState,
    };
    let modded =
        ResolvedProfessions::prepare(vec![("test".into(), engineer())], |_, _| true).unwrap();
    let vanilla = ResolvedProfessions::vanilla();
    let (mut mob, kinds) = saved_mob(species, id);
    modded.resolve_loaded(&mut mob);
    assert!(matches!(
        mob.extra.profession,
        ProfessionState::Registered { .. }
    ));
    let expected_offers = mob.extra.trade.as_ref().unwrap().offers.clone();
    let first = mob_to_nbt(&mob, [7, 8, 9, 10], None, None, |_| None);
    let name = |tag: &horizon_core::nbt::Tag| {
        crate::save::text(
            tag.get("VillagerData")
                .and_then(|data| data.get("profession")),
        )
        .unwrap()
    };
    assert_eq!(name(&first), "test:engineer");
    let mut loaded = mob_from_nbt(&first, &kinds, 72, None).unwrap();
    let table = if missing { &vanilla } else { &modded };
    table.resolve_loaded(&mut loaded);
    assert_eq!(loaded.extra.profession.id(), "test:engineer");
    assert_eq!(loaded.extra.profession.index(), None);
    assert_eq!(
        table.state_index(&loaded.extra.profession),
        if missing { None } else { Some(15) }
    );
    if missing {
        assert!(matches!(
            loaded.extra.profession,
            ProfessionState::Unresolved(_)
        ));
    }
    assert_eq!(loaded.extra.trade.as_ref().unwrap().offers, expected_offers);
    assert_eq!(loaded.extra.trade.as_ref().unwrap().xp, 37);
    assert_eq!(loaded.extra.villager_level, 3);
    let second = mob_to_nbt(&loaded, [7, 8, 9, 10], Some(&first), None, |_| None);
    assert_eq!(name(&second), "test:engineer");
    assert_eq!(second.get("Offers"), first.get("Offers"));
    let mut restored = mob_from_nbt(&second, &kinds, 73, None).unwrap();
    modded.resolve_loaded(&mut restored);
    assert!(matches!(
        restored.extra.profession,
        ProfessionState::Registered { .. }
    ));
    assert_eq!(modded.state_index(&restored.extra.profession), Some(15));
    assert_eq!(
        restored.extra.trade.as_ref().unwrap().offers,
        expected_offers
    );
    assert!(!restored.extra.trade.as_ref().unwrap().unset);
}
#[test]
fn villager_saved_offers_survive_reload_with_mod() {
    save_cycle(crate::mobs::Species::Villager, "minecraft:villager", false);
}
#[test]
fn zombie_villager_saved_offers_survive_reload_with_mod() {
    save_cycle(
        crate::mobs::Species::ZombieVillager,
        "minecraft:zombie_villager",
        false,
    );
}
#[test]
fn villager_missing_mod_save_and_restore_preserves_name_and_offers() {
    save_cycle(crate::mobs::Species::Villager, "minecraft:villager", true);
}
#[test]
fn zombie_villager_missing_mod_save_and_restore_preserves_name_and_offers() {
    save_cycle(
        crate::mobs::Species::ZombieVillager,
        "minecraft:zombie_villager",
        true,
    );
}

#[test]
fn villager_restored_name_resolves_after_registry_reordering_without_trade_changes() {
    use crate::mobs::{Species, villager::ProfessionState};
    let mut other = engineer();
    other.key = ProfessionKey(Identifier::parse("test:mechanic").unwrap());
    other.held_poi.clear();
    other.acquirable_poi.clear();
    other.trades.clear();
    let reordered = ResolvedProfessions::prepare(
        vec![("other".into(), other), ("test".into(), engineer())],
        |_, _| true,
    )
    .unwrap();
    let (mut mob, _) = saved_mob(Species::Villager, "minecraft:villager");
    let offers = mob.extra.trade.as_ref().unwrap().offers.clone();
    assert!(matches!(
        mob.extra.profession,
        ProfessionState::Unresolved(_)
    ));
    reordered.resolve_loaded(&mut mob);
    assert_eq!(reordered.state_index(&mob.extra.profession), Some(15));
    assert_eq!(mob.extra.profession.id(), "test:engineer");
    assert_eq!(mob.extra.profession.index(), None);
    assert_eq!(mob.extra.trade.as_ref().unwrap().offers, offers);
}

#[test]
fn villager_profession_slots_are_canonical_across_owner_order() {
    let a = engineer();
    let mut b = engineer();
    b.key = ProfessionKey(Identifier::parse("other:mechanic").unwrap());
    b.acquirable_poi = vec![PoiKey(Identifier::parse("other:bench").unwrap())];
    for declarations in [
        vec![("z".into(), a.clone()), ("a".into(), b.clone())],
        vec![("a".into(), b), ("z".into(), a)],
    ] {
        let table = ResolvedProfessions::prepare(declarations, |_, _| true).unwrap();
        assert_eq!(table.resolve("other:mechanic"), Some(15));
        assert_eq!(table.resolve("test:engineer"), Some(16));
        assert_eq!(table.resolve("minecraft:weaponsmith"), Some(14));
    }
}

#[test]
fn villager_unacquirable_custom_workstation_is_refused() {
    let mut definition = engineer();
    definition.acquirable_poi.clear();
    definition
        .workstations
        .push(mod_api::ProfessionWorkstation {
            poi: PoiKey(Identifier::parse("test:assembler").unwrap()),
            block: Identifier::parse("test:assembler").unwrap(),
            properties: vec![],
        });
    let errors =
        ResolvedProfessions::prepare(vec![("test".into(), definition)], |_, _| true).unwrap_err();
    assert!(errors.iter().any(
        |error| matches!(error, E::UnacquirablePoi { key } if key.0.to_string() == "test:assembler")
    ));
}

#[test]
fn villager_reports_registered_and_unresolved_names() {
    use crate::mobs::{
        Species,
        villager::{ProfessionState, command_name, reported_profession},
    };
    let (mut mob, _) = saved_mob(Species::Villager, "minecraft:villager");
    for state in [
        ProfessionState::Registered {
            id: "test:engineer".into(),
            slot: 15,
        },
        ProfessionState::Unresolved("test:engineer".into()),
    ] {
        mob.extra.profession = state;
        assert_eq!(reported_profession(&mob), "test:engineer");
        assert_eq!(
            command_name(&mob),
            crate::text::Text::translate("entity.minecraft.villager.test:engineer", &[])
        );
    }
    mob.extra.profession = ProfessionState::from_index(5);
    assert_eq!(reported_profession(&mob), "farmer");
}
