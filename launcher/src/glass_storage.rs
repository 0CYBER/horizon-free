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

impl Launcher {
    pub(super) fn refresh_storage(&mut self, clear: Option<Category>, all: bool) {
        if self.storage_pending {
            return;
        }
        self.storage_pending = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = storage::Storage::open()
                .and_then(|storage| storage.report(clear, all))
                .map(Box::new)
                .map_err(native_mods::cache_error);
            let _ = tx.send(Msg::Storage(result));
        });
    }

    /// A finished measurement or Clear from the worker.
    pub(super) fn storage_arrived(&mut self, result: Result<Box<storage::Report>, String>) {
        self.storage_pending = false;
        match result {
            Ok(report) => {
                self.storage_report = Some(*report);
                self.storage_error = None;
            }
            Err(error) => self.storage_error = Some(error),
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
        let total_bytes = self.storage_report.as_ref().map(|report| {
            report
                .rows
                .iter()
                .map(|row| row.remaining_bytes)
                .sum::<u64>()
        });
        let total =
            total_bytes.map_or_else(|| tr("Calculating…").to_string(), storage::format_bytes);
        // Nothing to clear is not a press: Clear dims at nothing, as while a build runs.
        let all_blocked = blocked || total_bytes == Some(0);
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
            self.settings.cache_limit_gib.unwrap_or(25),
            &t,
            {
                let this = cx.entity().downgrade();
                move |value, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.cache_limit_gib = Some(value);
                        if let Err(error) = this.settings.save() {
                            this.storage_error = Some(error);
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
                .child(row.map_or_else(
                    || tr("Calculating…").to_string(),
                    |row| storage::format_bytes(row.remaining_bytes),
                ))
                .child(tr(category_about(category)));
            let row_blocked = blocked || row.is_some_and(|row| row.remaining_bytes == 0);
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
        let rows = self.storage_report.iter().flat_map(|report| &report.rows);
        let bytes: u64 = rows
            .filter(|row| asked.is_none_or(|category| row.category == category))
            .map(|row| row.remaining_bytes)
            .sum();
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
