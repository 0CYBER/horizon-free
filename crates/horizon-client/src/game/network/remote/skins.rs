//! The tab list's players, their profiles and skins.

use super::*;

impl Game<'_> {
    /// The player list's view of the connection: the listed players (with
    /// their downloaded skins once ready), online mode, and which players
    /// are in the level with an upside-down name.
    pub(super) fn publish_tab_object_skins_body(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let fingerprint = (
            r.listed.len(),
            r.listed.iter().fold(0, |xor, uuid| xor ^ uuid),
            r.skins_revision,
            self.gui.atlas.object_skins_revision,
        );
        if r.published_skins == Some(fingerprint) {
            return;
        }
        for uuid in &r.listed {
            let Some(p) = r.tab_players.get(uuid) else {
                continue;
            };
            let skin = r.skins.get(uuid).map_or_else(
                || horizon_client::tab_list::default_skin(*uuid),
                String::as_str,
            );
            for key in [
                format!("id:{uuid:032x}"),
                format!("name:{}", p.name.to_ascii_lowercase()),
            ] {
                if object_skin_changed(&self.gui.atlas.object_skins, &key, skin) {
                    self.gui.atlas.set_object_skin(key, skin.to_owned());
                }
            }
        }
        // Publication itself mutates the atlas; cache its final revision.
        r.published_skins = Some((
            fingerprint.0,
            fingerprint.1,
            fingerprint.2,
            self.gui.atlas.object_skins_revision,
        ));
    }
    pub(super) fn remote_tab_players_body(
        &mut self,
    ) -> Option<(Vec<horizon_client::tab_list::TabPlayer>, bool, Vec<u128>)> {
        self.update_remote_skins();
        self.publish_tab_object_skins();
        let mut players: Vec<_> = {
            let r = self.remote.as_ref()?;
            r.listed
                .iter()
                .filter_map(|u| r.tab_players.get(u).cloned())
                .collect()
        };
        for p in &mut players {
            p.skin = self
                .remote
                .as_ref()
                .and_then(|r| r.skins.get(&p.uuid).cloned())
                .unwrap_or_else(|| horizon_client::tab_list::default_skin(p.uuid).to_owned());
        }
        let r = self.remote.as_ref()?;
        let upside_down = players
            .iter()
            .filter(|p| {
                (p.name == "Dinnerbone" || p.name == "Grumm")
                    && (p.uuid == r.profile || r.level_players.values().any(|u| *u == p.uuid))
            })
            .map(|p| p.uuid)
            .collect();
        Some((players, r.online_mode, upside_down))
    }

    /// Resolve inline `object:player` profiles through the same skin cache as
    /// player heads on items. The next frame uses the uploaded atlas texture.
    pub(super) fn resolve_inline_profiles_body(
        &mut self,
        profiles: &[std::sync::Arc<serde_json::Value>],
    ) {
        use horizon_client::skins;
        for profile in profiles {
            let skin = skins::head_skin(
                profile,
                &mut self.skin_profiles.borrow_mut(),
                &mut self.skin_downloads.borrow_mut(),
                |location| self.gui.atlas.sprites.contains_key(location),
            );
            let id = skins::parse_profile(profile).id;
            let name = profile
                .as_str()
                .or_else(|| profile.get("name").and_then(serde_json::Value::as_str));
            // The player list's skin for the same profile is drawn too.
            let keys = [
                Some(format!("id:{id:032x}")),
                name.map(|n| format!("name:{}", n.to_ascii_lowercase())),
            ];
            for key in keys.iter().flatten() {
                if let Some(listed) = self.gui.atlas.object_skins.get(key)
                    && listed.starts_with("skins/")
                {
                    let listed = listed.clone();
                    self.again_skin(&listed);
                }
            }
            self.gui
                .atlas
                .set_object_skin(format!("id:{id:032x}"), skin.clone());
            if let Some(name) = name {
                self.gui
                    .atlas
                    .set_object_skin(format!("name:{}", name.to_ascii_lowercase()), skin);
            }
        }
    }
    pub(super) fn poll_own_profile_body(&mut self) {
        let auth = self.auth();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        match &mut r.own_profile {
            OwnProfile::Idle => {
                let Some(auth) = auth else {
                    r.own_profile = OwnProfile::Done(None);
                    return;
                };
                let (send, recv) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("Own profile".into())
                    .spawn(move || {
                        let _ = send.send(horizon_client::auth::own_textures(auth.as_ref()));
                    })
                    .ok();
                r.own_profile = OwnProfile::Fetching(recv);
            }
            OwnProfile::Fetching(recv) => {
                if let Ok(own) = recv.try_recv() {
                    r.own_profile = OwnProfile::Done(own);
                }
            }
            OwnProfile::Done(_) => {}
        }
    }
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    /// The 3D skin layers model for a player (Options → Skin Customization,
    /// `threeDSkinLayers`): `kind`'s outer layer with one slab per painted
    /// texel, built once per (skin, slim) from the decoded image the skin
    /// downloader still holds (`horizon_client::mobs::skin_layers`). `None`
    /// while the option is off or the image is not in memory yet.
    pub(super) fn skin_3d_model_body(
        &self,
        kind: &std::sync::Arc<horizon_client::mobs::MobType>,
        skin: &str,
        slim: bool,
    ) -> Option<std::sync::Arc<horizon_client::mobs::Part>> {
        use horizon_client::mobs::skin_layers;
        if !self.settings.extras.three_d_skin_layers {
            return None;
        }
        let key = (skin.to_owned(), slim);
        if let Some(model) = self.skin_3d.borrow_mut().get(&key) {
            return Some(model);
        }
        let model_key = if slim { "player_slim#main" } else { "main" };
        let base = kind
            .layer(model_key)
            .cloned()
            .unwrap_or_else(|| kind.model.clone());
        let image = self.skin_downloads.borrow().image(skin)?.to_vec();
        let model = std::sync::Arc::new(skin_layers::extrude(
            &base,
            &image,
            skin_layers::SIZE,
            skin_layers::DEPTH,
        )?);
        self.skin_3d.borrow_mut().insert(key, model.clone());
        Some(model)
    }

    pub(super) fn update_remote_skins_body(&mut self) {
        let local = self.remote.as_ref().map(|r| r.profile);
        self.poll_own_profile();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let local = local.unwrap_or(r.profile);
        let mut entries = skin_entries(
            &r.level_players,
            &r.skin_textures,
            &mut r.level_player_textures,
        );
        let own = match &r.own_profile {
            OwnProfile::Done(Some(t)) => Some(t.clone()),
            _ => None,
        };
        // Minecraft.profileFuture: the signed-in account's active skin and cape
        // from `/minecraft/profile` when the server sent none (single player) or
        // only a skin without the equipped cape.
        if let Some(own) = own {
            if let Some((_, t)) = entries.iter_mut().find(|(u, _)| *u == local) {
                if t.skin.is_none() {
                    t.skin = own.skin.clone();
                    if own.skin.is_some() {
                        t.slim = own.slim;
                    }
                }
                if t.cape.is_none() {
                    t.cape = own.cape.clone();
                }
            } else if own.skin.is_some() || own.cape.is_some() {
                entries.retain(|(u, _)| *u != local);
                entries.push((local, own));
            }
        }
        let mut resolved: Vec<(u128, String)> = Vec::new();
        // PlayerSkin.model of each resolved skin: the textures' `slim`.
        let mut slim = std::collections::HashSet::new();
        // PlayerSkin.cape of each resolved skin.
        let mut capes: HashMap<u128, String> = HashMap::new();
        for (uuid, textures) in entries {
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned.
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                horizon_client::skins::lookup(&textures, uuid != local, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            if let Some(skin) = skin {
                if textures.slim {
                    slim.insert(uuid);
                }
                if let Some(cape) = horizon_client::skins::cape(&textures) {
                    capes.insert(uuid, cape);
                }
                resolved.push((uuid, skin));
            }
        }
        // An offline name's skin from the launcher, where neither the server
        // nor an account gives the local player one.
        if !resolved.iter().any(|(u, _)| *u == local)
            && let Some((rgba, is_slim)) = horizon_client::skins::offline_skin()
        {
            const OFFLINE: &str = "horizon/offline_skin";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.clone());
            }
            if *is_slim {
                slim.insert(local);
            }
            resolved.push((local, OFFLINE.into()));
        }
        // An offline name's cape from the launcher, where the profile has none.
        if !capes.contains_key(&local)
            && let Some(rgba) = horizon_client::skins::offline_cape()
        {
            const OFFLINE: &str = "capes/offline";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.to_vec());
            }
            capes.insert(local, OFFLINE.into());
        }
        // PlayerInfo.getSkin: the downloaded skin and its model, else the
        // profile's default skin (DefaultPlayerSkin.get), slim or wide.
        let skin_of = |skins: &HashMap<u128, String>, uuid: u128| match skins.get(&uuid) {
            Some(skin) => (skin.clone(), slim.contains(&uuid)),
            None => {
                let skin = horizon_client::tab_list::default_skin(uuid);
                (skin.to_owned(), skin.contains("/slim/"))
            }
        };
        let skins: HashMap<_, _> = resolved.into_iter().collect();
        let Some((level, own)) = self.remote.as_ref().map(|r| {
            (
                r.level_players
                    .iter()
                    .map(|(id, uuid)| (*id as u32, *uuid))
                    .collect::<Vec<_>>(),
                r.profile,
            )
        }) else {
            return;
        };
        // Only the skins drawn hold atlas slots: the level's players, not
        // every entry of a busy server's player list.
        // Build once: resolving each player must not scan every level entity.
        let players: HashMap<_, _> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:player")
            .map(|(i, m)| (m.id, i))
            .collect();
        for (id, uuid) in level {
            let Some(&i) = players.get(&id) else {
                continue;
            };
            let (skin, is_slim) = self.drawn_skin(uuid, skin_of(&skins, uuid));
            let kind = self.mobs[i].kind.clone();
            let skin_3d = self.skin_3d_model(&kind, &skin, is_slim);
            let cape = capes.get(&uuid).filter(|c| self.again_skin(c)).cloned();
            let mob = &mut self.mobs[i];
            mob.skin = Some(skin);
            mob.slim = is_slim;
            mob.skin_3d = skin_3d;
            mob.cape = cape;
        }
        if self.player_model.is_some() {
            let (skin, is_slim) = self.drawn_skin(own, skin_of(&skins, own));
            // Avatar.isModelPartShown(CAPE): the Skin Customization option.
            let cape = capes
                .get(&own)
                .filter(|_| self.settings.file.bool("modelPart_cape"))
                .filter(|c| self.again_skin(c))
                .cloned();
            let kind = self.player_model.as_ref().map(|m| m.kind.clone());
            let skin_3d = kind
                .as_ref()
                .and_then(|kind| self.skin_3d_model(kind, &skin, is_slim));
            if let Some(mob) = self.player_model.as_mut() {
                mob.skin = Some(skin);
                mob.slim = is_slim;
                mob.skin_3d = skin_3d;
                mob.cape = cape;
            }
        }
        if let Some(r) = self.remote.as_mut()
            && r.skins != skins
        {
            r.skins_revision = r.skins_revision.wrapping_add(1);
            r.skins = skins;
        }
        // ClientMannequin.updateSkin (PlayerSkinRenderCache.lookup): a
        // mannequin's DATA_PROFILE `textures` property resolves like a
        // player's, never required to be signed.
        let mannequins: Vec<(usize, horizon_client::skins::Textures)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:mannequin" && m.profile.is_some())
            .map(|(i, m)| {
                let profile = m.profile.as_ref();
                let t = profile.and_then(|p| p.properties.iter().find(|p| p.name == "textures"));
                let textures = if let Some(t) = t {
                    let signature = if t.signature.is_some() {
                        horizon_client::skins::Signature::Signed
                    } else {
                        horizon_client::skins::Signature::Unsigned
                    };
                    horizon_client::skins::unpack_known(&t.value, signature)
                } else {
                    // `ResolvableProfile.resolveProfile`: no `textures` and
                    // only a name or an id resolves through the services.
                    let key = profile.and_then(|p| match (&p.name, p.uuid) {
                        (Some(name), None) => {
                            Some(horizon_client::skins::NameOrId::Name(name.clone()))
                        }
                        (None, Some(id)) => Some(horizon_client::skins::NameOrId::Id(id)),
                        _ => None,
                    });
                    key.and_then(
                        |key| match self.skin_profiles.borrow_mut().state_for_game(&key) {
                            horizon_client::skins::Resolve::Done(Some(profile)) => {
                                profile.textures().map(|(value, signature)| {
                                    horizon_client::skins::unpack_known(&value, signature)
                                })
                            }
                            _ => None,
                        },
                    )
                    .unwrap_or_default()
                };
                (i, textures)
            })
            .collect();
        let mut resolved: Vec<(usize, Option<String>, bool)> = Vec::new();
        for (id, textures) in mannequins {
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                horizon_client::skins::lookup(&textures, false, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            let slim = skin.is_some() && textures.slim;
            resolved.push((id, skin, slim));
        }
        for (i, skin, slim) in resolved {
            let skin = skin.filter(|s| !s.starts_with("skins/") || self.again_skin(s));
            // Resolution and atlas uploads do not insert or remove mobs.
            let mob = &mut self.mobs[i];
            mob.slim = slim && skin.is_some();
            mob.skin = skin;
        }
    }
    /// Puts a downloaded skin in an atlas skin slot (`SkinSlots::place`),
    /// unless every slot holds a skin drawn in the last frame.
    pub(super) fn store_skin_body(&mut self, location: String, rgba: Vec<u8>) {
        // 3D skin layers reads the pixels back when it builds the slabs.
        self.skin_downloads
            .borrow_mut()
            .remember_image(&location, &rgba);
        let Some((slot, evicted)) = self.skin_slots.place(location.clone()) else {
            return;
        };
        if let Some(old) = evicted {
            self.gui.atlas.sprites.remove(&old);
        }
        let Some(mut rect) = self
            .gui
            .atlas
            .sprites
            .get(&format!("horizon/skin{slot}"))
            .copied()
        else {
            return;
        };
        let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
        self.map_patches.push(horizon_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width: 64,
            height: 64,
            rgba,
        });
        // A cape is a 64x32 texture in the top half of the slot (skins::process_cape).
        if location.starts_with("capes/") {
            rect.v1 = (rect.v0 + rect.v1) / 2.0;
        }
        self.gui.atlas.sprites.insert(location, rect);
    }
    /// Whether a downloaded skin or cape is in the atlas to draw, marking it
    /// drawn; if not (new, or its slot taken), its image is queued for upload
    /// again and it is drawn from the next frame.
    pub(super) fn again_skin_body(&mut self, location: &str) -> bool {
        let mut downloads = self.skin_downloads.borrow_mut();
        // A slot without its sprite (the atlas was rebuilt, or the upload
        // never happened) counts as missing, so the skin is stored again.
        if self.skin_slots.contains(location) && self.gui.atlas.sprites.contains_key(location) {
            downloads.drawn(location);
            true
        } else {
            downloads.again(location);
            false
        }
    }
    /// A player's skin and model as drawn: the downloaded skin once it is in
    /// the atlas, until then the profile's default (`DefaultPlayerSkin.get`),
    /// as `PlayerInfo.getSkin` gives before the texture has loaded.
    pub(super) fn drawn_skin_body(
        &mut self,
        uuid: u128,
        (skin, slim): (String, bool),
    ) -> (String, bool) {
        if (skin.starts_with("skins/") || skin == "horizon/offline_skin") && !self.again_skin(&skin)
        {
            let skin = horizon_client::tab_list::default_skin(uuid);
            return (skin.to_owned(), skin.contains("/slim/"));
        }
        (skin, slim)
    }
    /// `drawn_skin` for a face in the player list or spectator menu.
    pub(super) fn face_skin_body(&mut self, uuid: u128, skin: &str) -> String {
        self.drawn_skin(uuid, (skin.to_owned(), false)).0
    }
    /// ChatType.Bound.decorate (`Decoration::Chat`) and `decorateNarration`
    /// (`Decoration::Narration`): the chat type's decoration from the
    /// server's registry, applied to `content`.
    pub(super) fn decorate_chat_body(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> horizon_client::text::Text {
        use horizon_client::scoreboard::comp_text;
        let Some(r) = self.remote.as_ref() else {
            return comp_text(content);
        };
        let chat_type = r.registries.data("minecraft:chat_type", bound.chat_type);
        comp_text(&chat_type_decorate(chat_type, decoration, bound, content))
    }
}
