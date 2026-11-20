//! Opening and closing a local world from the client: the title screen's actions, entering a
//! world on the server thread, and saving and leaving it on exit.
use super::*;
use horizon_client::save::level_mods;

impl Game<'_> {
    /// `onAbort`: the confirm screen's Back or Escape reopens the Data
    /// Packs screen with the selection it had.
    pub(super) fn reopen_data_packs(&mut self, event_loop: &dyn EventLoopCtl) -> Result<()> {
        let action = self
            .title
            .as_mut()
            .and_then(|t| t.reopen_data_packs_action());
        if let Some(action) = action {
            self.title_action(event_loop, action)?;
        }
        Ok(())
    }

    /// What the title screen asked for, from a click or Enter.
    pub(super) fn title_action(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        action: Action,
    ) -> Result<()> {
        match action {
            // A splitscreen player's title never ends the process (P1's does).
            Action::Quit if self.options.split_guest => {}
            Action::Quit => {
                self.begin_closing_save(true)?;
                event_loop.set_control_flow(ControlFlow::Poll);
            }
            Action::Options => self.open_options(true),
            Action::Accessibility => {
                self.open_options_at(true, horizon_client::options_ui::Page::Accessibility)
            }
            Action::Language => {
                self.open_options_at(true, horizon_client::options_ui::Page::Language)
            }
            Action::Sounds => self.open_options_at(true, horizon_client::options_ui::Page::Sound),
            Action::Credits => {
                self.open_options_at(true, horizon_client::options_ui::Page::Credits)
            }
            Action::Mods => self.open_options_at(true, horizon_client::options_ui::Page::Mods),
            Action::OpenUri(uri) => packs::open_uri(&uri),
            Action::CopyText(text) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&text);
            }
            Action::Play { folder } => self.open_existing_world(&folder)?,
            Action::Create {
                folder,
                name,
                seed,
                creative,
                options,
            } => self.open_world(&folder, Some(&name), seed, creative, Some(&options))?,
            Action::Connect {
                address,
                lan,
                realm,
            } => self.connect_server_with_options(
                &address,
                &self.options.player_name(),
                None,
                lan,
                realm,
            )?,
            Action::DataPacks { dir, selected } => {
                self.keys.clear();
                self.set_screen(Screen::Options);
                self.options_from_title = true;
                let resources = self.vanilla_root();
                let additional = self
                    .discover_mod_packs(mod_api::PackType::ServerData)
                    .into_iter()
                    .filter_map(|pack| pack.selection)
                    .collect();
                self.ui
                    .options
                    .open_data_packs(dir, selected, resources, additional);
            }
            Action::None => {}
        }
        Ok(())
    }

    /// Enter a local world from the client: it runs on the client's server thread.
    pub(super) fn open_world(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&horizon_client::title::WorldOptions>,
    ) -> Result<()> {
        self.open_world_as(folder, name, seed, creative, create, false)
    }

    /// A world the title listed (Play, or a native mod's request answered Yes): it opens only while the
    /// save still exists, and nothing is created for a folder that is gone.
    pub(super) fn open_existing_world(&mut self, folder: &str) -> Result<()> {
        self.open_world_as(folder, None, None, false, None, true)
    }

    /// `open_world`, with `existing_only` set for a save that must already be there (see
    /// `open_existing_world`). It is checked here, before anything is read or written, and again when a
    /// deferred entry resumes, since the save may have gone in between.
    pub(super) fn open_world_as(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&horizon_client::title::WorldOptions>,
        existing_only: bool,
    ) -> Result<()> {
        let saves = std::env::current_dir()?.join("saves");
        if refuses_missing_save(&saves, folder, existing_only) {
            log::warn!("Horizon: \"{folder}\" is no longer a save; nothing opened");
            return Ok(());
        }
        // openWorldLoadLevelData: an existing world whose level data does not read opens
        // RecoverWorldDataScreen instead.
        if create.is_none()
            && let Some(title) = self.title.as_mut()
            && title.open_recover(folder)
        {
            return Ok(());
        }
        world_loading::world_data_used();
        if self.defer_world_entry(PendingWorldEntry::Local {
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned().map(Box::new),
            existing_only,
        })? {
            return Ok(());
        }
        // A world saved with mods that are not loaded now asks first (arismoko/horizon#3856), unless the player
        // already chose to open it anyway; the question reads the world and writes nothing.
        if create.is_none() {
            let dir = std::env::current_dir()?.join("saves").join(folder);
            if !level_mods::is_confirmed(&dir) {
                let missing = self.ask_missing_mods(&dir);
                if !missing.is_empty() {
                    if let Some(title) = self.title.as_mut() {
                        title.ask_missing_mods(folder, missing);
                        return Ok(());
                    }
                    for m in &missing {
                        log::warn!("Horizon: {}", level_mods::describe(m));
                    }
                }
            }
        }
        self.join_world_rest()?;
        // Create World's data packs, in the world before it opens.
        if let Some(dir) = create.and_then(|o| o.pack_dir.as_deref()) {
            let world = std::env::current_dir()?.join("saves").join(folder);
            let packs = create.map_or(&[][..], |o| &o.data_packs[..]);
            if let Err(e) = horizon_client::datapack::install(dir, &world, packs) {
                log::warn!("Horizon: data packs for {folder}: {e}");
            }
            let _ = std::fs::remove_dir_all(dir);
        }
        // The world's pack selection (`DataPackConfig`), for the pack stack
        // to read: the picked packs on, the feature packs left out off.
        // Only when `install` did not run (no Data Packs screen opened): it
        // writes the file itself, and its `disabled` must not be lost.
        if let Some(o) = create.filter(|o| o.pack_dir.is_none()) {
            let world = std::env::current_dir()?.join("saves").join(folder);
            if let Err(e) = std::fs::create_dir_all(&world) {
                log::warn!("Horizon: data packs for {folder}: {}: {e}", world.display());
            } else {
                let disabled: Vec<String> =
                    horizon_client::datapack::feature_packs(&self.vanilla_root())
                        .into_iter()
                        .map(|p| p.id)
                        .filter(|id| !o.data_packs.contains(id))
                        .collect();
                let json = serde_json::json!({"enabled": o.data_packs, "disabled": disabled});
                let file = world.join("datapacks.json");
                if let Err(e) = std::fs::write(&file, json.to_string()) {
                    log::warn!("Horizon: data packs for {folder}: {}: {e}", file.display());
                }
            }
        }
        // A new world opened without the Create screen still gets the game folder's packs.
        if create.is_none() {
            let game_dir = std::env::current_dir()?;
            let world = game_dir.join("saves").join(folder);
            if let Err(e) = horizon_client::datapack::install_new_world_packs(&game_dir, &world) {
                log::warn!("Horizon: data packs for {folder}: {e}");
            }
        }
        // IntegratedServer.initServer's line, which other launchers read in `latest.log` to
        // offer the world again.
        horizon_client::logging::write_as(
            "Server thread",
            log::Level::Info,
            &format!(
                "Starting integrated minecraft server version {}",
                horizon_core::VANILLA_VERSION
            ),
        );
        self.open_integrated(folder, name, seed, creative, create)
    }

    /// Load authoritative world state synchronously, without client entry or
    /// integrated-server selection: a test-only convenience over the same two
    /// steps production startup calls separately (`prepare_world_and_rules`,
    /// then `finish_open_world`).
    #[cfg(test)]
    pub(super) fn load_world(&mut self, request: world_loading::WorldRequest) -> Result<()> {
        let prepared = self.prepare_world_and_rules(request)?;
        self.finish_open_world(prepared)
    }

    /// Leaving the game with a world open (window closed, Quit, `quit`):
    /// menu items go back (Player.closeContainer), then the world is saved,
    /// once.
    pub(super) fn finish_perf(&mut self) {
        // One session per computer: P1's (a splitscreen player's would count it again).
        if self.options.split_guest {
            return;
        }
        let s = &self.settings;
        let settings = horizon_telemetry::payload::Settings {
            render_distance: s.render_distance.clamp(0, 255) as u8,
            simulation_distance: s.simulation_distance.clamp(0, 255) as u8,
            // 260 is the slider's "unlimited".
            max_fps: if s.max_fps >= 260 {
                0
            } else {
                s.max_fps.min(u32::from(u16::MAX)) as u16
            },
            vsync: s.vsync,
            fullscreen: s.fullscreen,
            graphics: s.graphics_preset.clone(),
        };
        // Again now: on Wayland the window is on a screen only once it has been shown.
        if let Some(window) = &self.window {
            horizon_client::telemetry::note_display(window);
        }
        self.perf
            .finish(&horizon_client::logging::game_home(), settings);
    }

    pub(super) fn save_for_exit(&mut self) -> Result<()> {
        // Native application termination can bypass CloseRequested. Finish any
        // save screen synchronously here, before the process can go away.
        while self.pending_save.is_some() {
            self.poll_closing_save()?;
            if self.pending_save.is_some() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        // A repro recording is finished and bundled before the process may go away.
        self.repro_on_exit();
        self.finish_perf();
        if self.title.is_some() || self.saved_for_exit {
            return Ok(());
        }
        // exitWorldAndClose disconnects the client from its world.
        self.client_logging_out();
        // Native application termination can bypass the close request, so make the same
        // client-side worker abort explicit before the game-owned streaming state is dropped.
        self.streaming.abort_generation();
        if let Some(server) = self.integrated.take() {
            // The server thread saves the world.
            server.stop()?;
            self.saved_for_exit = true;
            return Ok(());
        }
        if let Some(mut remote) = self.remote.take() {
            // Minecraft.disconnect closes the connection, so the host sees a clean leave.
            remote.session.disconnect();
            self.saved_for_exit = true;
            return Ok(());
        }
        self.close_container();
        if self.remote.is_none() {
            self.award_custom_stat("leave_game", 1);
        }
        self.lan_close(false);
        self.free_player_beds()?;
        self.save_game()?;
        self.saved_for_exit = true;
        Ok(())
    }

    pub(super) fn close_world(&mut self) -> Result<()> {
        self.destroying.clear();
        // Pause-menu and death-screen exits use the normal world-close action.
        // A standalone replay owns a synthetic remote session, so it must go
        // through replay cleanup instead of being treated like a server.
        if self.replay.playing() {
            self.close_studio_replay();
            return Ok(());
        }
        // A recording cannot cross into another world, and a replay feeding the
        // title screen is nonsense.
        self.repro_world_closed();
        self.clear_waypoints();
        self.post_effects.clear();
        self.spectated_post_effect = None;
        // Mod screens' living scenes go with the world their entities were copied from.
        self.live_scenes.get_mut().clear();
        if self.external_server() {
            return self.leave_server(None);
        }
        self.begin_closing_save(false)
    }
}

/// Whether an open must refuse `saves/<folder>` because it must be an existing save and is not one: nothing
/// is read or written for it. A save is one the title's list recognizes (`is_save_folder`), so an empty
/// folder is not one either; a link out of `saves/` is left to Play's symlink alert, and a mod's request
/// checks containment itself (`world_request_entry`).
fn refuses_missing_save(saves: &std::path::Path, folder: &str, existing_only: bool) -> bool {
    existing_only && !horizon_client::title::is_save_folder(saves, folder)
}

#[cfg(test)]
mod existing_save_tests {
    use super::refuses_missing_save;
    use std::path::{Path, PathBuf};

    /// Every entry under `root`, with its bytes, so a change to any file or folder shows.
    fn tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                let bytes = if path.is_dir() {
                    None
                } else {
                    Some(std::fs::read(&path).unwrap())
                };
                if path.is_dir() {
                    stack.push(path.clone());
                }
                out.push((path, bytes));
            }
        }
        out
    }

    #[test]
    fn a_save_that_is_gone_is_refused_and_the_saves_folder_is_left_as_it_was() {
        let root = std::env::temp_dir().join(format!("existing-save-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let saves = root.join("saves");
        std::fs::create_dir_all(saves.join("empty")).unwrap();
        std::fs::create_dir_all(saves.join("other")).unwrap();
        std::fs::write(saves.join("other").join("notes.txt"), b"keep").unwrap();
        let before = tree(&saves);
        // Missing, and present but without level.dat (an empty folder): both refused, and nothing created.
        assert!(refuses_missing_save(&saves, "gone", true));
        assert!(refuses_missing_save(&saves, "empty", true));
        assert_eq!(tree(&saves), before);
        assert!(!saves.join("gone").exists());
        // The flag off keeps an open's behaviour for a folder that is not a save: nothing refused here.
        assert!(!refuses_missing_save(&saves, "gone", false));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
