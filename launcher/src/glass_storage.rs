//! Cached storage inventory; filesystem work never runs on the render thread.
use super::*;
use crate::glass_ui as ui;
use horizon_launcher::browser;
use storage::Category;

/// What a Clear click asks to confirm, and what answering does: `Some(clear)` starts that
/// cleanup, `None` (Cancel) clears nothing. Either answer ends the question.
fn answer(pending: &mut Option<Option<Category>>, accept: bool) -> Option<Option<Category>> {
    let asked = pending.take()?;
    accept.then_some(asked)
}

/// Whether Settings counts storage now: when nothing was counted yet, or when the last count
/// failed in an earlier visit (`tried_this_visit` false). A count that failed (a build or the
/// startup cleanup held the build lock) is tried again when Storage is opened again (#19481),
/// once per visit rather than on every frame.
pub(crate) fn count_now(
    counted: bool,
    pending: bool,
    failed: bool,
    tried_this_visit: bool,
) -> bool {
    !pending && if failed { !tried_this_visit } else { !counted }
}

/// Whether the cache limit moving from `old` (unset: the default) to `new` (0: no limit) lowers
/// or sets the cap, so that what is over it should be freed now rather than at the next build or
/// start.
fn tightens(old: Option<u64>, new: u64) -> bool {
    let old = old.unwrap_or(storage::policy::DEFAULT_LIMIT_GIB);
    new != 0 && (old == 0 || new < old)
}

/// What a size reads before it is known: counting, or not counted when the count failed.
fn counted(bytes: Option<u64>, pending: bool) -> String {
    match bytes {
        Some(bytes) => storage::format_bytes(bytes),
        None if pending => tr("Calculating…").to_string(),
        None => tr("Not counted").to_string(),
    }
}

/// The cleanups lowered limits ask for, by generation: each lower or new cap asks for one,
/// and one that ran settles the requests made before it started. A cleanup the build lock
/// refused settles nothing, and one that started before a newer limit was saved leaves that
/// limit's cleanup owed (#19481).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Trims {
    asked: u64,
    settled: u64,
}

impl Trims {
    fn owed(self) -> bool {
        self.asked > self.settled
    }

    /// The limit moved from `old` (unset: the default) to `new` (0: no limit), already saved.
    fn limit_moved(&mut self, old: Option<u64>, new: u64) {
        if new == 0 {
            self.settled = self.asked;
        } else if tightens(old, new) {
            self.asked += 1;
        }
    }

    /// The generation a cleanup starting now settles. It reads the limit when it runs, after
    /// every limit saved up to now.
    fn start(self) -> u64 {
        self.asked
    }

    /// A cleanup started at `generation` ended.
    fn ended(&mut self, generation: u64, succeeded: bool) {
        if succeeded {
            self.settled = self.settled.max(generation);
        }
    }
}

/// Whether a count asked for (no Clear) runs the owed cleanup first. A cleanup the build lock
/// refused stays owed, so Recount or the next visit runs it rather than only counting (#19481).
fn trims(owed: bool, clear: Option<Category>, all: bool) -> bool {
    owed && clear.is_none() && !all
}

/// What a Clear of `asked` (None: Clear all) can free at most: the row's size, or for all, the
/// report's total, where a unit listed under two rows counts once (#19481).
fn up_to(report: &storage::Report, asked: Option<Category>) -> u64 {
    match asked {
        None => report.remaining_bytes,
        Some(category) => report
            .rows
            .iter()
            .filter(|row| row.category == category)
            .map(|row| row.remaining_bytes)
            .sum(),
    }
}

impl Launcher {
    /// Counts storage (after a Clear, when asked). With a lowered limit's cleanup owed, a plain
    /// count frees what is over the limit first, then counts, under one hold of the build lock so
    /// the count cannot find that cleanup busy.
    pub(super) fn refresh_storage(&mut self, clear: Option<Category>, all: bool) {
        if self.storage_pending {
            return;
        }
        self.storage_trimming =
            trims(self.storage_trims.owed(), clear, all).then(|| self.storage_trims.start());
        if self.storage_trimming.is_some() {
            self.storage_work(|storage| {
                storage.evict()?;
                storage.report(None, false)
            });
        } else {
            self.storage_work(move |storage| storage.report(clear, all));
        }
    }

    fn storage_work(
        &mut self,
        work: impl FnOnce(&storage::Storage) -> Result<storage::Report, storage::Error> + Send + 'static,
    ) {
        self.storage_pending = true;
        self.storage_tried = self.storage_visit;
        // The last failure is stale once the count is tried again: the running count shows instead.
        self.storage_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = storage::Storage::open()
                .and_then(|storage| work(&storage))
                .map(Box::new)
                .map_err(native_mods::cache_error);
            let _ = tx.send(Msg::Storage(result));
        });
    }

    /// A finished measurement or Clear from the worker.
    pub(super) fn storage_arrived(&mut self, result: Result<Box<storage::Report>, String>) {
        self.storage_pending = false;
        let trimming = self.storage_trimming.take();
        let succeeded = result.is_ok();
        if let Some(generation) = trimming {
            self.storage_trims.ended(generation, succeeded);
        }
        match result {
            Ok(report) => {
                self.storage_report = Some(*report);
                self.storage_error = None;
            }
            Err(error) => self.storage_error = Some(error),
        }
        // Asked for while other work ran, or a newer limit saved while a cleanup ran: it runs
        // now. One that just failed waits for Recount or the next visit, not a retry loop
        // against a held lock.
        if self.storage_trims.owed() && (trimming.is_none() || succeeded) {
            self.refresh_storage(None, false);
        }
    }

    pub(super) fn glass_storage(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let blocked = self.storage_pending
            || self.storage_error.as_deref()
                == Some(tr(
                    "A native build or cache cleanup is running. Try again when it finishes.",
                ));
        let mut card = div().flex().flex_col().p(px(8.)).gap(px(12.));
        let total_bytes = self
            .storage_report
            .as_ref()
            .map(|report| up_to(report, None));
        let total = counted(total_bytes, self.storage_pending);
        // Nothing to clear is not a press: Clear dims at nothing, as while a build runs, and
        // while nothing is counted it can't say what it would free.
        let all_blocked = blocked || total_bytes.is_none_or(|bytes| bytes == 0);
        card = card.child(ui::setting_row(
            tr("Launcher caches"),
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(total)
                    .child(tr(
                        "Downloads and build files the launcher can fetch or make again. Your \
                         worlds, instances and settings are never touched.",
                    ))
                    .into_any_element(),
            ),
            Some(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button("storage-refresh", "Recount", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.refresh_storage(None, false);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("storage-clear-all", "Clear all", Kind::Secondary, &t)
                            .opacity(if all_blocked { 0.4 } else { 1.0 })
                            .when(all_blocked, |b| b.cursor_default())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !all_blocked {
                                    this.storage_confirm = Some(None);
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            ),
        ));
        let limit = segments(
            "storage-limit",
            &[
                (10u64, "10 GB"),
                (25, "25 GB"),
                (50, "50 GB"),
                (100, "100 GB"),
                (0, "No limit"),
            ],
            self.settings
                .cache_limit_gib
                .unwrap_or(storage::policy::DEFAULT_LIMIT_GIB),
            &t,
            {
                let this = cx.entity().downgrade();
                move |value, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        let old = this.settings.cache_limit_gib.replace(value);
                        match this.settings.save() {
                            // The cleanup reads the saved limit.
                            Ok(()) => {
                                this.storage_trims.limit_moved(old, value);
                                if this.storage_trims.owed() {
                                    this.refresh_storage(None, false);
                                }
                            }
                            // Unsaved, the limit stays what it was: picking it again is a change.
                            Err(error) => {
                                this.settings.cache_limit_gib = old;
                                this.storage_error = Some(error);
                            }
                        }
                        cx.notify();
                    });
                }
            },
        );
        card = card.child(ui::setting_row(
            tr("Cache limit"),
            None,
            Some(limit.into_any_element()),
        ));
        if self.client.is_some() {
            card = card.child(tr("A game is running. Files it may use are kept."));
        }
        if self.storage_pending {
            card = card.child(tr("A build or storage operation is running."));
        }
        if let Some(error) = &self.storage_error {
            card = card.child(error.clone());
        }
        if let Some(asked) = self.storage_confirm {
            card = card.child(self.storage_confirmation(asked, cx));
        }
        let summary = card;
        let mut card = crate::glass::plate(horizon_theme::glass::radius::PANEL)
            .flex()
            .flex_col()
            .p(px(8.));
        for category in Category::ALL {
            let label = category_label(category);
            let row = self
                .storage_report
                .as_ref()
                .and_then(|report| report.rows.iter().find(|row| row.category == category));
            let mut detail = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(counted(
                    row.map(|row| row.remaining_bytes),
                    self.storage_pending,
                ))
                .child(tr(category_about(category)));
            let row_blocked = blocked || row.is_none_or(|row| row.remaining_bytes == 0);
            if let Some(row) = row {
                for kept in &row.kept {
                    detail = detail.child(div().child(trf(
                        "Space it couldn't free: {bytes}. {reason}",
                        &[
                            ("bytes", &storage::format_bytes(kept.bytes)),
                            ("reason", &kept.reason),
                        ],
                    )));
                }
            }
            card = card.child(ui::setting_row(
                tr(label),
                Some(detail.into_any_element()),
                Some(
                    button(
                        SharedString::from(format!("storage-clear-{category:?}")),
                        "Clear",
                        Kind::Secondary,
                        &t,
                    )
                    .opacity(if row_blocked { 0.4 } else { 1.0 })
                    .when(row_blocked, |b| b.cursor_default())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !row_blocked {
                            this.storage_confirm = Some(Some(category));
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                ),
            ));
        }
        let breakdown = card;
        let mut card = crate::glass::plate(horizon_theme::glass::radius::PANEL).p(px(8.));
        if self.settings.native_mods_allowed_at.is_some() {
            // Turn off asks first: it switches mods off in every instance, and each comes back
            // only from its own instance. Counting and switching off read every instance's
            // mods, so both run off the window's thread.
            let (detail, actions) = match self.storage_revoke {
                None => (
                    tr(
                        "Allowed: they run with access to your computer. Turn off takes the \
                        permission back; Horizon asks again before the next build.",
                    )
                    .to_string(),
                    div().child(match self.storage_counting {
                        // Counting: the button waits, so a second press starts nothing.
                        Some(_) => disabled(button(
                            "storage-revoke",
                            "Counting your native mods…",
                            Kind::Secondary,
                            &t,
                        )),
                        None => button("storage-revoke", "Turn off", Kind::Secondary, &t)
                            .on_click(cx.listener(|this, _, _, cx| this.storage_count_native(cx))),
                    }),
                ),
                Some((mods, instances_)) => (
                    match (mods, instances_) {
                        (0, _) => tr("No native mods are switched on. Turning off only takes \
                                      the permission back; Horizon asks again before the next \
                                      build.")
                        .to_string(),
                        (1, _) => tr("This takes the permission back and switches off your \
                                      one native mod. Turn it back on from its instance's \
                                      Installed mods.")
                        .to_string(),
                        (_, 1) => trf(
                            "This takes the permission back and switches off {mods} native \
                             mods. Turn them back on from the instance's Installed mods.",
                            &[("mods", &mods.to_string())],
                        ),
                        _ => trf(
                            "This takes the permission back and switches off {mods} native \
                             mods in {instances} instances. Turn them back on from each \
                             instance's Installed mods.",
                            &[
                                ("mods", &mods.to_string()),
                                ("instances", &instances_.to_string()),
                            ],
                        ),
                    },
                    // Keeping them is the leading choice, as on the other questions.
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("storage-revoke-yes", "Turn off", Kind::Secondary, &t).on_click(
                                cx.listener(|this, _, _, cx| this.turn_off_native_mods(cx)),
                            ),
                        )
                        .child(
                            button(
                                "storage-revoke-cancel",
                                // With nothing switched on, what stays is the permission.
                                match mods {
                                    0 => "Keep the permission",
                                    1 => "Keep it",
                                    _ => "Keep them",
                                },
                                Kind::Primary,
                                &t,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.storage_revoke = None;
                                cx.notify();
                            })),
                        ),
                ),
            };
            // A Turn off that didn't finish says so here, where it was pressed.
            let detail = div().child(detail).children(
                self.storage_revoke_error
                    .clone()
                    .map(|error| div().text_color(ui::danger()).child(error)),
            );
            card = card.child(ui::setting_row(
                tr("Native mods"),
                Some(detail.into_any_element()),
                Some(actions.into_any_element()),
            ));
        } else {
            card = card.child(ui::setting_row(
                tr("Native mods"),
                Some(
                    div()
                        .child(tr(
                            "Not allowed: Horizon asks before it builds the first one.",
                        ))
                        .children(
                            self.storage_revoke_error
                                .clone()
                                .map(|error| div().text_color(ui::danger()).child(error)),
                        )
                        .into_any_element(),
                ),
                None,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(summary)
            .child(breakdown)
            .child(card)
    }

    /// Counts the switched-on native mods and the instances holding them, on a worker, then
    /// asks before Turn off.
    fn storage_count_native(&mut self, cx: &mut Context<Self>) {
        let visit = self.storage_visit;
        self.storage_counting = Some(visit);
        self.storage_revoke_error = None;
        let task = cx.background_executor().spawn(async move {
            let counts: Vec<usize> = instances::list()
                .iter()
                .map(|i| native_status::enabled(&i.home).len())
                .filter(|n| *n > 0)
                .collect();
            (counts.iter().sum::<usize>(), counts.len())
        });
        cx.spawn(async move |this, cx| {
            let counted = task.await;
            let _ = this.update(cx, |this, cx| {
                // A count from a visit the player has left asks nothing.
                if this.storage_counting == Some(visit) {
                    this.storage_counting = None;
                    this.storage_revoke = Some(counted);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Takes the permission back (saved first: nothing changes if it can't be), then switches
    /// every native mod off, on a worker; a build card still open is cleared, so its buttons ask
    /// again rather than fail.
    fn turn_off_native_mods(&mut self, cx: &mut Context<Self>) {
        self.storage_revoke = None;
        if let Err(error) = self.settings.revoke_native_mods() {
            self.storage_revoke_error = Some(error);
            cx.notify();
            return;
        }
        self.native_error = None;
        self.native_worlds = None;
        self.native_failed_mods.clear();
        // A build waiting for a preparation is dropped now, not asked about later.
        self.native_look_again(None, None);
        let task = cx.background_executor().spawn(async move {
            let mut failure = None;
            for instance in instances::list() {
                for (id, _, _) in native_status::enabled(&instance.home) {
                    if let Err(error) = browser::java::set_enabled(&instance.home, &id, false) {
                        failure.get_or_insert((id, instance.name.clone(), error));
                    }
                }
            }
            failure
        });
        cx.spawn(async move |this, cx| {
            let failure = task.await;
            let _ = this.update(cx, |this, cx| {
                this.storage_revoke_error = failure.map(|(id, instance, error)| {
                    trf(
                        "The permission is off, but {mod} in {instance} couldn't be switched off \
                         ({error}). Switch it off in {instance}'s Installed mods.",
                        &[
                            ("mod", &id),
                            ("instance", &instance),
                            ("error", error.trim_end_matches('.')),
                        ],
                    )
                });
                this.instance_mods_changed();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The question before a manual Clear: which files, and how much space they hold now.
    fn storage_confirmation(&self, asked: Option<Category>, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let bytes = self
            .storage_report
            .as_ref()
            .map_or(0, |report| up_to(report, asked));
        let size = storage::format_bytes(bytes);
        let question = match asked {
            None => trf(
                "Clear all launcher caches? Up to {size} can be freed. Files a game or build uses are kept.",
                &[("size", &size)],
            ),
            Some(category) => trf(
                "Clear {category}? Up to {size} can be freed. Files a game or build uses are kept.",
                &[("category", tr(category_label(category))), ("size", &size)],
            ),
        };
        div().flex().flex_col().gap(px(8.)).child(question).child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    button("storage-confirm-cancel", "Cancel", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _, _, cx| {
                            answer(&mut this.storage_confirm, false);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button("storage-confirm-clear", "Clear", Kind::Primary, &t).on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(clear) = answer(&mut this.storage_confirm, true) {
                                this.refresh_storage(clear, clear.is_none());
                            }
                            cx.notify();
                        }),
                    ),
                ),
        )
    }
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::BuildFiles => "Build files",
        Category::PrebuiltGames => "Prebuilt games",
        Category::SpareGameBuilds => "Spare game builds",
        Category::RustCompiler => "Rust compiler",
        Category::OtherCaches => "Other caches",
    }
}

/// What a row holds, and that clearing it costs only a download or a rebuild.
fn category_about(category: Category) -> &'static str {
    match category {
        Category::BuildFiles => "What building native mods leaves behind: made again when needed.",
        Category::PrebuiltGames => {
            "The game packages native mods are built against: downloaded again when needed."
        }
        Category::SpareGameBuilds => {
            "Game builds kept for running servers: downloaded again when needed."
        }
        Category::RustCompiler => {
            "The compiler that builds native mods: downloaded again the next time one is built."
        }
        Category::OtherCaches => "Other downloads kept to make the launcher faster.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #19481: a count that failed (the build lock busy) is tried again on the next visit,
    /// once, and a count that worked is not repeated.
    #[test]
    fn a_failed_count_is_tried_again_on_the_next_visit_only() {
        assert!(count_now(false, false, false, false), "first visit counts");
        assert!(!count_now(false, true, false, false), "one count at a time");
        assert!(
            !count_now(true, false, false, false),
            "counted stays counted"
        );
        assert!(
            count_now(false, false, true, false),
            "failed earlier: again"
        );
        assert!(count_now(true, false, true, false), "a failed Clear: again");
        assert!(
            !count_now(false, false, true, true),
            "failed this visit: not every frame"
        );
    }

    /// #19481: a lowered limit's cleanup the build lock refused stays owed, and the next count
    /// (Recount, the next visit) runs it; only a cleanup that ran, or no limit, settles it.
    #[test]
    fn a_refused_cleanup_stays_owed_until_it_runs() {
        let mut trims_ = Trims::default();
        trims_.limit_moved(Some(25), 10);
        assert!(trims_.owed());
        let busy = trims_.start();
        trims_.ended(busy, false);
        assert!(trims_.owed(), "a busy cleanup is not dropped");
        assert!(
            trims(trims_.owed(), None, false),
            "Recount and a revisit run it"
        );
        assert!(
            !trims(trims_.owed(), Some(Category::RustCompiler), false),
            "a Clear stays a Clear"
        );
        trims_.limit_moved(Some(10), 10);
        assert!(trims_.owed(), "picking the same limit keeps it owed");
        let ran = trims_.start();
        trims_.ended(ran, true);
        assert!(!trims_.owed(), "a cleanup that ran settles it");
        trims_.limit_moved(Some(10), 50);
        assert!(!trims_.owed(), "a higher limit asks nothing");
        trims_.limit_moved(Some(50), 25);
        trims_.limit_moved(Some(25), 0);
        assert!(!trims_.owed(), "no limit settles what was owed");
    }

    /// #19481: a cleanup that started at 25 GB and ends after 10 GB was saved leaves the 10 GB
    /// cleanup owed.
    #[test]
    fn a_newer_limit_outlasts_the_cleanup_already_running() {
        let mut trims_ = Trims::default();
        trims_.limit_moved(None, 25);
        trims_.limit_moved(Some(50), 25);
        let running = trims_.start();
        trims_.limit_moved(Some(25), 10);
        trims_.ended(running, true);
        assert!(trims_.owed());
        let next = trims_.start();
        trims_.ended(next, true);
        assert!(!trims_.owed());
    }

    /// #19481: Clear all offers the report's total, which counts a unit in two rows once, not
    /// the sum of the rows.
    #[test]
    fn clear_all_offers_the_total_not_the_rows_sum() {
        let row = |category, remaining_bytes| storage::Row {
            category,
            bytes: remaining_bytes,
            freed_bytes: 0,
            remaining_bytes,
            kept: Vec::new(),
        };
        let report = storage::Report {
            rows: vec![
                row(Category::RustCompiler, 300),
                row(Category::OtherCaches, 200),
            ],
            total_bytes: 400,
            remaining_bytes: 400,
        };
        assert_eq!(up_to(&report, None), 400);
        assert_eq!(up_to(&report, Some(Category::OtherCaches)), 200);
    }

    #[test]
    fn a_size_not_being_counted_does_not_read_calculating() {
        assert_eq!(counted(None, true), tr("Calculating…"));
        assert_eq!(counted(None, false), tr("Not counted"));
        assert_ne!(counted(None, false), counted(None, true));
        assert_eq!(
            counted(Some(2_000_000), false),
            storage::format_bytes(2_000_000)
        );
    }

    /// #19481: lowering the limit frees what is over it now; raising or removing it frees nothing.
    #[test]
    fn a_lower_or_new_limit_cleans_up_and_a_higher_one_does_not() {
        assert!(tightens(None, 10), "below the default");
        assert!(!tightens(None, storage::policy::DEFAULT_LIMIT_GIB));
        assert!(!tightens(None, 50));
        assert!(tightens(Some(100), 50));
        assert!(tightens(Some(0), 100), "no limit to a limit");
        assert!(!tightens(Some(10), 0), "a limit to no limit");
        assert!(!tightens(Some(10), 25));
        assert!(!tightens(Some(10), 10));
    }

    #[test]
    fn cancel_clears_nothing_and_accept_clears_what_was_asked() {
        let mut pending = Some(Some(Category::PrebuiltGames));
        assert_eq!(answer(&mut pending, false), None);
        assert_eq!(pending, None);
        assert_eq!(
            answer(&mut pending, true),
            None,
            "nothing asked, nothing cleared"
        );
        let mut pending = Some(None);
        assert_eq!(answer(&mut pending, true), Some(None), "Clear all");
        assert_eq!(pending, None);
        let mut pending = Some(Some(Category::RustCompiler));
        assert_eq!(
            answer(&mut pending, true),
            Some(Some(Category::RustCompiler))
        );
    }
}
