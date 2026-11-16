//! A remote catalogue supplies recipe membership, never the client's local recipe JSON.
use super::native_book_displays::Resources;
use super::*;
use horizon_client::recipe_book::{DisplayResources, ModBookIndex, placement_sync as sync};
use std::collections::{BTreeMap, BTreeSet};
use std::result::Result;
struct RemoteResources<'a, 'w> {
    inner: Resources<'a, 'w>,
    recipes: BTreeMap<String, String>,
    types: BTreeSet<String>,
}
impl DisplayResources for RemoteResources<'_, '_> {
    fn book(&self, key: &mod_api::RecipeBookTypeKey) -> bool {
        self.inner.book(key)
    }
    fn item(&self, id: &str) -> bool {
        self.inner.item(id)
    }
    fn stack(&self, stack: &mod_api::ItemStack) -> bool {
        self.inner.stack(stack)
    }
    fn tag(&self, id: &str) -> bool {
        self.inner.tag(id)
    }
    fn component(&self, id: &str) -> bool {
        self.inner.component(id)
    }
    fn trim_pattern(&self, id: &str) -> bool {
        self.inner.trim_pattern(id)
    }
    fn menu(&self, id: &str) -> bool {
        self.inner.menu(id)
    }
    fn menu_layout(
        &self,
        id: &str,
    ) -> Option<horizon_client::recipe_book::menu_layout::MenuLayoutMetadata> {
        self.inner.menu_layout(id)
    }
    fn recipe_type(&self, id: &str) -> bool {
        self.types.contains(id) || self.inner.recipe_type(id)
    }
    fn recipe(&self, id: &str, kind: &str) -> bool {
        self.recipes.get(id).is_some_and(|value| value == kind)
    }
}
impl Game<'_> {
    fn load_remote_mod_book_catalogue(
        &self,
        data: &sync::CatalogBytes,
    ) -> Result<ModBookIndex, sync::Refusal> {
        let declarations: Vec<mod_api::RecipeBookCategory> =
            mod_api::wire::Reader::decode(&data.bytes).map_err(|_| sync::Refusal::Invalid)?;
        let mut recipes = BTreeMap::new();
        for category in &declarations {
            if category.target_menu != data.identity.menu.as_str()
                || category.book.as_str() != data.identity.book.as_str()
            {
                return Err(sync::Refusal::Invalid);
            }
            for entry in &category.displays {
                sync::ResourceId::try_from(entry.recipe.clone())
                    .map_err(|_| sync::Refusal::Invalid)?;
                recipes.insert(entry.recipe.clone(), category.recipe_type.clone());
            }
        }
        let types = mod_api::RecipeBookLayout::registered_menus()
            .iter()
            .filter_map(|menu| mod_api::RecipeBookLayout::registered(menu).map(|(_, kind, _)| kind))
            .collect();
        let index = ModBookIndex::build(
            declarations,
            &RemoteResources {
                inner: Resources(self),
                recipes,
                types,
            },
        )
        .map_err(|error| {
            log::error!(
                "remote recipe-book catalogue {} refused: {error:?}",
                data.identity.menu.as_str()
            );
            sync::Refusal::Invalid
        })?;
        Ok(index)
    }
    pub(super) fn remote_mod_book_placement(&mut self, frame: sync::Frame) {
        match frame {
            sync::Frame::Ready { epoch, schema } => {
                if self.native_net.placement.client.ready(epoch, schema) {
                    if self.native_net.placement.client.window().is_none() {
                        self.native_net.placement.deadline = None;
                    }
                    self.ensure_mod_book_catalog();
                }
            }
            sync::Frame::Start {
                identity,
                layout,
                storage,
                total,
            } => {
                if self.native_book_placement_window().as_ref()
                    != self.native_net.placement.client.window()
                {
                    return;
                }
                let expected = mod_api::RecipeBookLayout::registered(identity.menu.as_str())
                    .map(|(_, _, layout)| layout);
                let matches = expected.as_ref() == Some(&layout)
                    && Resources(self).menu_layout(identity.menu.as_str()) == Some(storage);
                let epoch = identity.epoch;
                let started = self
                    .native_net
                    .placement
                    .client
                    .start(identity, layout, total);
                if (started.is_ok() && !matches) || started == Err(sync::Refusal::TooLarge) {
                    self.native_net.placement.client.invalidate(epoch);
                    self.native_net.placement.view = None;
                    self.reset_mod_book_placement_ui();
                    log::error!(
                        "remote recipe-book catalogue load refused: incompatible menu metadata or oversized catalogue"
                    );
                }
            }
            sync::Frame::Part {
                epoch,
                number,
                offset,
                bytes,
            } => {
                if self.native_book_placement_window().as_ref()
                    != self.native_net.placement.client.window()
                {
                    return;
                }
                let Ok(Some(data)) = self
                    .native_net
                    .placement
                    .client
                    .part(epoch, number, offset, bytes)
                else {
                    return;
                };
                match self.load_remote_mod_book_catalogue(&data) {
                    Ok(index) => {
                        if self
                            .native_net
                            .placement
                            .client
                            .accept_catalog(data.identity.clone())
                        {
                            self.native_net.placement.deadline = None;
                            self.native_net.placement.view =
                                Some(super::native_book_placement::View {
                                    identity: data.identity,
                                    index,
                                });
                            self.reset_mod_book_placement_ui();
                            self.build_screen();
                        }
                    }
                    Err(reason) => {
                        log::error!(
                            "remote recipe-book catalogue {} load refused: {reason:?}",
                            data.identity.menu.as_str()
                        );
                        self.native_net.placement.client.invalidate(epoch);
                        self.native_net.placement.view = None;
                        self.reset_mod_book_placement_ui();
                    }
                }
            }
            sync::Frame::Reply { selection, outcome } => {
                if self.native_book_placement_window().as_ref()
                    == self.native_net.placement.client.window()
                    && let Some(outcome) =
                        self.native_net.placement.client.reply(&selection, outcome)
                {
                    self.native_net.placement.deadline = None;
                    match outcome {
                        sync::Outcome::MissingIngredients => {
                            self.install_mod_book_ghost(&selection)
                        }
                        sync::Outcome::Refused(
                            sync::Refusal::Stale | sync::Refusal::UnknownDisplay,
                        ) => {
                            self.clear_mod_book_placement_screen();
                            self.ensure_mod_book_catalog();
                        }
                        _ => {}
                    }
                }
            }
            sync::Frame::Refused {
                epoch,
                number,
                window,
                nonce,
                reason,
            } => {
                if self
                    .native_net
                    .placement
                    .client
                    .refused(epoch, number, window, nonce)
                {
                    self.native_net.placement.deadline = None;
                    self.native_net.placement.view = None;
                    self.reset_mod_book_placement_ui();
                    log::error!("mod recipe-book placement load refused: {reason:?}");
                }
            }
            sync::Frame::Invalidate { epoch }
                if self.native_net.placement.client.catalog().is_some()
                    && self.native_net.placement.client.invalidate(epoch) =>
            {
                self.clear_mod_book_placement_screen();
                self.ensure_mod_book_catalog();
            }
            _ => {}
        }
    }
}
