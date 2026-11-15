/// Doubles every comparator reading, and attaches `Gauge` to `test:gauge`, `TagGauge` to
/// tag `test:gauged` and `Press` to `test:press`.
pub(crate) struct Doubler;
#[mod_api::__subscriptions]
impl Hooks for Doubler {
    fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
        let block_key = r.block_key("test:gauge");
        r.block_part(block_key, Gauge);
        let block_key = r.block_key("test:connector");
        r.block_part(block_key, Connector);
        r.block_part_tag("test:gauged", TagGauge);
        let kind = r
            .block_entity_type("press_entity")
            .blocks(&["test:press"])
            .build();
        r.block_entity_part(&kind, Press).expect("native type");
        let hoard = r.block_entity_type("hoard").blocks(&["test:hoard"]).build();
        let granite = r.block_key("minecraft:granite");
        r.block_entity_part(&hoard, Hoard(granite))
            .expect("native type");
        // These handlers wrap the test hosts' own slots and tanks, as a mod replacing how a
        // block entity's are reached does: ahead of the game's.
        let pipe = r.block_entity_type_key("test:pipe");
        r.block_entity_capability_before_game(&ITEMS, &pipe, |_, at| Some(Box::new(Pipe(at))));
        let greedy = r.block_entity_type_key("test:greedy");
        r.block_entity_capability_before_game(&ITEMS, &greedy, |_, at| Some(Box::new(Greedy(at))));
        let item_overstates = r.block_entity_type_key("test:item_overstates");
        r.block_entity_capability_before_game(&ITEMS, &item_overstates, |_, _| {
            Some(Box::new(ItemOverstates))
        });
        r.entity_capability(&ENTITY_ITEMS, "test:robot", |_, _, ()| {
            Some(Box::new(Cargo))
        });
        // A battery reached from any side but below, as a mod's sided entity storage.
        r.entity_capability(
            &ENTITY_ENERGY,
            "minecraft:armor_stand",
            |_, entity, side| {
                (side != Some(Direction::Down)).then(|| Box::new(EntityBattery(entity)) as _)
            },
        );
        r.entity_capability(&ENTITY_FLUIDS, "minecraft:armor_stand", |_, entity, _| {
            Some(Box::new(EntityTank(entity)))
        });
        r.item_capability_before_game(&ITEM_ITEMS, "test:pouch", |_, place| {
            Some(Box::new(Pouch(place.clone())))
        });
        r.item_capability(&ITEM_ENERGY, "test:cell", |_, place| {
            Some(Box::new(Cell(place.clone())))
        });
        r.item_capability_before_game(&ITEM_ITEMS, "test:bag", |_, place| {
            Some(Box::new(Bag(place.clone())))
        });
        r.item_capability_before_game(&ITEM_ITEMS, "test:case", |_, place| {
            Some(Box::new(Case(place.clone())))
        });
        r.place_provider("test:charms", Arc::new(Charms));
        r.place_provider("test:lying-place", Arc::new(deep_places::LyingPlace));
        r.item_capability_before_game(&ITEM_ITEMS, "test:outer", |_, place| {
            Some(Box::new(deep_places::Wrap(place.clone())))
        });
        // The `test:ordered` block's first providers, registered ahead of its block entity
        // type's: one after the game answering at x = 5, one ahead of it at x = 12.
        let ordered_block = r.block_key("test:ordered");
        r.block_capability(&ITEMS, &ordered_block, |_, at, _, _| {
            (at.pos[0] == 5).then(|| Box::new(Marked(7)) as _)
        });
        r.block_capability_before_game(&ITEMS, &ordered_block, |_, at, _, _| {
            (at.pos[0] == 12).then(|| Box::new(Marked(8)) as _)
        });
        // `test:ordered`: one provider that declines, two that answer, and one ahead of the
        // game that answers only at x = 9.
        let ordered = r.block_entity_type_key("test:ordered");
        r.block_entity_capability(&ITEMS, &ordered, |_, _| None);
        r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(1))));
        r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(2))));
        r.block_entity_capability_before_game(&ITEMS, &ordered, |_, at| {
            (at.pos[0] == 9).then(|| Box::new(Marked(3)) as _)
        });
        r.block_entity_capability_before_game(&ITEMS, &ordered, |_, at| {
            (at.pos[0] >= 9).then(|| Box::new(Marked(4)) as _)
        });
        // `test:ordered_item` the same, the one ahead of the game answering for two items.
        r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| None);
        r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
            Some(Box::new(Marked(1)))
        });
        r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
            Some(Box::new(Marked(2)))
        });
        r.item_capability_before_game(&ITEM_ITEMS, "test:ordered_item", |host, place| {
            let two = place.amount(host) == 2;
            two.then(|| Box::new(Marked(3)) as _)
        });
        for (id, lie) in [
            ("test:tank", None),
            ("test:gives_more", Some(Lie::GivesMore)),
            ("test:takes_more", Some(Lie::TakesMore)),
        ] {
            let kind = r.block_entity_type_key(id);
            r.block_entity_capability_before_game(&FLUIDS, &kind, move |_, at| {
                Some(Box::new(ModTank { at, lie }))
            });
        }
        for (id, lie) in [
            ("test:battery_gives_more", Lie::GivesMore),
            ("test:battery_takes_more", Lie::TakesMore),
        ] {
            let kind = r.block_entity_type_key(id);
            r.block_entity_capability_before_game(&ENERGY, &kind, move |_, at| {
                Some(Box::new(ModBattery { at, lie }))
            });
        }
        let chem_tank = r.block_entity_type_key("test:chem_tank");
        r.block_entity_capability(&CHEMICALS, &chem_tank, |_, at| Some(Box::new(ChemTank(at))));
        // `test:void_bin`, a block with no block entity, voids what it is given.
        let void_bin = r.block_key("test:void_bin");
        r.block_capability(&ITEMS, &void_bin, |_, _, _, entity| {
            entity.is_none().then(|| Box::new(VoidBin) as _)
        });
        // The `test:ordered` block's later providers, registered after its block entity
        // type's: one ahead of the game answering at x = 6 and x = 11, and one after it
        // answering anywhere.
        r.block_capability_before_game(&ITEMS, &ordered_block, |_, at, _, _| {
            matches!(at.pos[0], 6 | 11).then(|| Box::new(Marked(5)) as _)
        });
        r.block_capability(&ITEMS, &ordered_block, |_, _, _, _| {
            Some(Box::new(Marked(6)))
        });
    }
}
