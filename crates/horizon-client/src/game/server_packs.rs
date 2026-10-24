//! A server's resource packs bound to the game (`ClientCommonPacketListenerImpl.
//! handleResourcePackPush` and `handleResourcePackPop`, its `PackConfirmScreen`,
//! and `DownloadedPackSource` as `Minecraft` drives it): the prompt, the
//! server list's setting, downloads into `downloads/`, and a resource reload
//! with the server's packs on top of the player's own.
use super::Game;
use horizon_client::{
    Result,
    confirm_screen::Confirm,
    hud::{HudAtlas, HudBatch},
    multiplayer, packs,
    scoreboard::{comp_text, tag_to_json},
    server_packs::{Prompt, ServerPacks, action, download, parse_sha1, valid_url},
    text::{Text, named_color},
    toasts::Slide,
    ui,
};
use horizon_protocol::{packets::configuration::ResourcePackPush, servers};
use std::{path::PathBuf, sync::mpsc::Receiver};

/// `ClientCommonPacketListenerImpl.PackConfirmScreen`: the packs it asks
/// about, whether the last one is required, and the server's message.
pub(super) struct PackPrompt {
    requests: Vec<(u128, String, String)>,
    required: bool,
    prompt: Option<Text>,
}
impl PackPrompt {
    fn texts(&self) -> (Text, Text, [Text; 2]) {
        let t = |key: &str| Text::translate(key, &[]);
        let (title, line2) = if self.required {
            let yellow = named_color("yellow").unwrap_or(0xFFFF55);
            (
                t("multiplayer.requiredTexturePrompt.line1"),
                t("multiplayer.requiredTexturePrompt.line2")
                    .with_color(yellow)
                    .bold(),
            )
        } else {
            (
                t("multiplayer.texturePrompt.line1"),
                t("multiplayer.texturePrompt.line2"),
            )
        };
        // preparePackPrompt.
        let message = match &self.prompt {
            Some(prompt) => Text::translate(
                "multiplayer.texturePrompt.serverPrompt",
                &[line2, prompt.clone()],
            ),
            None => line2,
        };
        let buttons = if self.required {
            [t("gui.proceed"), t("menu.disconnect")]
        } else {
            [t("gui.yes"), t("gui.no")]
        };
        (title, message, buttons)
    }
    fn layout(&self, atlas: &HudAtlas, size: (f32, f32)) -> Confirm {
        let (title, message, _) = self.texts();
        Confirm::new(&atlas.font, size, &title, &message)
    }
}

/// The game's `DownloadedPackSource`.
#[derive(Default)]
pub(super) struct ServerPackState {
    packs: ServerPacks,
    /// `downloads/` in the game folder, opened at the first download.
    queue: Option<download::Queue>,
    pub(super) prompt: Option<PackPrompt>,
    /// The server list entry the setting is saved to (none: a direct join).
    address: Option<String>,
    /// The account the downloads are made for (the download headers).
    user: (String, u128),
    /// The packs loaded on top of the player's own, bottom first.
    pub(super) active: Vec<packs::Pack>,
    /// `packDownloadNotifier` (`PackDownloadNotificationComponent`): the
    /// toast while packs download; `downloadFinished` hides it, a failed
    /// batch turns it into `download.pack.failed` until it slides out.
    toast: Option<PackToast>,
    toast_rx: Option<Receiver<download::Progress>>,
}
/// The download toast: vanilla's `SystemToast` `download.pack.title` with the
/// running progress, `download.pack.failed` when a batch fails.
#[derive(Default)]
struct PackToast {
    slide: Slide,
    /// `downloadedBytes`: the bytes the request in flight has written.
    bytes: u64,
    /// Its Content-Length — the progress is a percent, else a byte count.
    size: Option<u64>,
    failed: bool,
}
/// `Unit.humanReadable`, for `download.pack.progress.bytes`.
fn human_readable(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn saves() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join("saves")
}

impl Game<'_> {
    /// `ConnectScreen`: the server's setting decides what its packs do
    /// (`configureForServerControl`); single player's own server takes them
    /// all, quietly (`configureForLocalWorld`).
    pub(super) fn server_packs_connect(&mut self, address: Option<&str>, user: (String, u128)) {
        let state = &mut self.server_packs;
        state.user = user;
        state.prompt = None;
        state.toast = None;
        state.toast_rx = None;
        let Some(address) = address else {
            state.address = None;
            state.packs.local_world();
            return;
        };
        let file = multiplayer::servers_file(&saves());
        let entry = servers::load(&file).into_iter().find(|s| s.ip == address);
        let setting = entry.as_ref().and_then(|e| e.accept_textures);
        state.address = entry.map(|e| e.ip);
        state.packs.connect(Prompt::for_server(setting));
    }
    /// `Minecraft.clearDownloadedResourcePacks`: every server pack goes at once.
    pub(super) fn server_packs_disconnect(&mut self) {
        self.server_packs.prompt = None;
        self.server_packs.toast = None;
        self.server_packs.toast_rx = None;
        self.server_packs.packs.disconnect();
        self.tick_server_packs();
    }

    /// `handleResourcePackPush`.
    pub(super) fn handle_pack_push(&mut self, p: ResourcePackPush) {
        if !valid_url(&p.url) {
            self.send_pack_status(p.id, action::INVALID_URL);
            return;
        }
        let setting = self.server_setting();
        let prompt = Prompt::for_server(setting);
        if prompt != Prompt::Pending && (!p.required || prompt != Prompt::Declined) {
            self.server_packs
                .packs
                .push(p.id, &p.url, parse_sha1(&p.hash));
            return;
        }
        // addOrUpdatePackPrompt: one prompt for every pack pushed before it is answered.
        let text = p.prompt.as_ref().map(|t| comp_text(&tag_to_json(t)));
        let state = &mut self.server_packs;
        let open = state.prompt.get_or_insert(PackPrompt {
            requests: Vec::new(),
            required: p.required,
            prompt: None,
        });
        open.requests.push((p.id, p.url, p.hash));
        open.required = p.required;
        open.prompt = text;
        self.capture(false);
    }
    /// `handleResourcePackPop`: one pack, or all of them.
    pub(super) fn handle_pack_pop(&mut self, id: Option<u128>) {
        match id {
            Some(id) => self.server_packs.packs.pop(id),
            None => self.server_packs.packs.pop_all(),
        }
    }
    /// The joined server's entry in the server list: its `acceptTextures`.
    fn server_setting(&self) -> Option<bool> {
        let address = self.server_packs.address.as_deref()?;
        servers::load(&multiplayer::servers_file(&saves()))
            .into_iter()
            .find(|s| s.ip == address)?
            .accept_textures
    }
    fn send_pack_status(&self, id: u128, status: i32) {
        if let Some(r) = &self.remote {
            r.session.send_pack_status(id, status);
        }
    }

    /// The prompt's answer (`PackConfirmScreen`'s callback): the server's
    /// setting follows it, declining a required pack disconnects, and every
    /// pack it asked about is pushed.
    pub(super) fn answer_pack_prompt(&mut self, yes: bool) -> Result<()> {
        let Some(prompt) = self.server_packs.prompt.take() else {
            return Ok(());
        };
        if self.player.is_some() && self.screen == super::Screen::Playing {
            self.capture(true);
        }
        let setting = if yes {
            self.server_packs.packs.allow();
            Some(true)
        } else {
            self.server_packs.packs.reject();
            if prompt.required {
                let reason = Text::translate("multiplayer.requiredTexturePrompt.disconnect", &[]);
                return self.leave_server(Some(reason.plain()));
            }
            Some(false)
        };
        for (id, url, hash) in prompt.requests {
            self.server_packs.packs.push(id, &url, parse_sha1(&hash));
        }
        // ServerList.saveSingleServer.
        if let Some(address) = &self.server_packs.address {
            let file = multiplayer::servers_file(&saves());
            let mut list = servers::load(&file);
            for entry in list.iter_mut().filter(|s| &s.ip == address) {
                entry.accept_textures = setting;
            }
            if let Err(e) = servers::save(&file, &list) {
                log::warn!("Horizon: could not save {}: {e}", file.display());
            }
        }
        Ok(())
    }

    /// Runs what the packs need (`createUpdateScheduler`): the downloads
    /// that finished, the ones to start, a resource reload when the active
    /// packs change, and the statuses for the server.
    pub(super) fn tick_server_packs(&mut self) {
        // SystemToast.update: the toast slides for 5000 ms fully shown.
        if let Some(toast) = &mut self.server_packs.toast {
            let (sound, finished) = toast.slide.update_now(|fully| fully < 5000);
            if let Some(sound) = sound {
                self.audio.play(
                    if sound {
                        "minecraft:ui.toast.in"
                    } else {
                        "minecraft:ui.toast.out"
                    },
                    None,
                    1.0,
                    1.0,
                );
            }
            if finished {
                self.server_packs.toast = None;
            }
        }
        // A reload waits for the world's resources (`reload_packs`).
        if self.rest_loader.is_some() || self.world_loader.is_some() {
            return;
        }
        let progress = self
            .server_packs
            .toast_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for p in progress {
            let toast = &mut self.server_packs.toast;
            match p {
                download::Progress::RequestStart(size) => {
                    if let Some(t) = toast {
                        t.bytes = 0;
                        t.size = size;
                    }
                }
                download::Progress::Bytes(done) => {
                    if let Some(t) = toast {
                        t.bytes = done;
                    }
                }
                download::Progress::Finished => {
                    if toast.as_ref().is_some_and(|t| !t.failed) {
                        self.server_packs.toast = None;
                    }
                }
            }
        }
        let finished = self
            .server_packs
            .queue
            .as_ref()
            .map(download::Queue::finished)
            .unwrap_or_default();
        for batch in finished {
            let failed = !batch.failed.is_empty();
            self.server_packs.packs.downloaded(batch);
            if failed {
                // PackDownloadNotificationComponent.onFailure: the failure
                // toast replaces the progress one and stays its 5 seconds.
                self.server_packs.toast = Some(PackToast {
                    failed: true,
                    ..PackToast::default()
                });
            }
        }
        while self.server_packs.packs.wants_tick() {
            let tick = self.server_packs.packs.tick();
            if !tick.download.is_empty() {
                self.start_pack_downloads(tick.download);
            }
            if let Some(mut load) = tick.reload {
                while let Some(recovery) = {
                    let ok = self.reload_server_packs(&load);
                    self.server_packs.packs.reloaded(ok)
                } {
                    load = recovery;
                }
            }
        }
        for (id, status) in self.server_packs.packs.take_replies() {
            self.send_pack_status(id, status);
        }
    }
    fn start_pack_downloads(&mut self, requests: Vec<horizon_client::server_packs::Request>) {
        let state = &mut self.server_packs;
        if state.queue.is_none() {
            let dir = std::env::current_dir()
                .unwrap_or_default()
                .join("downloads");
            match download::Queue::open(dir) {
                Ok(queue) => state.queue = Some(queue),
                Err(e) => {
                    log::error!("Horizon: failed to open the download queue: {e}");
                    state.toast = Some(PackToast {
                        failed: true,
                        ..PackToast::default()
                    });
                    let failed = requests.iter().map(|r| r.id).collect();
                    state.packs.downloaded(horizon_client::server_packs::Batch {
                        downloaded: Vec::new(),
                        failed,
                    });
                    return;
                }
            }
        }
        let headers = download::headers(&state.user.0, state.user.1);
        // PackDownloadNotificationComponent: a toast follows the download.
        let (progress, rx) = std::sync::mpsc::channel();
        state.toast = Some(PackToast::default());
        state.toast_rx = Some(rx);
        if let Some(queue) = &state.queue {
            queue.download(requests, headers, progress);
        }
    }
    /// `DownloadedPackSource.startReload`: the packs on top of the player's
    /// own, then `Minecraft.reloadResourcePacks`. False when a pack has no
    /// readable `pack.mcmeta` or the reload failed.
    fn reload_server_packs(&mut self, load: &[(u128, PathBuf)]) -> bool {
        let packs: Option<Vec<_>> = load
            .iter()
            .map(|(id, path)| packs::server_pack(*id, path))
            .collect();
        let Some(packs) = packs else {
            log::warn!("Horizon: invalid pack metadata in a server pack, ignoring all");
            return false;
        };
        self.server_packs.active = packs;
        match self.reload_selected_packs() {
            Ok(()) => true,
            Err(e) => {
                log::warn!("Horizon: server resource pack reload failed: {e}");
                false
            }
        }
    }

    pub(super) fn draw_pack_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.server_packs.prompt else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        if self.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(
                atlas,
                0.0,
                0.0,
                w,
                h,
                horizon_client::hud::rgba(20, 20, 20, 255),
            );
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.player.is_some());
        let (_, _, [yes, no]) = prompt.texts();
        prompt
            .layout(atlas, (w, h))
            .draw(batch, atlas, self.cursor, [&yes, &no]);
    }
    /// `PackDownloadNotificationComponent`'s toast: `download.pack.title`
    /// with the running progress, or `download.pack.failed` alone.
    pub(super) fn draw_pack_toast(&self, batch: &mut HudBatch) {
        let Some(toast) = &self.server_packs.toast else {
            return;
        };
        let atlas = &self.gui.atlas;
        let t = |key: &str| Text::translate(key, &[]).plain();
        let title = if toast.failed {
            t("download.pack.failed")
        } else {
            t("download.pack.title")
        };
        let body = (!toast.failed).then(|| {
            if let Some(size) = toast.size.filter(|s| *s > 0) {
                Text::tr(
                    "download.pack.progress.percent",
                    &[&(toast.bytes * 100 / size)],
                )
                .plain()
            } else {
                Text::tr(
                    "download.pack.progress.bytes",
                    &[&human_readable(toast.bytes)],
                )
                .plain()
            }
        });
        let width = body.as_ref().map_or_else(
            || HudBatch::text_width(atlas, &title),
            |body| HudBatch::text_width(atlas, &title).max(HudBatch::text_width(atlas, body)),
        );
        let width = width.ceil().max(160.0) + 30.0;
        // Toast.xPos: in from the right edge by the visible portion.
        let x = batch.width - width * toast.slide.portion;
        horizon_client::title::toast_background(atlas, batch, x, width);
        batch.text(atlas, &title, x + 18.0, 7.0, u32::MAX, false);
        if let Some(body) = &body {
            batch.text(atlas, body, x + 18.0, 19.0, u32::MAX, false);
        }
    }
    /// A left click on the prompt: its answer when it hit a button.
    pub(super) fn pack_prompt_click(&mut self) -> Result<()> {
        let Some(prompt) = &self.server_packs.prompt else {
            return Ok(());
        };
        let answer = prompt
            .layout(&self.gui.atlas, self.gui_size())
            .answer_at(self.cursor);
        match answer {
            Some(yes) => {
                self.audio.play_button_click();
                self.answer_pack_prompt(yes)
            }
            None => Ok(()),
        }
    }
}
