//! Imported modpacks, on the Mods page's Modpacks tab: import a pack, read its coverage report,
//! port what this computer can and play it as its own profile (`horizon_launcher::modpacks`).
use super::*;
use horizon_launcher::browser::horizon::pack as horizon_pack;
use horizon_launcher::modpacks::{self, Imported};
use horizon_launcher::other_launchers::Found;
use horizon_port_pack::report::Status;
use std::path::{Path, PathBuf};

/// The report's filter: every row, or one status.
pub(super) const FILTERS: [(Option<Status>, &str); 5] = [
    (None, "All"),
    (Some(Status::Ported), "Ported"),
    (Some(Status::Partial), "Partial"),
    (Some(Status::NotYet), "Not yet"),
    (Some(Status::NotNeeded), "Not needed"),
];

/// Lines of the pipeline's log the page shows while it works.
const LOG_SHOWN: usize = 6;

/// Why pack folder `work` can't be ported or started now, given the folder a start is preparing
/// and the one a Port step is rebuilding: Port rebuilds the carried mods a start reads (#18074).
pub(super) fn folder_busy(
    work: &Path,
    starting: Option<&Path>,
    porting: Option<&Path>,
) -> Option<&'static str> {
    if starting == Some(work) {
        Some("This pack is starting. Port it once the game has started.")
    } else if porting == Some(work) {
        Some("This pack is being ported. Play it once the port ends.")
    } else {
        None
    }
}

/// A list read on a thread: the imported packs (#18074), an instance's game packs, its server
/// list. Only the window's latest read applies, and only when the window changed nothing since
/// asking for it.
#[derive(Default)]
pub(super) struct PackReads {
    asked: u64,
    running: Option<u64>,
}

impl PackReads {
    /// A new read: its number, which its packs come back with.
    pub(super) fn ask(&mut self) -> u64 {
        self.asked += 1;
        self.running = Some(self.asked);
        self.asked
    }

    pub(super) fn running(&self) -> bool {
        self.running.is_some()
    }

    /// The window changed its packs (a step ended, a pack was removed): a read running began
    /// before and is out of date. True when one was, so a fresh one is to be asked for.
    pub(super) fn changed(&mut self) -> bool {
        self.running.take().is_some()
    }

    /// Whether the packs of read `read` apply: the latest, with no change since.
    pub(super) fn arrived(&mut self, read: u64) -> bool {
        let applies = self.running == Some(read);
        if applies {
            self.running = None;
        }
        applies
    }
}

impl Launcher {
    fn open_pack(&self) -> Option<&Imported> {
        let slug = self.pack_open.as_ref()?;
        self.packs.iter().find(|p| &p.slug == slug)
    }

    /// Runs a pipeline step on a thread; its lines and its end arrive as messages.
    pub(super) fn pack_job(
        &mut self,
        job: impl FnOnce(&dyn Fn(String)) -> Result<Imported, String> + Send + 'static,
    ) {
        if self.pack_busy {
            return;
        }
        self.pack_busy = true;
        self.pack_log.clear();
        self.pack_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let log = move |line: String| {
                let _ = lines.send(Msg::PackLine(line));
            };
            // A panic in a step (a pack the port didn't expect) still ends the job: the page
            // would stay busy, refusing every later step, until the launcher restarts.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&log)))
                .unwrap_or_else(|_| Err(tr("Something went wrong.").into()));
            let _ = tx.send(Msg::PackDone(result.map(Box::new)));
        });
    }

    /// A finished step: the pack's new state, opened.
    pub(super) fn pack_done(&mut self, result: Result<Box<Imported>, String>) {
        self.pack_busy = false;
        self.porting = None;
        match result {
            Ok(pack) => {
                self.mods.report_list.reset();
                self.pack_open = Some(pack.slug.clone());
                self.packs.retain(|p| p.slug != pack.slug);
                self.packs.push(*pack);
                self.packs.sort_by_key(|p| p.title().to_ascii_lowercase());
                self.packs_changed();
            }
            Err(e) => self.pack_error = Some(e),
        }
    }

    pub(super) fn choose_pack(&mut self, folder: bool, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: false,
            prompt: Some(tr("Import").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.pack_job(move |log| modpacks::import(&path, log));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Ports the open pack; `from` is a folder of the player's jars, chosen when `ask`.
    fn port_pack(&mut self, ask: bool, cx: &mut Context<Self>) {
        let Some(work) = self.open_pack().map(|p| p.work.clone()) else {
            return;
        };
        let resources = self
            .selected()
            .filter(|tag| backend::is_prepared(tag))
            .map(|tag| backend::resources(tag));
        if !ask {
            let job_work = work.clone();
            self.port_job(work, move |log| {
                modpacks::port(&job_work, resources.as_deref(), None, log)
            });
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Use these jars").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(from) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        let job_work = work.clone();
                        this.port_job(work, move |log| {
                            modpacks::port(&job_work, resources.as_deref(), Some(&from), log)
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Why pack folder `work` can't be ported or started now ([`folder_busy`]).
    pub(super) fn pack_folder_busy(&self, work: &Path) -> Option<&'static str> {
        let starting = self
            .starting_work
            .as_deref()
            .filter(|_| self.starting_client.is_some());
        folder_busy(work, starting, self.porting.as_deref())
    }

    /// Ports pack folder `work` with `job`, unless a start of it is preparing.
    fn port_job(
        &mut self,
        work: PathBuf,
        job: impl FnOnce(&dyn Fn(String)) -> Result<Imported, String> + Send + 'static,
    ) {
        if let Some(why) = self.pack_folder_busy(&work) {
            self.pack_error = Some(tr(why).into());
            return;
        }
        if self.pack_busy {
            return;
        }
        self.porting = Some(work);
        self.pack_job(job);
    }

    fn play_pack(&mut self, work: PathBuf) {
        // The pack's instance plays its own version, if it has one.
        let pack = instances::pack_for_launch(&work)
            .and_then(|instance| Ok((instance.resolve_version(None)?, instance.id)));
        let (tag, pack_id) = match pack {
            Ok(launch) => launch,
            Err(e) => {
                self.pack_error = Some(e);
                return;
            }
        };
        if !self.can_start(&tag) {
            return;
        }
        self.starting_pack = Some(work);
        self.start_client(tag, Some(pack_id));
    }

    /// Looks for other launchers' modpacks on a thread: their folders can be slow to read.
    fn scan_found_packs(&mut self) {
        if self.found_scanning {
            return;
        }
        self.found_scanning = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            use horizon_launcher::sources::{Folders, Os};
            let found = Folders::current()
                .map(|f| modpacks::found_on_this_computer(Os::current(), &f))
                .unwrap_or_default();
            let _ = tx.send(Msg::FoundPacks(found));
        });
    }

    /// Reads the imported packs, once (the sidebar and the Modpacks tab both show them), and
    /// looks again for other launchers' modpacks.
    pub(super) fn load_packs(&mut self) {
        if self.packs_loaded.is_none() {
            self.scan_found_packs();
            self.registry_packs = horizon_pack::list();
            self.reload_instances();
            // On a thread: a pack's reports and carried mods can be tens of thousands of files,
            // and the sidebar asks for them on the first frame (#18074).
            let read = self.packs_reading.ask();
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::ImportedPacks(read, modpacks::list()));
            });
            self.packs_loaded = Some(Instant::now());
        }
    }

    /// The window changed its packs: a read still running is out of date, and is asked again.
    fn packs_changed(&mut self) {
        if self.packs_reading.changed() {
            self.packs_loaded = None;
        }
    }

    /// The imported packs of read `read`; one overtaken by a later read or a change is dropped.
    pub(super) fn packs_read(&mut self, read: u64, packs: Vec<Imported>) {
        if !self.packs_reading.arrived(read) {
            return;
        }
        self.mods.report_list.reset();
        self.packs = packs;
        if self.pack_open.is_none() {
            self.pack_open = self.packs.first().map(|p| p.slug.clone());
        }
    }

    /// The imported packs, above the Modpacks tab's search.
    pub(super) fn imported_packs(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.load_packs();
        let busy = self.pack_busy;
        let pick = |id: &'static str, label: &'static str, folder: bool| {
            let b = button(id, label, Kind::Secondary, &t);
            if busy {
                disabled(b)
            } else {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_pack(folder, cx);
                    cx.notify();
                }))
            }
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(pick("import-file", "Import a pack file…", false))
            .child(pick("import-folder", "Import an instance folder…", true))
            .child(text(theme::CAPTION).w_full().text_color(t.muted).child(tr(
                ".mrpack (Modrinth), .zip (CurseForge), or a Prism, MultiMC or CurseForge instance. \
                 Horizon ports what it can on this computer, from your own downloads, and plays \
                 the pack as its own profile. Nothing is uploaded or shared.",
            )));
        let mut page = div().flex().flex_col().gap(px(16.));
        if !self.found_packs.is_empty() {
            page = page.child(self.found_pack_rows(cx));
        }
        page = page
            .child(section_title("Your modpacks", "archive", &t))
            .child(actions);
        if busy || !self.pack_log.is_empty() {
            page = page.child(self.pack_log_plate(&t));
        }
        if let Some(e) = &self.pack_error {
            page = page.child(notice(e.clone(), &t));
        }
        if !self.registry_packs.is_empty() {
            page = page.child(self.registry_pack_rows(cx));
        }
        if self.packs.is_empty()
            && (!self.registry_packs.is_empty() || self.packs_reading.running())
        {
            return page;
        }
        if self.packs.is_empty() {
            return page.child(text(theme::CAPTION).text_color(t.muted).child(tr(
                "No modpack imported yet: import one, or find one below.",
            )));
        }
        if self.packs.len() > 1 {
            let mut list = div().flex().flex_col();
            for (i, p) in self.packs.iter().enumerate() {
                let open = self.pack_open.as_ref() == Some(&p.slug);
                let slug = p.slug.clone();
                let right = if open {
                    caps(tr("Open")).text_color(t.accent).into_any_element()
                } else {
                    button(format!("open-{slug}"), "Open", Kind::Ghost, &t)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pack_open = Some(slug.clone());
                            this.mods.report_list.reset();
                            cx.notify();
                        }))
                        .into_any_element()
                };
                list = list.child(index_row(
                    i + 1,
                    p.title().to_string(),
                    Some(p.summary().into_any_element()),
                    Some(right),
                    &t,
                ));
            }
            page = page.child(list);
        }
        match self.pack_detail(cx) {
            Some(detail) => page.child(detail),
            None => page,
        }
    }

    /// Other launchers' modpacks on this computer, each imported with one click the way "Import
    /// an instance folder…" imports it, or opened once it is.
    fn found_pack_rows(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut list = div().flex().flex_col();
        for (i, found) in self.found_packs.iter().enumerate() {
            let slug = modpacks::found_slug(found);
            let id = format!("found-{i}");
            let right = if self.packs.iter().any(|p| p.slug == slug) {
                button(id, "Open", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.pack_open = Some(slug.clone());
                        this.mods.report_list.reset();
                        cx.notify();
                    },
                ))
            } else if self.pack_busy {
                disabled(button(id, "Import", Kind::Secondary, &t))
            } else {
                let folder = modpacks::found_folder(found);
                button(id, "Import", Kind::Secondary, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        let folder = folder.clone();
                        this.pack_job(move |log| modpacks::import(&folder, log));
                        cx.notify();
                    },
                ))
            };
            list = list.child(index_row(
                i + 1,
                found.name.clone(),
                Some(found_detail(found, &t).into_any_element()),
                Some(right.into_any_element()),
                &t,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(section_title("Found on this computer", "folder", &t))
            .child(list)
    }

    /// The modpacks installed from Horizon's registry: each plays its own profile.
    fn registry_pack_rows(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut list = div().flex().flex_col();
        for (i, p) in self.registry_packs.iter().enumerate() {
            let (play, remove) = (p.work.clone(), p.work.clone());
            let id = p.id.clone();
            let key = format!("horizon:{id}");
            let confirming = self.confirm_remove.as_deref() == Some(key.as_str());
            let right = div()
                .flex()
                .gap(px(8.))
                .child(
                    button(format!("play-pack-{id}"), "Play", Kind::Primary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_pack(play.clone());
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button(
                        format!("remove-pack-{id}"),
                        if confirming {
                            "Remove for good"
                        } else {
                            "Remove"
                        },
                        if confirming {
                            Kind::Secondary
                        } else {
                            Kind::Ghost
                        },
                        &t,
                    )
                    .when(confirming, |b| b.text_color(t.danger))
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if this.confirm_remove.as_ref() == Some(&key) {
                                match horizon_pack::remove(&remove) {
                                    Ok(()) => this.packs_loaded = None,
                                    Err(e) => this.pack_error = Some(e),
                                }
                                this.confirm_remove = None;
                            } else {
                                this.confirm_remove = Some(key.clone());
                            }
                            cx.notify();
                        },
                    )),
                );
            let detail = trf(
                "{version} from Horizon's registry, its own profile: {mods}",
                &[("version", &p.version), ("mods", &p.mods.join(", "))],
            );
            list = list.child(index_row(
                i + 1,
                p.name.clone(),
                Some(
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(detail)
                        .into_any_element(),
                ),
                Some(right.into_any_element()),
                &t,
            ));
        }
        list
    }

    fn pack_log_plate(&self, t: &Theme) -> Div {
        let start = self.pack_log.len().saturating_sub(LOG_SHOWN);
        plate(t)
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(16.))
            .child(
                caps(tr(if self.pack_busy { "Working" } else { "Done" }))
                    .text_color(if self.pack_busy { t.accent } else { t.muted }),
            )
            .children(self.pack_log.iter().skip(start).map(|line| {
                text(theme::CAPTION)
                    .font_family(theme::MONO)
                    .text_color(t.ink_soft)
                    .truncate()
                    .child(line.clone())
            }))
    }

    fn pack_detail(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let p = self.open_pack()?;
        let mut counts = div().flex().gap(px(12.));
        for (status, label) in FILTERS.iter().skip(1) {
            let status = status.expect("the first filter is All");
            let color = match status {
                Status::Ported => t.success,
                Status::Partial => t.accent,
                _ => t.ink,
            };
            counts = counts.child(
                plate(&t)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(4.))
                    .p(px(16.))
                    .child(
                        text(theme::DISPLAY)
                            .text_color(color)
                            .child(p.count(status).to_string()),
                    )
                    .child(caps(tr(label)).text_color(t.muted)),
            );
        }
        // Prepared, nothing preparing or running (`play_pack` checks the same).
        let can_play = self.can_start_selected();
        let mut buttons = div().flex().flex_wrap().items_center().gap(px(8.));
        let play_label = if p.mods.is_empty() {
            tr("Play").to_string()
        } else {
            trf(
                "Play with {count} mods",
                &[("count", &p.mods.len().to_string())],
            )
        };
        let mut play = button("pack-play", play_label, Kind::Primary, &t);
        if !p.can_play() || !can_play || self.pack_busy {
            play = disabled(play);
        } else {
            let work = p.work.clone();
            play = play.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_pack(work.clone());
                cx.notify();
            }));
        }
        let port_label = if p.indexed_resources == 0 {
            trf("Port {count} mods", &[("count", &p.portable.to_string())])
        } else if p.portable == 0 {
            format!("Get {} resource packs", p.indexed_resources)
        } else {
            format!(
                "Port {} mods + {} resource packs",
                p.portable, p.indexed_resources
            )
        };
        let mut port = button("pack-port", port_label, Kind::Secondary, &t);
        let mut from = button("pack-from", "Use jars from my instance…", Kind::Ghost, &t);
        if self.pack_busy || !p.can_port() || self.pack_folder_busy(&p.work).is_some() {
            port = disabled(port);
            from = disabled(from);
        } else {
            port = port.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.port_pack(false, cx);
                cx.notify();
            }));
            if p.portable == 0 {
                from = disabled(from);
            } else {
                from = from.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.port_pack(true, cx);
                    cx.notify();
                }));
            }
        }
        let confirming = self.confirm_remove.as_deref() == Some(p.slug.as_str());
        let slug = p.slug.clone();
        let work = p.work.clone();
        let remove = button(
            "pack-remove",
            if confirming {
                "Remove for good"
            } else {
                "Remove"
            },
            if confirming {
                Kind::Secondary
            } else {
                Kind::Ghost
            },
            &t,
        )
        .when(confirming, |b| b.text_color(t.danger))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if this.confirm_remove.as_ref() == Some(&slug) {
                match modpacks::remove(&work) {
                    Ok(()) => {
                        this.packs.retain(|p| p.slug != slug);
                        this.pack_open = this.packs.first().map(|p| p.slug.clone());
                        this.mods.report_list.reset();
                        this.packs_changed();
                    }
                    Err(e) => this.pack_error = Some(e),
                }
                this.confirm_remove = None;
            } else {
                this.confirm_remove = Some(slug.clone());
            }
            cx.notify();
        }));
        buttons = buttons.child(play).child(port).child(from).child(remove);
        let mut notes = vec![trf(
            "Port gets the jars of the {count} mods this computer can port today (content with data \
             and assets only) from the pack's own links, Modrinth, or your folder, and carries \
             them over. The ports stay here, in {folder}.",
            &[
                ("count", &p.portable.to_string()),
                ("folder", &short(&p.work)),
            ],
        )];
        notes.push(format!(
            "{} resource packs available. Enable them in the game's Resource Packs screen.",
            p.resourcepacks
        ));
        if let Some(jars) = p.jars() {
            notes.push(jars);
        }
        for line in &p.left_off {
            notes.push(trf("Left off: {line}.", &[("line", line)]));
        }
        if !can_play {
            notes.push(tr("Playing needs a prepared Horizon version (Play page).").into());
        }
        let mut rows: Vec<&horizon_port_pack::report::Row> = p
            .rows
            .iter()
            .filter(|r| self.pack_filter.is_none_or(|s| r.status == s))
            .collect();
        rows.sort_by(|a, b| (a.status, &a.key).cmp(&(b.status, &b.key)));
        let filter = segments("pack-filter", &FILTERS, self.pack_filter, &t, {
            let this = cx.entity().downgrade();
            move |value, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.pack_filter = value;
                    this.mods.report_list.reset();
                    cx.notify();
                });
            }
        });
        let row = |i: usize| {
            let r = rows[i];
            let (color, status_label) = match r.status {
                Status::Ported => (t.success, tr("Ported")),
                Status::Partial => (t.accent, tr("Partial")),
                Status::NotYet => (t.ink_soft, tr("Not yet")),
                Status::NotNeeded => (t.muted, tr("Not needed")),
            };
            let name = if r.name.is_empty() || r.name == r.key {
                r.key.clone()
            } else {
                format!("{}  ({})", r.name, r.key)
            };
            // What it means in plain words, then the technical terms for modders. A report an
            // older build wrote has no plain words.
            let technical = format!("{} · {} · {}: {}", r.class, r.tier, r.license, r.reason);
            let detail = if r.plain.is_empty() {
                technical.into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_color(t.ink_soft).child(modpacks::row_plain(r)))
                    .child(technical)
                    .into_any_element()
            };
            index_row(
                i + 1,
                name,
                Some(detail),
                Some(
                    caps(status_label)
                        .flex_none()
                        .text_color(color)
                        .into_any_element(),
                ),
                &t,
            )
        };
        let headline = p.headline();
        let list = self.mods.report_list.column(
            &self.content_scroll.handle,
            rows.len(),
            8.,
            |_| false,
            row,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            text(theme::TITLE)
                                .text_color(t.ink)
                                .child(p.title().to_string()),
                        )
                        .child(text(theme::UI).text_color(t.muted).child(p.summary())),
                )
                .children((!headline.is_empty()).then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .max_w(px(720.))
                        .children(headline.into_iter().enumerate().map(|(i, line)| {
                            if i == 0 {
                                text(theme::UI).text_color(t.ink).child(line)
                            } else {
                                text(theme::CAPTION).text_color(t.muted).child(line)
                            }
                        }))
                }))
                .child(counts)
                .child(buttons)
                .children(self.client_error.clone().map(|e| notice(e, &t)))
                .children(notes.into_iter().map(|n| {
                    text(theme::CAPTION)
                        .max_w(px(720.))
                        .text_color(t.muted)
                        .child(n)
                }))
                .child(div().flex().child(filter))
                .child(list),
        )
    }
}

/// A found modpack's app, Minecraft version and loader, under its name.
fn found_detail(found: &Found, t: &Theme) -> Div {
    text(theme::CAPTION)
        .text_color(t.muted)
        .child(modpacks::found_summary(found))
}

#[cfg(test)]
mod tests {
    use super::{folder_busy, PackReads};
    use std::path::Path;

    /// A pack folder being started is not ported, and one being ported is not started; other
    /// folders go on (#18074). Compared by folder, whatever the instance list holds.
    #[test]
    fn a_pack_is_never_started_and_ported_at_once() {
        let (pack, other) = (Path::new("ports/bmc2"), Path::new("ports/other"));
        assert_eq!(folder_busy(pack, None, None), None);
        assert!(folder_busy(pack, Some(pack), None)
            .unwrap()
            .contains("starting"));
        assert!(folder_busy(pack, None, Some(pack))
            .unwrap()
            .contains("ported"));
        assert_eq!(folder_busy(pack, Some(other), Some(other)), None);
    }

    /// A read the window changed its packs after (a step ended, a pack was removed) never
    /// replaces what the window has, even when its packs arrive right after the change, in the
    /// same batch of messages; the fresh read asked then applies. A read with no change since
    /// applies (#18074).
    #[test]
    fn a_read_from_before_a_change_never_applies() {
        let mut reads = PackReads::default();
        let first = reads.ask();
        assert!(reads.changed(), "a fresh read is asked for");
        assert!(!reads.arrived(first));
        assert!(!reads.running());
        let fresh = reads.ask();
        assert!(!reads.arrived(first));
        assert!(reads.arrived(fresh));
        assert!(!reads.arrived(fresh), "applied once");

        // A change with no read running asks for none; an overtaken read is dropped.
        assert!(!reads.changed());
        let older = reads.ask();
        let newer = reads.ask();
        assert!(!reads.arrived(older));
        assert!(reads.arrived(newer));
    }
}
