use super::*;
fn item(id: &str) -> RecipeSlotDisplay {
    RecipeSlotDisplay::Item(id.into())
}
fn grid() -> CheckedSlotLayout {
    CheckedSlotLayout::new(
        SlotLayout::Crafting {
            width: 3,
            height: 3,
            slots: (2..=10).collect(),
            result: 11,
        },
        12,
    )
    .unwrap()
}
fn shaped(width: u32, height: u32, ingredients: Vec<RecipeSlotDisplay>) -> RecipeBookDisplay {
    RecipeBookDisplay::Shaped {
        width,
        height,
        ingredients,
        result: item("stick"),
        station: item("table"),
    }
}
#[test]
fn vanilla_grid_positions_preserve_holes_and_center_only_small_shapes() {
    assert_eq!(
        grid().map(&shaped(1, 1, vec![item("oak")]), false).unwrap(),
        vec![(6, item("oak")), (11, item("stick"))]
    );
    assert_eq!(
        grid()
            .map(
                &shaped(
                    2,
                    2,
                    vec![
                        item("oak"),
                        RecipeSlotDisplay::Empty,
                        item("stone"),
                        item("oak")
                    ]
                ),
                false
            )
            .unwrap(),
        vec![
            (2, item("oak")),
            (5, item("stone")),
            (6, item("oak")),
            (11, item("stick"))
        ]
    );
    let display = RecipeBookDisplay::Shapeless {
        ingredients: vec![item("oak"), item("stone")],
        result: item("stick"),
        station: item("table"),
    };
    assert_eq!(
        grid().map(&display, false).unwrap(),
        vec![(2, item("oak")), (3, item("stone")), (11, item("stick"))]
    );
}
#[test]
fn malformed_layouts_are_refused_before_indexing() {
    assert!(matches!(
        CheckedSlotLayout::new(
            SlotLayout::Crafting {
                width: u16::MAX,
                height: u16::MAX,
                slots: vec![],
                result: 0
            },
            256
        ),
        Err(LayoutError::InvalidGrid)
    ));
    assert!(matches!(
        CheckedSlotLayout::new(
            SlotLayout::Furnace {
                input: 0,
                fuel: 1,
                result: 1
            },
            3
        ),
        Err(LayoutError::DuplicateSlot(1))
    ));
    assert!(matches!(
        CheckedSlotLayout::new(
            SlotLayout::Stonecutter {
                input: 0,
                result: 3
            },
            3
        ),
        Err(LayoutError::OutOfRange(3))
    ));
    assert!(matches!(
        grid().map(&shaped(u32::MAX, u32::MAX, vec![]), false),
        Err(LayoutError::InvalidDisplayGrid)
    ));
    assert!(matches!(
        grid().map(&shaped(2, 2, vec![item("oak")]), false),
        Err(LayoutError::InvalidDisplayGrid)
    ));
}
#[test]
fn geometry_must_include_each_mapped_slot_and_be_finite() {
    let mut geometry = vec![Some([0.0, 0.0]); 12];
    assert_eq!(grid().validate_geometry(&geometry), Ok(()));
    geometry[6] = None;
    assert_eq!(
        grid().validate_geometry(&geometry),
        Err(LayoutError::MissingGeometry(6))
    );
    geometry[6] = Some([f32::NAN, 0.0]);
    assert_eq!(
        grid().validate_geometry(&geometry),
        Err(LayoutError::MissingGeometry(6))
    );
}
#[test]
fn named_station_roles_and_fuel_occupancy_are_not_grid_guesses() {
    let furnace = CheckedSlotLayout::new(
        SlotLayout::Furnace {
            input: 4,
            fuel: 7,
            result: 9,
        },
        10,
    )
    .unwrap();
    let display = RecipeBookDisplay::Furnace {
        ingredient: item("ore"),
        fuel: item("coal"),
        result: item("iron"),
        station: item("furnace"),
        duration: 200,
        experience: 0.7,
    };
    assert_eq!(
        furnace.map(&display, true).unwrap(),
        vec![(4, item("ore")), (7, item("coal")), (9, item("iron"))]
    );
    assert_eq!(
        furnace.map(&display, false).unwrap(),
        vec![(4, item("ore")), (9, item("iron"))]
    );
    assert_eq!(
        grid().map(&display, true),
        Err(LayoutError::IncompatibleDisplay)
    );
    let cutter = CheckedSlotLayout::new(
        SlotLayout::Stonecutter {
            input: 4,
            result: 9,
        },
        10,
    )
    .unwrap();
    let display = RecipeBookDisplay::Stonecutter {
        input: item("stone"),
        result: item("slab"),
        station: item("cutter"),
    };
    assert_eq!(
        cutter.map(&display, false).unwrap(),
        vec![(4, item("stone")), (9, item("slab"))]
    );
    let smith = CheckedSlotLayout::new(
        SlotLayout::Smithing {
            template: 2,
            base: 5,
            addition: 3,
            result: 8,
        },
        10,
    )
    .unwrap();
    let display = RecipeBookDisplay::Smithing {
        template: item("template"),
        base: item("armor"),
        addition: item("material"),
        result: item("upgrade"),
        station: item("smith"),
    };
    assert_eq!(
        smith.map(&display, false).unwrap(),
        vec![
            (2, item("template")),
            (5, item("armor")),
            (3, item("material")),
            (8, item("upgrade"))
        ]
    );
}

#[test]
fn storage_agreement_and_existing_menu_limit() {
    use super::super::menu_layout::{MenuLayoutMetadata, compile};
    let metadata = MenuLayoutMetadata {
        slots: 12,
        crafting: Some((2, 3, 11)),
    };
    assert!(
        compile(
            SlotLayout::Crafting {
                width: 3,
                height: 3,
                slots: (2..=10).collect(),
                result: 11
            },
            metadata
        )
        .is_ok()
    );
    assert!(matches!(
        compile(
            SlotLayout::Crafting {
                width: 3,
                height: 3,
                slots: (1..=9).collect(),
                result: 11
            },
            metadata
        ),
        Err(LayoutError::InvalidGrid)
    ));
    assert!(matches!(
        compile(
            SlotLayout::Stonecutter {
                input: 2,
                result: 11
            },
            metadata
        ),
        Err(LayoutError::IncompatibleDisplay)
    ));
    let layout = CheckedSlotLayout::new(
        SlotLayout::Crafting {
            width: 255,
            height: 1,
            slots: (0..255).collect(),
            result: 255,
        },
        256,
    )
    .unwrap();
    let display = shaped(1, 1, vec![item("oak")]);
    let mapped = layout.map(&display, false).unwrap();
    assert_eq!(mapped, vec![(127, item("oak")), (255, item("stick"))]);
}
