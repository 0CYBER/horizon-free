//! Validate registered destination roles before publishing content menu types.
use horizon_client::recipe_book::{
    menu_layout::{MenuLayoutMetadata, compile},
    slot_layout::LayoutError,
};
use horizon_mod_package::content::MenuType;
use mod_api::RecipeBookLayout;

pub(super) fn validate_registered(menus: &[&MenuType]) -> Result<(), String> {
    for menu in RecipeBookLayout::registered_menus() {
        let Some((_, _, layout)) = RecipeBookLayout::registered(&menu) else {
            continue;
        };
        let screen = RecipeBookLayout::registered_screen(&menu)
            .ok_or_else(|| format!("recipe book menu {menu:?}: registered screen missing"))?;
        validate(menus, &menu, layout, &screen)?;
    }
    Ok(())
}

fn validate(
    menus: &[&MenuType],
    id: &str,
    layout: RecipeBookLayout,
    screen: &mod_api::Screen,
) -> Result<(), String> {
    let menu = menus
        .iter()
        .find(|menu| menu.id == id)
        .ok_or_else(|| format!("recipe book menu {id:?}: unknown content menu"))?;
    let metadata = MenuLayoutMetadata {
        slots: menu.slots,
        crafting: menu
            .crafting
            .map(|grid| (grid.grid, grid.size, grid.result)),
    };
    compile(layout, metadata)
        .map_err(|error: LayoutError| format!("recipe book menu {id:?}: {error:?}"))?;
    if let Some((slot, _)) = screen
        .slots
        .iter()
        .enumerate()
        .skip(usize::from(menu.slots))
        .find(|(_, position)| position.is_some())
    {
        return Err(format!(
            "recipe book menu {id:?}: screen defines nonexistent container slot {slot}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn screen() -> mod_api::Screen {
        mod_api::Screen {
            width: 176.0,
            height: 166.0,
            slots: vec![Some([8.0, 8.0]); 3],
            ..Default::default()
        }
    }
    fn menu() -> MenuType {
        MenuType {
            id: "demo:pot".into(),
            slots: 3,
            data_slots: 0,
            results: vec![2],
            crafting: None,
            links: None,
        }
    }
    #[test]
    fn actual_content_count_is_required_before_publication() {
        let menu = menu();
        let roles = RecipeBookLayout::Furnace {
            input: 0,
            fuel: 1,
            result: 2,
        };
        assert!(validate(&[&menu], "demo:pot", roles.clone(), &screen()).is_ok());
        assert!(
            validate(&[], "demo:pot", roles.clone(), &screen())
                .unwrap_err()
                .contains("unknown content menu")
        );
        let short = MenuType { slots: 2, ..menu };
        assert!(
            validate(&[&short], "demo:pot", roles, &screen())
                .unwrap_err()
                .contains("OutOfRange(2)")
        );
    }
    #[test]
    fn screen_cannot_define_slots_outside_actual_storage() {
        let menu = menu();
        let mut geometry = screen();
        geometry.slots.push(Some([8.0, 26.0]));
        let roles = RecipeBookLayout::Furnace {
            input: 0,
            fuel: 1,
            result: 2,
        };
        assert!(
            validate(&[&menu], "demo:pot", roles, &geometry)
                .unwrap_err()
                .contains("nonexistent container slot 3")
        );
    }
    #[test]
    fn persistent_grid_roles_are_valid_but_computed_storage_must_agree() {
        let persistent = MenuType {
            slots: 8,
            results: vec![6],
            ..menu()
        };
        let geometry = mod_api::Screen {
            slots: vec![Some([8.0, 8.0]); 8],
            ..screen()
        };
        let roles = RecipeBookLayout::Crafting {
            width: 3,
            height: 2,
            slots: (0..6).collect(),
            result: 6,
        };
        assert!(validate(&[&persistent], "demo:pot", roles.clone(), &geometry).is_ok());
        let computed = MenuType {
            crafting: Some(horizon_mod_package::content::MenuCrafting {
                grid: 0,
                size: 2,
                result: 6,
            }),
            ..persistent
        };
        assert!(
            validate(&[&computed], "demo:pot", roles, &geometry)
                .unwrap_err()
                .contains("InvalidGrid")
        );
        let matching = RecipeBookLayout::Crafting {
            width: 2,
            height: 2,
            slots: (0..4).collect(),
            result: 6,
        };
        assert!(validate(&[&computed], "demo:pot", matching, &geometry).is_ok());
    }
}
