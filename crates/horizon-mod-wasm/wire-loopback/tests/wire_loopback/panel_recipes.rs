use super::*;
use mod_api::prelude::RecipesClientHost;
use mod_api::{PanelStyle, RecipeLayout, RecipeResult, RecipeTransferCheck, RecipeView, Stack};
use std::cell::RefCell;

thread_local! {
    /// The page the game was asked to show: the mod, the title, the tab and the recipes.
    static SHOWN: RefCell<Option<(String, String, u32, Vec<RecipeView>)>> = const { RefCell::new(None) };
}

struct Client;
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(recipes));
// A boundary fixture: only the recipe page is shown.
impl RecipesClientHost for Client {
    fn panel_items(&mut self, _: &str, _: Vec<Stack>) {
        unreachable!()
    }
    fn panel_style(&mut self, _: &str, _: PanelStyle) {
        unreachable!()
    }
    fn place_recipe(&mut self, _: i32, _: bool) -> Result<(), String> {
        unreachable!()
    }
    fn transfer_recipe(&mut self, _: i32) {
        unreachable!()
    }
    fn transfer_recipe_id(&mut self, _: &str, _: bool) {
        unreachable!()
    }
    fn transfer_recipe_check(&mut self, _: i32) -> Option<RecipeTransferCheck> {
        unreachable!()
    }
    fn recipe_book_ids(&mut self, _: &str) -> Arc<[i32]> {
        unreachable!()
    }
    fn panel_recipes(
        &mut self,
        mod_id: &str,
        title: &str,
        tabs: Vec<Stack>,
        tab: u32,
        views: Vec<RecipeView>,
    ) {
        assert!(tabs.is_empty());
        SHOWN.set(Some((mod_id.into(), title.into(), tab, views)));
    }
}

#[test]
fn a_recipe_page_reaches_the_game_as_the_mod_sent_it() {
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client;
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the client and its frame outlive this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mut grid = RecipeView::new(
        RecipeLayout::Grid,
        vec![Vec::new(); 6],
        vec![RecipeResult::new(Vec::new())],
    );
    (grid.width, grid.height, grid.action) = (3, 2, Some(false));
    let mut lucky = RecipeResult::new(Vec::new());
    lucky.chance = Some(25.0);
    let mut cooking = RecipeView::new(RecipeLayout::Cooking, vec![Vec::new()], vec![lucky]);
    cooking.note = "10 s, 0.1 XP".into();
    let views = vec![grid, cooking];
    guest::Guest.panel_recipes("recipes", "Recipes: Stick", Vec::new(), 1, views.clone());
    assert_eq!(
        SHOWN.take(),
        Some(("recipes".into(), "Recipes: Stick".into(), 1, views))
    );
    guest::set_transport(None);
}
