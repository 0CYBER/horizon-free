//! Home's Console tab: the game's output of this run as it comes, with a search, Follow (keep the
//! newest line in view), Copy, and Share log (`horizon_launcher::game_log`): an upload to mclo.gs
//! that happens only after the player confirms, without their home folder or names.
use super::servers_page::Field;
use super::*;
use crate::scrollbar::TakeWheel;
use gpui::ClipboardItem;
use horizon_launcher::game_log::{self, Shared};

pub(super) struct Console {
    follow: bool,
    /// Which lines show: every one, warnings and errors, or errors only.
    level: Level,
    /// The last run's `latest.log`, shown while this run has no lines.
    file: Option<FileLog>,
    /// A read of `file` is running off the UI thread.
    reading: bool,
    /// When `file` was last checked for a change.
    checked: Option<std::time::Instant>,
    scroll: Scrollbar,
    /// Revision, lines shown and search at the last draw: a change means scroll down when
    /// following.
    seen: (u64, usize, String),
    confirm_share: bool,
    sharing: bool,
    shared: Option<Result<Shared, String>>,
}

impl Default for Console {
    fn default() -> Self {
        Console {
            follow: true,
            level: Level::All,
            file: None,
            reading: false,
            checked: None,
            scroll: Scrollbar::default(),
            seen: (0, 0, String::new()),
            confirm_share: false,
            sharing: false,
            shared: None,
        }
    }
}

/// A file's length and modification time: a change in either means it is read again.
type Stamp = (u64, Option<std::time::SystemTime>);

/// How much of the end of `latest.log` is read.
const TAIL_BYTES: u64 = 4 << 20;

/// What the last run's log file holds, as far as it is known.
#[derive(Debug, PartialEq)]
enum Tail {
    Loading,
    /// No file, or an empty one: the game has not logged yet.
    Absent,
    /// The file is there and could not be read, with the reason in plain words.
    Failed(String),
    Lines(Vec<String>),
}

struct FileLog {
    path: std::path::PathBuf,
    stamp: Option<Stamp>,
    tail: Tail,
}

/// Reads the last `LOG_LINES` lines from the end of the file; `None` when it still has the
/// `known` stamp.
fn read_tail(
    file: &mut std::fs::File,
    known: Option<Stamp>,
) -> std::io::Result<Option<(Stamp, Vec<String>)>> {
    use std::io::{Read, Seek, SeekFrom};
    let meta = file.metadata()?;
    let stamp = (meta.len(), meta.modified().ok());
    if known == Some(stamp) {
        return Ok(None);
    }
    let start = stamp.0.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // Reading from the middle of the file starts inside a line: drop that part.
    let skip = usize::from(start > 0);
    let all: Vec<&str> = text.lines().skip(skip).collect();
    let tail = all[all.len().saturating_sub(LOG_LINES)..]
        .iter()
        .map(|l| l.to_string())
        .collect();
    Ok(Some((stamp, tail)))
}

/// The state of the log file at `path`, or `None` when it has not changed since `known`.
fn load_tail(path: &std::path::Path, known: Option<Stamp>) -> Option<(Option<Stamp>, Tail)> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some((None, Tail::Absent)),
        Err(e) => return Some((None, Tail::Failed(plain_error(&e)))),
    };
    match read_tail(&mut file, known) {
        Ok(None) => None,
        Ok(Some((stamp, lines))) if lines.is_empty() => Some((Some(stamp), Tail::Absent)),
        Ok(Some((stamp, lines))) => Some((Some(stamp), Tail::Lines(lines))),
        Err(e) => Some((None, Tail::Failed(plain_error(&e)))),
    }
}

/// An I/O error without its operating-system code: "Permission denied".
fn plain_error(e: &std::io::Error) -> String {
    let text = e.to_string();
    text.split(" (os error").next().unwrap_or(&text).to_string()
}

/// The log's level filter (the mockup's All, Warnings, Errors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    All,
    Warnings,
    Errors,
}

/// Keeps a received line, dropping the oldest past `LOG_LINES`, and counts it in `revision`.
pub(super) fn keep_line(log: &mut VecDeque<String>, revision: &mut u64, line: String) {
    log.push_back(line);
    *revision += 1;
    if log.len() > LOG_LINES {
        log.pop_front();
    }
}

/// How bad a log line is: 0 for information, 1 for a warning, 2 for an error.
fn severity(line: &str) -> u8 {
    if line.contains("/ERROR]") || line.contains("ERROR:") || line.contains(" ERROR ") {
        2
    } else if line.contains("/WARN]") || line.contains(" WARN ") || line.contains("WARN:") {
        1
    } else {
        0
    }
}

impl Launcher {
    /// What Share log sends: this run's lines, else the selected instance's latest.log.
    fn log_to_share(&self) -> (Result<String, std::path::PathBuf>, Vec<std::path::PathBuf>) {
        let home = self
            .client_home
            .clone()
            .or_else(|| self.selected_instance().map(|i| i.home.clone()))
            .unwrap_or_else(backend::game_home);
        let text = match self.prepare_log.is_empty() {
            false => Ok(self
                .prepare_log
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"),
            true => Err(home.join("logs/latest.log")),
        };
        (text, vec![home, backend::game_home()])
    }

    fn share_log(&mut self, cx: &mut Context<Self>) {
        let shared = self.log_to_share();
        self.console.confirm_share = false;
        self.console.sharing = true;
        let task = cx.background_executor().spawn(async move {
            let (source, homes) = shared;
            let text = match source {
                Ok(text) => text,
                Err(path) => match std::fs::read(&path) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(e) => {
                        return Err(trf(
                            "Could not read the log: {error}",
                            &[("error", &plain_error(&e))],
                        )
                        .to_string())
                    }
                },
            };
            let homes: Vec<&std::path::Path> = homes.iter().map(|h| h.as_path()).collect();
            let names = game_log::names(&homes);
            game_log::share(&game_log::private(
                &text,
                dirs::home_dir().as_deref(),
                &names,
            ))
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                this.console.sharing = false;
                this.console.shared = Some(done);
                cx.notify();
            });
        })
        .detach();
    }

    fn unshare_log(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(shared)) = self.console.shared.clone() else {
            return;
        };
        self.console.sharing = true;
        let task = cx
            .background_executor()
            .spawn(async move { game_log::unshare(&shared) });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                this.console.sharing = false;
                this.console.shared = match done {
                    Ok(()) => None,
                    Err(e) => Some(Err(e)),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Home's Console tab, once the game has said something (or is running): how many lines
    /// (beside the tabs) and the log.
    pub(super) fn console_section(&mut self, cx: &mut Context<Self>) -> Option<(Div, Div)> {
        let t = self.theme;
        if self.prepare_log.is_empty() && self.client.is_none() {
            return None;
        }
        let count = self.prepare_log.len();
        let header = text(theme::CAPTION)
            .text_color(t.muted)
            .child(match self.client {
                Some(_) => trf(
                    "{count} lines, the game is running",
                    &[("count", &count.to_string())],
                ),
                None => trf(
                    "{count} lines from the last run",
                    &[("count", &count.to_string())],
                ),
            });
        let mut section = div().flex().flex_col().gap(px(10.));
        let search = self.servers.value(Field::ConsoleSearch).trim().to_string();
        let lines: Vec<String> = self
            .prepare_log
            .iter()
            .filter(|l| game_log::matches(l, &search))
            .cloned()
            .collect();
        let now = (self.log_revision, count, search);
        if self.console.follow && now != self.console.seen {
            self.console.scroll.handle.scroll_to_bottom();
        }
        self.console.seen = now;
        let follow = div()
            .id("console-follow")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("console-follow-switch", self.console.follow, &t))
            .child(
                text(theme::LABEL)
                    .text_color(t.ink_soft)
                    .child(tr("Follow")),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.console.follow = !this.console.follow;
                this.console.seen.0 = u64::MAX;
                cx.notify();
            }));
        let copied = lines.join("\n");
        let tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(self.field(Field::ConsoleSearch, tr("Search the log"), 260., cx))
            .child(follow)
            .child(div().flex_1())
            .child(
                button("console-copy", "Copy", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        copy_to_clipboard(cx, ClipboardItem::new_string(copied.clone()));
                        this.console.shared = None;
                        cx.notify();
                    },
                )),
            )
            .child({
                let b = button("console-share", "Share log…", Kind::Ghost, &t);
                match self.console.sharing {
                    true => disabled(b),
                    false => b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.console.confirm_share = true;
                        cx.notify();
                    })),
                }
            })
            .child({
                let logs = self
                    .client_home
                    .clone()
                    .or_else(|| self.selected_instance().map(|i| i.home.clone()))
                    .unwrap_or_else(backend::game_home)
                    .join("logs");
                button("console-folder", "Open logs folder", Kind::Ghost, &t).on_click(
                    move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&logs);
                        crate::glass_toast::open(cx, &logs);
                    },
                )
            });
        section = section.child(tools);
        if self.console.confirm_share {
            section = section.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .child(div().flex_1().min_w_0().child(
                        text(theme::CAPTION).text_color(t.ink_soft).child(tr(
                            "Upload this log to mclo.gs, a paste service for Minecraft logs? \
                             Anyone with the link can read it, until it is deleted or unread \
                             for 90 days. Your home folder, user name and account names are \
                             taken out first.",
                        )),
                    ))
                    .child(
                        button("console-share-no", "Cancel", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.console.confirm_share = false;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("console-share-yes", "Upload", Kind::Secondary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.share_log(cx);
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        match &self.console.shared {
            _ if self.console.sharing => {
                section = section.child(
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(tr("Sending…")),
                )
            }
            Some(Ok(shared)) => {
                let (url, open_url) = (shared.url.clone(), shared.url.clone());
                section = section.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            text(theme::CAPTION)
                                .text_color(t.ink_soft)
                                .child(trf("Shared: {url}", &[("url", &url)])),
                        )
                        .child(
                            button("console-link-copy", "Copy link", Kind::Ghost, &t).on_click(
                                move |_, _, cx| {
                                    copy_to_clipboard(cx, ClipboardItem::new_string(url.clone()))
                                },
                            ),
                        )
                        .child(
                            button("console-link-open", "Open", Kind::Ghost, &t)
                                .on_click(move |_, _, cx| cx.open_url(&open_url)),
                        )
                        .child(
                            button("console-unshare", "Delete it", Kind::Ghost, &t).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.unshare_log(cx);
                                    cx.notify();
                                }),
                            ),
                        ),
                );
            }
            Some(Err(e)) => section = section.child(notice(e.clone(), &t)),
            None => {}
        }
        let shown = lines.len().saturating_sub(LOG_LINES);
        let body = div()
            .id("console-lines")
            .track_scroll(&self.console.scroll.handle)
            .h(px(320.))
            .overflow_y_scroll()
            .take_wheel(&self.console.scroll)
            .p(px(10.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .children(lines.into_iter().skip(shown).map(|line| {
                let color = if line.contains("/ERROR]") || line.contains("ERROR:") {
                    t.danger
                } else if line.contains("/WARN]") {
                    t.warn
                } else {
                    t.ink_soft
                };
                div().text_color(color).child(line)
            }));
        Some((header, section.child(body)))
    }
}

impl Launcher {
    /// Home's Console tab in the glass look: the count and the tools over the log, which is the
    /// mockup's `Logs` (round at 12, black at 25 %, Geist Mono 12 px on 1.75, white at 75 %).
    /// Not in the mockup (its Overview shows a game log only while starting): kept, in its
    /// patterns. Dark appearance only.
    pub(super) fn glass_console(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::glass_ui::{self as ui, white};
        use horizon_theme::glass::{ink, radius};
        let t = self.theme;
        let logs = self
            .client_home
            .clone()
            .or_else(|| self.selected_instance().map(|i| i.home.clone()))
            .unwrap_or_else(backend::game_home)
            .join("logs");
        let latest = logs.join("latest.log");
        // No line from this run: the last run's latest.log, its end read off the UI thread and
        // again only when it changed.
        let from_file = self.prepare_log.is_empty() && self.client.is_none();
        if from_file {
            if self.console.file.as_ref().is_none_or(|f| f.path != latest) {
                self.console.file = Some(FileLog {
                    path: latest.clone(),
                    stamp: None,
                    tail: Tail::Loading,
                });
                self.console.checked = None;
            }
            let due = self
                .console
                .checked
                .is_none_or(|c| c.elapsed() > std::time::Duration::from_secs(2));
            if due && !self.console.reading {
                self.console.reading = true;
                self.console.checked = Some(std::time::Instant::now());
                let known = self.console.file.as_ref().and_then(|f| f.stamp);
                let path = latest.clone();
                let task = cx.background_executor().spawn({
                    let path = path.clone();
                    async move { load_tail(&path, known) }
                });
                cx.spawn(async move |this, cx| {
                    let loaded = task.await;
                    let _ = this.update(cx, |this, cx| {
                        this.console.reading = false;
                        match (this.console.file.as_mut(), loaded) {
                            (Some(file), Some((stamp, tail))) if file.path == path => {
                                file.stamp = stamp;
                                file.tail = tail;
                            }
                            (Some(file), None) if file.path == path => return,
                            _ => {}
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        let tail = match (&self.console.file, from_file) {
            (Some(f), true) => Some(&f.tail),
            _ => None,
        };
        let loading = tail == Some(&Tail::Loading);
        let absent = tail == Some(&Tail::Absent);
        let failure = match tail {
            Some(Tail::Failed(e)) => Some(e.clone()),
            _ => None,
        };
        let file_revision = match (&self.console.file, from_file) {
            (
                Some(FileLog {
                    stamp: Some((len, _)),
                    ..
                }),
                true,
            ) => *len,
            _ => 0,
        };
        let source: &[String] = match tail {
            Some(Tail::Lines(lines)) => lines,
            Some(_) => &[],
            None => self.prepare_log.make_contiguous(),
        };
        let count = source.len();
        let note = match self.client {
            _ if from_file && count == 0 => tr("Game log").to_string(),
            _ if from_file => trf(
                "{count} lines from the last run",
                &[("count", &count.to_string())],
            )
            .to_string(),
            Some(_) => trf(
                "{count} lines, the game is running",
                &[("count", &count.to_string())],
            ),
            None => trf(
                "{count} lines from the last run",
                &[("count", &count.to_string())],
            ),
        };
        let search = self.servers.value(Field::ConsoleSearch).trim().to_string();
        let level = self.console.level;
        let floor = match level {
            Level::All => 0,
            Level::Warnings => 1,
            Level::Errors => 2,
        };
        let lines: Vec<String> = source
            .iter()
            .filter(|l| severity(l) >= floor && game_log::matches(l, &search))
            .cloned()
            .collect();
        let now = (
            match from_file {
                true => file_revision,
                false => self.log_revision,
            },
            count,
            search,
        );
        if self.console.follow && now != self.console.seen {
            self.console.scroll.handle.scroll_to_bottom();
        }
        self.console.seen = now;
        let follow = div()
            .id("console-follow")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(ui::toggle(
                "console-follow-switch",
                self.console.follow,
                self.look.accent,
                self.look.on_accent,
                &t,
            ))
            .child(ui::text(14., FontWeight::NORMAL, ink::BODY).child(tr("Follow")))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.console.follow = !this.console.follow;
                this.console.seen.0 = u64::MAX;
                cx.notify();
            }));
        let copied = lines.join("\n");
        // With no line there is nothing to search, copy or send: those controls show dimmed and
        // do not react.
        let nothing_to_act_on = count == 0;
        fn inert(control: impl IntoElement, off: bool) -> AnyElement {
            match off {
                true => div()
                    .relative()
                    .opacity(0.4)
                    .child(control)
                    .child(div().absolute().inset_0().occlude())
                    .into_any_element(),
                false => control.into_any_element(),
            }
        }
        let this = cx.entity().downgrade();
        let levels = ui::segments(
            "console-level",
            &[
                (Level::All, "All"),
                (Level::Warnings, "Warnings"),
                (Level::Errors, "Errors"),
            ],
            level,
            &t,
            move |pick, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.console.level = pick;
                    this.console.seen.0 = u64::MAX;
                    cx.notify();
                });
            },
        );
        let share = ui::quiet("console-share", tr("Share log…"), 36., &t)
            .tooltip(ui::tip(tr("Uploads the log to mclo.gs, after you confirm")));
        let share = match self.console.sharing {
            true => share.opacity(0.4).cursor_default(),
            false => share.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.console.confirm_share = true;
                cx.notify();
            })),
        };
        let tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .pb(px(12.))
            .child(inert(
                self.field(Field::ConsoleSearch, tr("Search the log"), 220., cx),
                nothing_to_act_on,
            ))
            .child(inert(levels, nothing_to_act_on))
            .child(inert(
                div().ml(px(8.)).child(follow).into_any_element(),
                nothing_to_act_on,
            ))
            .child(div().flex_1())
            .child(inert(
                ui::quiet("console-copy", tr("Copy"), 36., &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        copy_to_clipboard(cx, ClipboardItem::new_string(copied.clone()));
                        this.console.shared = None;
                        cx.notify();
                    },
                )),
                nothing_to_act_on,
            ))
            .child(inert(share, nothing_to_act_on))
            .child(
                ui::icon_button(
                    "console-folder",
                    "folder-01",
                    36.,
                    17.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .tooltip(ui::tip(tr("Open the logs folder")))
                .on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&logs);
                    crate::glass_toast::open(cx, &logs);
                }),
            )
            .when(latest.is_file(), |d| {
                d.child(
                    ui::quiet("console-open-file", tr("Open file"), 36., &t)
                        .on_click(move |_, _, cx| crate::glass_toast::open(cx, &latest)),
                )
            });
        let mut body = div()
            .flex()
            .flex_col()
            .child(ui::list_head(note))
            .child(tools);
        // Share log asks first, on a card as the patch log's (white at 4 %, round at 14).
        if self.console.confirm_share {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .mx(px(12.))
                    .mb(px(12.))
                    .p(px(16.))
                    .rounded(px(radius::LARGE))
                    .bg(white(0.04))
                    .child(div().flex_1().min_w_0().child(
                        ui::text(13., FontWeight::NORMAL, ink::SOFT).child(tr(
                            "Upload this log to mclo.gs, a paste service for Minecraft logs? \
                             Anyone with the link can read it, until it is deleted or unread \
                             for 90 days. Your home folder, user name and account names are \
                             taken out first.",
                        )),
                    ))
                    .child(
                        ui::quiet("console-share-no", tr("Cancel"), 36., &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.console.confirm_share = false;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        ui::solid("console-share-yes", tr("Upload"), 36., 14., &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.share_log(cx);
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        let status =
            match &self.console.shared {
                _ if self.console.sharing => Some(ui::list_head(tr("Sending…"))),
                Some(Ok(shared)) => {
                    let (url, open_url) = (shared.url.clone(), shared.url.clone());
                    Some(
                        ui::list_head(trf("Shared: {url}", &[("url", &shared.url)])).child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    ui::quiet("console-link-copy", tr("Copy link"), 36., &t)
                                        .on_click(move |_, _, cx| {
                                            copy_to_clipboard(
                                                cx,
                                                ClipboardItem::new_string(url.clone()),
                                            )
                                        }),
                                )
                                .child(
                                    ui::quiet("console-link-open", tr("Open"), 36., &t)
                                        .on_click(move |_, _, cx| cx.open_url(&open_url)),
                                )
                                .child(
                                    ui::quiet("console-unshare", tr("Delete it"), 36., &t)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.unshare_log(cx);
                                            cx.notify();
                                        })),
                                ),
                        ),
                    )
                }
                Some(Err(e)) => Some(
                    div().px(px(12.)).pb(px(8.)).child(
                        ui::text(13., FontWeight::NORMAL, ink::FULL)
                            .text_color(ui::danger())
                            .child(e.clone()),
                    ),
                ),
                None => None,
            };
        let shown = lines.len().saturating_sub(LOG_LINES);
        let empty = lines.is_empty();
        let filtered = count > 0;
        // The terminal (the mockup's `logbox`): black under the glass, Geist Mono 12 on 1.7, the
        // warnings and errors in their colours, a caret after the last line while the game runs.
        let title = if from_file { "latest.log" } else { "horizon" };
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(32.))
            .px(px(12.))
            .border_b_1()
            .border_color(white(0.08))
            .child(
                div()
                    .flex_1()
                    .text_center()
                    .font_family(theme::MONO)
                    .text_size(px(11.5))
                    .text_color(white(ink::QUIET))
                    .child(title),
            );
        let log = div()
            .id("console-lines")
            .track_scroll(&self.console.scroll.handle)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .take_wheel(&self.console.scroll)
            .px(px(14.))
            .py(px(10.))
            .font_family(theme::MONO)
            .text_size(px(12.))
            .line_height(px(20.))
            .children(lines.into_iter().skip(shown).map(|line| {
                let color = match severity(&line) {
                    2 => ui::danger(),
                    1 => ui::warn(),
                    _ => white(0.75),
                };
                div().text_color(color).child(line)
            }))
            .when(empty, |d| {
                d.child(
                    div()
                        .text_color(white(ink::DIM))
                        .child(match (&failure, filtered) {
                            (_, true) => {
                                tr("No line matches. Clear the search or pick All.").to_string()
                            }
                            (Some(e), _) => {
                                trf("Could not read the last game log: {error}", &[("error", e)])
                            }
                            _ if loading => tr("Reading the last game log…").to_string(),
                            _ if absent => {
                                tr("No game log yet. It shows here as soon as the game starts.")
                                    .to_string()
                            }
                            _ => tr("The game's log shows here as soon as it starts.").to_string(),
                        }),
                )
            })
            .when(failure.is_some() && !filtered, |d| {
                d.child(div().mt(px(10.)).child(
                    ui::quiet("console-retry", tr("Retry"), 36., &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.console.file = None;
                            cx.notify();
                        },
                    )),
                ))
            })
            .when(self.client.is_some(), |d| {
                d.child(
                    div()
                        .mt(px(2.))
                        .w(px(7.))
                        .h(px(14.))
                        .bg(ui::color(ui::look().accent)),
                )
            });
        let terminal = div()
            .flex()
            .flex_col()
            .h(px(380.))
            .mx(px(12.))
            .mb(px(12.))
            .rounded(px(12.))
            .overflow_hidden()
            .bg(gpui::black().opacity(0.42))
            .border_1()
            .border_color(white(0.12))
            .child(bar)
            .child(log);
        Some(body.children(status).child(terminal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_sees_new_lines_after_the_buffer_is_full() {
        let mut launcher_revision = 0u64;
        let mut log = VecDeque::new();
        let mut last = (launcher_revision, log.len());
        for n in 0..LOG_LINES + 3 {
            keep_line(&mut log, &mut launcher_revision, n.to_string());
            let now = (launcher_revision, log.len());
            assert_ne!(now, last, "line {n} must change what follow compares");
            last = now;
        }
        assert_eq!(log.len(), LOG_LINES);
    }

    #[test]
    fn tail_is_bounded_and_tells_absence_from_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("latest.log");
        assert_eq!(load_tail(&path, None), Some((None, Tail::Absent)));

        let line = "x".repeat(99) + "\n";
        std::fs::write(&path, line.repeat(60_000)).unwrap(); // about 6 MB
        let (stamp, tail) = load_tail(&path, None).unwrap();
        let Tail::Lines(lines) = tail else {
            panic!("read")
        };
        assert_eq!(lines.len(), LOG_LINES);
        assert!(lines.iter().all(|l| l.len() == 99));
        assert_eq!(load_tail(&path, stamp), None);

        // A folder where the file should be is not "no log yet".
        let folder = dir.path().join("dir.log");
        std::fs::create_dir(&folder).unwrap();
        assert!(matches!(
            load_tail(&folder, None),
            Some((None, Tail::Failed(_)))
        ));
    }
}
