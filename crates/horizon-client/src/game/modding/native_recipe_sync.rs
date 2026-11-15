//! Per-recipient recipe selection on the server thread.
use crate::game::Game;

impl Game<'_> {
    pub(in crate::game) fn prepare_recipe_join(&mut self, conn: u32) {
        self.ask_recipe_for_conn(conn);
        self.native_filter_commands(Some(conn));
    }

    /// Ask once per online player before rebuilding their recipe payloads.
    /// No online players means no calls.
    pub(in crate::game) fn sync_recipe_content(&mut self) {
        let connections: Vec<_> = self
            .lan
            .as_ref()
            .map(|host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| g.placed && g.participates_online())
                    .map(|g| g.conn)
                    .collect()
            })
            .unwrap_or_default();
        for conn in connections {
            self.ask_recipe_for_conn(conn);
        }
    }

    fn ask_for(&mut self, player: mod_api::PlayerRef) -> Vec<String> {
        let mut recipe_types = Vec::new();
        self.native_datapack_sync(mod_api::__private::DatapackSyncCall {
            player,
            recipe_types: &mut recipe_types,
        });
        recipe_types
    }

    fn ask_recipe_for_conn(&mut self, conn: u32) {
        let (Some(epoch), Some(host)) = (self.native_lifecycle.epoch(), self.lan.as_ref()) else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, g.entity_id as u32),
            g.uuid,
        );
        let requested = self.ask_for(player);
        let types = crate::crafting::mod_kinds::sync_recipe_types(&requested);
        let mut written = Vec::new();
        let registered: std::collections::HashSet<_> =
            mod_api::__private::parts_installation::recipe_serializers()
                .into_iter()
                .map(|(id, _)| id)
                .collect();
        let recipes = self.recipes();
        let values = recipes
            .crafting
            .iter()
            .filter_map(|(id, recipe)| {
                if let crate::crafting::Recipe::Mod(recipe) = recipe {
                    Some((id, recipe))
                } else {
                    None
                }
            })
            .chain(
                recipes
                    .mod_read_only
                    .iter()
                    .map(|(id, recipe)| (id, recipe)),
            );
        for (id, recipe) in values {
            if crate::crafting::mod_kinds::recipe_type(id, &recipe.serializer)
                .is_ok_and(|kind| types.contains(&kind))
            {
                match recipe.written().and_then(|text| {
                    serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())
                }) {
                    Ok(mut json) if json.is_object() => {
                        json["type"] = recipe.serializer.clone().into();
                        written.push((id.clone(), json));
                    }
                    Ok(_) => log::error!(
                        "recipe {id}, crafting kind {}: serializer wrote a non-object",
                        recipe.serializer
                    ),
                    Err(error) => {
                        log::error!("recipe {id}, crafting kind {}: {error}", recipe.serializer)
                    }
                }
            }
        }
        // Use the published generation: a failed reload must not leak candidate files.
        let mut sources: Vec<_> = recipes
            .by_type
            .iter()
            .filter(|(kind, _)| types.contains(kind))
            .flat_map(|(_, entries)| {
                entries
                    .iter()
                    .map(|entry| (entry.id.clone(), (*entry.json).clone()))
            })
            .collect();
        let written_ids: std::collections::HashSet<_> =
            written.iter().map(|(id, _)| id.as_str()).collect();
        sources.retain(|(id, json)| {
            !written_ids.contains(id.as_str())
                && !json["type"]
                    .as_str()
                    .is_some_and(|kind| registered.contains(kind))
        });
        drop(written_ids);
        sources.extend(written);
        sources.sort_by_cached_key(|(id, _)| crate::crafting::registration_key(id));
        if let Some(host) = self.lan.as_mut() {
            host.hub.update_guest(conn, |g| {
                g.recipe_sync.replace(g.brand == "horizon", types, &sources);
            });
        }
    }
}
