use super::*;

// no wire: frozen registration storage; each boundary value has its own Wire derive.
#[derive(Clone)]
pub(crate) struct MenuBookDeclaration {
    pub(crate) owner: &'static str,
    pub(crate) book: crate::RecipeBookTypeKey,
    pub(crate) recipe_type: String,
    pub(crate) layout: crate::RecipeBookLayout,
    pub(crate) screen: crate::Screen,
}

impl HookParts {
    pub(crate) fn menu<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn MenuPart>] {
        attached(&self.menus, id)
    }
}

/// Whether the Parts attached to the menu type `menu` subscribe to `hook`; the game
/// asks once when a menu opens, so a menu with none skips the hook while it is open.
pub fn menu_listens(hook: HookId, menu: &str) -> bool {
    if !has_part_hook(hook) {
        return false;
    }
    parts(hook, "menu routing")
        .for_hook(hook)
        .is_some_and(|table| !table.menu(|| Some(menu)).is_empty())
}

impl dyn Registrar + '_ {
    /// Attach `part` to the mod menu type `menu`.
    pub fn menu_part<P: MenuPart>(&mut self, menu: &str, part: P) {
        self.attach_menu_part(menu, type_name::<P>(), Arc::new(part));
    }
}

/// Registrar operations for menus.
pub trait MenusRegistrar {
    /// Declare this menu's book, authoritative recipe type and destination roles.
    /// The menu remains content-owned; loading checks these roles against its
    /// actual container slots and crafting declaration. `screen` is the existing
    /// typed screen value, frozen here: mapped slots need finite coordinates and
    /// its book must equal `book`. This screen owns client geometry and server
    /// hidden-slot policy; these menus do not call the dynamic screen hook.
    /// This does not turn display candidates into ingredient predicates.
    /// Invalid ownership, unknown books, duplicate menus or malformed screen
    /// geometry refuse installation, even when the caller ignores the error.
    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
        screen: crate::Screen,
    ) -> Result<(), String>;
    /// Attach `part`, of kind `kind`, to the mod menu type `menu`.
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>);
}
