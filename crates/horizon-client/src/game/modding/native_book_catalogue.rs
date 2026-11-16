//! Authoritative custom displays, with slot examples resolved in the server's context.
use super::api_values::TryIntoApi;
use super::*;
use horizon_client::recipe_book::{DisplayResources, placement_sync as sync};
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};
use std::result::Result;

impl Game<'_> {
    fn concretize_mod_book_slot(&self, slot: &mut Slot) -> Result<(), sync::Refusal> {
        if matches!(slot, Slot::Empty | Slot::ItemStack(_)) {
            return Ok(());
        }
        let stacks = self
            .resolve_mod_book_declaration_slot(slot)
            .into_iter()
            .map(|stack| {
                stack
                    .try_into_api()
                    .map(Slot::ItemStack)
                    .map_err(|_| sync::Refusal::Invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        *slot = Slot::Composite(stacks);
        Ok(())
    }
    fn concretize_mod_book_display(&self, display: &mut Display) -> Result<(), sync::Refusal> {
        match display {
            Display::Shaped {
                ingredients,
                result,
                station,
                ..
            }
            | Display::Shapeless {
                ingredients,
                result,
                station,
            } => {
                for slot in ingredients {
                    self.concretize_mod_book_slot(slot)?;
                }
                self.concretize_mod_book_slot(result)?;
                self.concretize_mod_book_slot(station)?;
            }
            Display::Furnace {
                ingredient,
                fuel,
                result,
                station,
                ..
            } => {
                for slot in [ingredient, fuel, result, station] {
                    self.concretize_mod_book_slot(slot)?;
                }
            }
            Display::Stonecutter {
                input,
                result,
                station,
            } => {
                for slot in [input, result, station] {
                    self.concretize_mod_book_slot(slot)?;
                }
            }
            Display::Smithing {
                template,
                base,
                addition,
                result,
                station,
            } => {
                for slot in [template, base, addition, result, station] {
                    self.concretize_mod_book_slot(slot)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn server_mod_book_catalogue(
        &mut self,
        conn: u32,
        frame: sync::Frame,
    ) -> Result<(sync::Frame, Vec<u8>), sync::Refusal> {
        use sync::Refusal;
        let sync::Frame::Catalog {
            epoch,
            number,
            window,
            nonce,
            menu,
            book,
        } = frame
        else {
            return Err(Refusal::Invalid);
        };
        let settings = self
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .ok_or(Refusal::Unsupported)?;
        if settings.epoch != epoch {
            return Err(Refusal::Stale);
        }
        if !settings.contains(book.as_str()) {
            return Err(Refusal::Unavailable);
        }
        let (bound_book, _, _) =
            mod_api::RecipeBookLayout::registered(menu.as_str()).ok_or(Refusal::Unavailable)?;
        if bound_book.as_str() != book.as_str() {
            return Err(Refusal::Unavailable);
        }
        let before = self.mod_book_menu_snapshot(conn, window, menu.as_str())?;
        self.ensure_mod_book_displays();
        let index = self
            .recipe_tables
            .books
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .ok_or(Refusal::Invalid)?;
        let layout = index
            .layout(menu.as_str())
            .ok_or(Refusal::Unavailable)?
            .declaration()
            .clone();
        let generation = index.generation();
        let mut declarations: Vec<_> = index
            .categories()
            .filter(|category| {
                category.target_menu == menu.as_str() && category.book.as_str() == book.as_str()
            })
            .cloned()
            .collect();
        for category in &mut declarations {
            category
                .displays
                .retain(|entry| before.known.contains(&entry.recipe));
            for entry in &mut category.displays {
                for display in &mut entry.displays {
                    self.concretize_mod_book_display(display)?;
                }
            }
        }
        let mut writer = mod_api::wire::Writer::new();
        writer.limit(sync::MAX_CATALOG_BYTES);
        writer.put(&declarations);
        if writer.error().is_some() {
            return Err(Refusal::TooLarge);
        }
        let bytes = writer.into_bytes();
        if !self.mod_book_menu_unchanged(&before) {
            return Err(Refusal::Changed);
        }
        let identity = sync::CatalogIdentity {
            epoch,
            number,
            window,
            nonce,
            instance: before.instance,
            generation,
            menu,
            book,
        };
        let storage = super::native_book_displays::Resources(self)
            .menu_layout(identity.menu.as_str())
            .ok_or(Refusal::Unavailable)?;
        let displays = declarations
            .iter()
            .flat_map(|category| &category.displays)
            .map(|entry| (entry.recipe.clone(), entry.displays.len() as u32))
            .collect();
        let authority = self
            .native_net
            .placement
            .servers
            .get_mut(&conn)
            .ok_or(Refusal::Unsupported)?;
        authority.peer.publish(identity.clone())?;
        authority.displays = displays;
        let start = sync::Frame::Start {
            identity,
            layout,
            storage,
            total: bytes.len() as u32,
        };
        Ok((start, bytes))
    }
}
