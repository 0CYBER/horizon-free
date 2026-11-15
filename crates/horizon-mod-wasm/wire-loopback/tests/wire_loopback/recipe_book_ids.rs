use super::*;
use mod_api::prelude::RecipesClientHost;
use mod_api::{PanelStyle, RecipeTransferCheck, Stack};

struct Client;
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(recipes));
// A boundary reply fixture: the rest of the area is not called.
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
    fn recipe_book_ids(&mut self, recipe_id: &str) -> Arc<[i32]> {
        match recipe_id {
            "minecraft:stick" => Arc::from([1421, 1422]),
            _ => Arc::from([]),
        }
    }
}

#[test]
fn recipe_book_ids_keep_their_values_and_order_across_the_host_call() {
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
    assert_eq!(
        *guest::Guest.recipe_book_ids("minecraft:stick"),
        [1421, 1422]
    );
    assert!(guest::Guest.recipe_book_ids("minecraft:nope").is_empty());
    guest::set_transport(None);
}
