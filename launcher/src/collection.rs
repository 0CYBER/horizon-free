//! Data collection in the window: the question asked once (and again when what is collected
//! changes), and its section in Settings. Everything goes through `horizon_launcher::telemetry`.
use super::*;
use horizon_launcher::telemetry;
use horizon_telemetry::{Choices, Consent, Kind as DataKind};

/// What the window keeps about data collection.
pub(crate) struct Collection {
    pub consent: Consent,
    body_scroll: Scrollbar,
    json_scroll: Scrollbar,
    /// The welcome's list, and whether its ping row shows what is sent.
    pub welcome_scroll: Scrollbar,
    pub welcome_detail: bool,
    /// The question's switches, until the player answers.
    pub draft: Choices,
    /// "View what would be sent": a line on what it is, and the JSON.
    pub view: Option<(String, String)>,
    /// What "Delete my data" did, or why it could not.
    pub note: Option<String>,
    pub confirm_delete: bool,
    /// The switches (by name) whose "What's sent?" is open, on the question and in Settings.
    sent_open: Vec<&'static str>,
}

impl Collection {
    pub fn load() -> Self {
        Self {
            consent: telemetry::consent(),
            body_scroll: Scrollbar::default(),
            json_scroll: Scrollbar::default(),
            welcome_scroll: Scrollbar::default(),
            welcome_detail: false,
            draft: Choices::NONE,
            view: None,
            note: None,
            confirm_delete: false,
            sent_open: Vec::new(),
        }
    }
}

fn kind_id(prefix: &str, kind: DataKind) -> &'static str {
    match (prefix, kind) {
        ("ask", DataKind::Perf) => "ask-perf",
        ("ask", DataKind::Crash) => "ask-crash",
        ("ask", DataKind::Bug) => "ask-bug",
        ("ask", DataKind::Benchmark) => "ask-benchmark",
        (_, DataKind::Perf) => "data-perf",
        (_, DataKind::Crash) => "data-crash",
        (_, DataKind::Bug) => "data-bug",
        (_, DataKind::Benchmark) => "data-benchmark",
    }
}

/// The exact JSON, in mono, scrolling inside a plate.
fn json_view(view: &(String, String), t: &Theme, scroll: &Scrollbar) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            text(theme::CAPTION)
                .text_color(t.muted)
                .child(view.0.clone()),
        )
        .child(
            scroll.wrap(
                div()
                    .id("data-json")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .p(px(16.))
                    .rounded(px(theme::RADIUS_PLATE))
                    .bg(t.plate)
                    .font_family(theme::MONO)
                    .text_size(px(theme::CAPTION.px))
                    .line_height(px(18.))
                    .text_color(t.ink_soft)
                    .children(view.1.lines().map(|l| div().child(l.to_string()))),
                t,
            ),
        )
}

impl Launcher {
    /// A switch's one plain sentence, with "What's sent?" opening the full list of what it sends.
    fn sent_detail(
        &self,
        name: &'static str,
        summary: &'static str,
        full: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let open = self.collection.sent_open.contains(&name);
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().child(tr(summary)))
            .child(
                div()
                    .id(name)
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child(tr(if open {
                        "Hide what's sent"
                    } else {
                        "What's sent?"
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let list = &mut this.collection.sent_open;
                        match list.iter().position(|n| *n == name) {
                            Some(i) => drop(list.remove(i)),
                            None => list.push(name),
                        }
                        cx.notify();
                    })),
            )
            .children(open.then(|| div().child(tr(full))))
    }

    /// The "I'm playing" ping's switch (`Settings::ping_off`), on by default.
    pub(crate) fn ping_switch(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        switch(id, !self.settings.ping_off, &self.theme).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.ping_off = !this.settings.ping_off;
                this.save_settings();
                cx.notify();
            },
        ))
    }

    /// Whether the question covers the window: not answered for this consent version, and the
    /// opening is over.
    pub(crate) fn asking_consent(&self) -> bool {
        self.opening.is_none() && self.collection.consent.needs_asking()
    }

    fn answer_consent(&mut self, choices: Choices) {
        match telemetry::answer(choices) {
            Ok(consent) => self.collection.consent = consent,
            Err(e) => {
                self.collection.note =
                    Some(trf("Could not save your choice: {error}", &[("error", &e)]))
            }
        }
        // The question and Settings say what the "I'm playing" ping holds, beside the switches.
        if !self.settings.ping_notice_seen {
            self.settings.ping_notice_seen = true;
            self.save_settings();
        }
        self.collection.view = None;
    }

    /// Escape on the data question closes it with every report off. The "I'm playing" ping stays
    /// as it is: only "Turn everything off" turns that off too.
    pub(crate) fn consent_escape(&mut self) -> bool {
        if self.asking_consent() {
            self.answer_consent(Choices::NONE);
            return true;
        }
        false
    }

    fn toggle_view(&mut self) {
        self.collection.view = match self.collection.view {
            Some(_) => None,
            None => Some(telemetry::preview()),
        };
    }

    /// The question, centred over the launcher. The reports start off and the ping on;
    /// "Turn everything off" is one click, as easy as saying yes.
    pub(crate) fn consent_screen(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let draft = self.collection.draft;
        let mut entries: Vec<_> = telemetry::SWITCHES
            .iter()
            .map(|&(kind, name, summary, what)| {
                let on = draft.allows(kind);
                let toggle = switch(kind_id("ask", kind), on, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.collection.draft.set(kind, !on);
                        cx.notify();
                    },
                ));
                (name, summary, what, toggle)
            })
            .collect();
        entries.push((
            telemetry::PING_NAME,
            telemetry::PING_SUMMARY,
            telemetry::PING,
            self.ping_switch("ask-ping", cx),
        ));
        let rows: Vec<Div> = entries
            .into_iter()
            .map(|(name, summary, what, toggle)| {
                div()
                    .flex()
                    .items_start()
                    .gap(px(16.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.rule)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(text(theme::UI).text_color(t.ink).child(tr(name)))
                            .child(
                                self.sent_detail(name, summary, what, cx)
                                    .text_size(px(theme::CAPTION.px))
                                    .text_color(t.muted),
                            ),
                    )
                    .child(toggle)
            })
            .collect();
        let viewing = self.collection.view.is_some();
        // The body scrolls inside the card, so the answers stay at its foot however short the
        // window and however long what would be sent.
        let body = div()
            .id("ask-body")
            .flex()
            .flex_col()
            .min_h_0()
            .gap(px(12.))
            .px(px(28.))
            .pt(px(28.))
            .pb(px(20.))
            .overflow_y_scroll()
            .child(caps(tr("Data collection")).text_color(t.muted))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child(tr("Help make Horizon faster and steadier?")),
            )
            .child(text(theme::UI).text_color(t.muted).child(trf(
                "Horizon can send a few things to its developers. {opening} Settings can change any of it at any time.",
                &[("opening", tr(telemetry::OPENING))],
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(t.rule)
                    .children(rows),
            )
            .child(text(theme::CAPTION).text_color(t.muted).child(tr(telemetry::NEVER)))
            .child(
                div()
                    .id("ask-view")
                    .text_size(px(theme::UI.px))
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child(tr(if viewing { "Hide what would be sent" } else { "View what would be sent" }))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_view();
                        cx.notify();
                    })),
            )
            .children(self.collection.view.as_ref().map(|v| json_view(v, &t, &self.collection.json_scroll)));
        let answers = div()
            .flex()
            .flex_none()
            .justify_end()
            .gap(px(12.))
            .px(px(28.))
            .pb(px(28.))
            .child(
                button("ask-none", "Turn everything off", Kind::Secondary, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.settings.ping_off = true;
                        this.save_settings();
                        this.answer_consent(Choices::NONE);
                        cx.notify();
                    }),
                ),
            )
            .child(
                button("ask-save", "Save choices", Kind::Primary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.answer_consent(this.collection.draft);
                        cx.notify();
                    },
                )),
            );
        let card = plate(&t)
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(680.))
            .max_h_full()
            .bg(t.page)
            .border_1()
            .border_color(t.rule)
            .child(self.collection.body_scroll.wrap(body, &t))
            .child(answers);
        div()
            .id("consent")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(32.))
            .py(px(24.))
            .bg(t.page.opacity(0.92))
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .occlude()
            .child(card)
    }

    /// Settings' data collection: a switch per kind, then the player's data.
    pub(crate) fn collection_section(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let consent = self.collection.consent.clone();
        let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, &(kind, name, summary, what)) in telemetry::SWITCHES.iter().enumerate() {
            let on = !consent.needs_asking() && consent.choices.allows(kind);
            let toggle = switch(kind_id("data", kind), on, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    let mut choices = if this.collection.consent.needs_asking() {
                        Choices::NONE
                    } else {
                        this.collection.consent.choices
                    };
                    choices.set(kind, !on);
                    this.answer_consent(choices);
                    cx.notify();
                },
            ));
            list = list.child(index_row(
                i + 1,
                tr(name),
                Some(self.sent_detail(name, summary, what, cx).into_any_element()),
                Some(toggle.into_any_element()),
                &t,
            ));
        }
        list = list.child(index_row(
            telemetry::SWITCHES.len() + 1,
            tr(telemetry::PING_NAME),
            Some(
                self.sent_detail(
                    telemetry::PING_NAME,
                    telemetry::PING_SUMMARY,
                    telemetry::PING,
                    cx,
                )
                .into_any_element(),
            ),
            Some(self.ping_switch("data-ping", cx).into_any_element()),
            &t,
        ));
        let waiting = horizon_telemetry::outbox::pending(&telemetry::home()).len();
        let id = match consent.install_id.as_str() {
            "" => tr("No install ID: nothing has been collected").to_string(),
            id => trf(
                "Install ID {prefix}…{suffix}",
                &[("prefix", &id[..4]), ("suffix", &id[id.len() - 4..])],
            ),
        };
        let server = if horizon_telemetry::endpoint().is_some() {
            trf(
                "{count} waiting to be sent",
                &[("count", &waiting.to_string())],
            )
        } else {
            tr("Sending is turned off here (HORIZON_TELEMETRY_URL=off)").into()
        };
        let confirm = self.collection.confirm_delete;
        let actions = div()
            .flex()
            .gap(px(8.))
            .child(
                button(
                    "data-view",
                    if self.collection.view.is_some() {
                        "Hide"
                    } else {
                        "View what would be sent"
                    },
                    Kind::Ghost,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_view();
                    cx.notify();
                })),
            )
            .child(
                button(
                    "data-delete",
                    if confirm {
                        "Delete everything?"
                    } else {
                        "Delete my data"
                    },
                    Kind::Secondary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if this.collection.confirm_delete {
                        this.collection.note = Some(match telemetry::delete_my_data() {
                            Ok(done) => done,
                            Err(e) => e,
                        });
                        this.collection.consent = telemetry::consent();
                        this.collection.view = None;
                    }
                    this.collection.confirm_delete = !this.collection.confirm_delete;
                    cx.notify();
                })),
            );
        list = list.child(index_row(
            telemetry::SWITCHES.len() + 2,
            tr("Your data"),
            Some(
                div()
                    .font_family(theme::MONO)
                    .child(format!("{id}  ·  {server}"))
                    .into_any_element(),
            ),
            Some(actions.into_any_element()),
            &t,
        ));
        // In the glass look the section is already titled Data & privacy, and its text lines up
        // with the rows' (14 px inside a 1 px edge).
        let inset = if t.glass { 15. } else { 0. };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .when(!t.glass, |d| {
                d.child(
                    text(theme::HEADING)
                        .text_color(t.ink)
                        .child(tr("Data collection")),
                )
            })
            .child(
                text(theme::UI)
                    .px(px(inset))
                    .max_w(px(600.))
                    .text_color(t.muted)
                    .child(trf(
                        "{opening} What each one sends is below it; the privacy page on playhorizon.dev says how long it is kept.",
                        &[("opening", tr(telemetry::OPENING))],
                    )),
            )
            .child(list)
            .child(
                text(theme::CAPTION)
                    .px(px(inset))
                    .max_w(px(600.))
                    .text_color(t.muted)
                    .child(tr(telemetry::NEVER)),
            )
            .children(
                self.collection
                    .note
                    .clone()
                    .map(|n| text(theme::UI).text_color(t.ink).child(n)),
            )
            .children(
                self.collection
                    .view
                    .as_ref()
                    .map(|v| json_view(v, &t, &self.collection.json_scroll)),
            )
    }
}
