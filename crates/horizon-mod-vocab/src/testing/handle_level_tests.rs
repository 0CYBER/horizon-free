use crate::testing::FakeHost;

#[test]
fn player_level_and_dimension_reads() {
    use crate::host::entities::EntitiesHost;
    use crate::host::players::PlayersHost;
    let mut host = FakeHost::new();
    let overworld = host.level("minecraft:overworld").unwrap();
    let nether = crate::LevelRef::new(overworld.epoch(), crate::DimensionKey::NETHER);
    host.join(1, 10);
    host.join(2, 20).level = nether;
    let p1 = host.player(1).unwrap();
    let p2 = host.player(2).unwrap();
    let unknown = crate::__private::game::player_ref(p1.entity(), 99);
    let stale = crate::__private::game::player_ref(
        crate::EntityRef::new(
            crate::WorldEpoch::new(p1.entity().epoch().get() + 1),
            p1.entity().id(),
        ),
        p1.uuid(),
    );
    let replaced = crate::__private::game::player_ref(p2.entity(), p1.uuid());
    for invalid in [unknown, stale, replaced] {
        assert_eq!(invalid.level(&host), None);
        assert_eq!(invalid.dimension(&host), None);
    }
    assert_eq!(p1.level(&host), Some(overworld));
    assert_eq!(
        p1.dimension(&host),
        Some(crate::Identifier::parse("minecraft:overworld").unwrap())
    );
    assert_eq!(p2.level(&host), Some(nether));
    assert_eq!(
        p2.dimension(&host),
        Some(crate::Identifier::parse("minecraft:the_nether").unwrap())
    );
    host.players = Default::default();
    for disconnected in [p1, p2] {
        assert_eq!(disconnected.level(&host), None);
        assert_eq!(disconnected.dimension(&host), None);
    }
}

#[test]
fn fresh_fake_player_kind_has_player_id() {
    use crate::host::players::PlayersHost;
    let mut host = FakeHost::new();
    host.join(1, 10);
    host.resolve_entity_types().unwrap();
    host.level.entity(20, "minecraft:player");
    let player = host.player(1).unwrap();
    let kind = player.entity().kind(&host).unwrap();
    assert_eq!(kind.id(&host), "minecraft:player");
}

#[test]
#[should_panic(
    expected = "fake entity kind test:late was not declared before entity-type resolution"
)]
fn an_undeclared_fake_entity_kind_after_resolution_is_a_setup_error() {
    crate::parts::install_input(crate::__private::parts_installation::InstalledInput::default());
    let mut host = FakeHost::new();
    host.level.entity(20, "test:declared");
    host.resolve_entity_types().unwrap();
    host.level.entity(21, "test:declared");
    // An existing id keeps its kind; the unused argument does not declare a new one.
    assert_eq!(host.level.entity(20, "test:unused").kind, "test:declared");
    host.level.entity(22, "test:late");
}
