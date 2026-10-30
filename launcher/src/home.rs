//! Home in the night look (docs/design-system.md, "Night"): the selected instance's name over the
//! sky with what it plays and whether it is ready, Play and the actions beside it, and a card with
//! what's new and joining a friend.
use super::*;
use crate::plate::alpha;
use crate::shell::{action, c, label, Action};
use theme::night;

impl Launcher {
    /// The focal point: what will start and its state, the instance's name, when it was played,
    /// and an update when one waits. No `tag`: a Java instance's Minecraft (`game` then names its
    /// loader too).
    pub(super) fn hero(
        &self,
        tag: &str,
        game: &str,
        prepared: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let n = t.night;
        let instance = self.selected_instance();
        let name = instance.map_or(
            if tag.is_empty() {
                "Minecraft"
            } else {
                "Horizon"
            },
            |i| i.name.as_str(),
        );
        let mut what = vec![format!("Minecraft: Java Edition {game}")];
        if !tag.is_empty() {
            what.push(format!(
                "Horizon {}",
                horizon_launcher::nightly::label(tag)
                    .unwrap_or_else(|| tag.trim_start_matches('v').to_string())
            ));
        }
        what.push(tr(if prepared { "Ready" } else { "Needs preparing" }).to_string());
        if let Some(playing) = self.playing {
            what.push(format!("{} playing now", thousands(playing)));
        }
        let state = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_none()
                    .size(px(6.))
                    .bg(if prepared { t.success } else { t.warn }),
            )
            .child(
                shell::text(night::CAPTION, c(n.ink_soft))
                    .font_weight(FontWeight::MEDIUM)
                    .child(what.join(" · ")),
            );
        // Never squeezed by the rows below: the ping notice can make it tall.
        let mut hero = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(10.))
            .child(state)
            .child(
                shell::text(night::DISPLAY, c(n.ink))
                    .truncate()
                    .child(name.to_string()),
            )
            .children(instance.map(|i| {
                shell::text(night::UI, c(n.ink_soft)).child(instances_page::played_line(i))
            }));
        // A Java instance (no Horizon tag) isn't offered Horizon's updates.
        if let Some(release) = self.updates.horizon.as_ref().filter(|_| !tag.is_empty()) {
            let installing = self.installing.as_ref().filter(|(t, _)| *t == release.tag);
            let line = shell::text(night::CAPTION, c(n.muted))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().size(px(6.)).bg(c(n.accent_hi)))
                .child(format!("{} available", update_name(release)));
            hero = hero.child(match installing {
                Some((_, p)) => line
                    .child(
                        div()
                            .w(px(120.))
                            .flex()
                            .child(progress_bar(Some(self.bar), &t)),
                    )
                    .child(format!("{:.0}%", p * 100.)),
                None => line.child(
                    div()
                        .id("install-update")
                        .text_color(c(n.accent_hi))
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child("Install")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.install_update();
                            cx.notify();
                        })),
                ),
            });
        }
        // Players who answered the first-run question before the ping existed read about it
        // once here.
        if !self.settings.ping_notice_seen && !self.collection.consent.needs_asking() {
            hero = hero.child(
                crate::plate::plate(&n)
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(16.))
                    .w_full()
                    .max_w(px(560.))
                    .child(
                        shell::text(night::UI, c(n.ink))
                            .relative()
                            .child(telemetry::PING_NAME),
                    )
                    .child(
                        shell::text(night::CAPTION, c(n.muted))
                            .relative()
                            .child(telemetry::PING),
                    )
                    .child(
                        shell::text(night::UI, c(n.accent_hi))
                            .relative()
                            .id("ping-notice-ok")
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .child("Got it")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.settings.ping_notice_seen = true;
                                this.settings.save();
                                cx.notify();
                            })),
                    ),
            );
        }
        hero
    }

    /// Play, then what else the instance offers: its version (Horizon), its mods, its folder.
    /// `ready`: prepared, or a Java instance (its game comes with Play).
    pub(super) fn home_actions(&self, ready: bool, java: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let running = self.client.is_some() || self.starting_client.is_some();
        let mut play = action("play", Action::Main, &t)
            .min_w(px(180.))
            .child(
                div()
                    .relative()
                    .child(blocks::icon("play", 20., c(t.night.on_accent))),
            )
            .child(label(
                tr(if running { "Running" } else { "Play" }).to_string(),
            ));
        match play_press(ready, self.owner, self.preparing || running) {
            PlayPress::Nothing => play = disabled(play),
            PlayPress::Play => {
                play = play.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = false;
                    this.play();
                    cx.notify();
                }))
            }
            PlayPress::PrepareThenPlay => {
                play = play.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = false;
                    this.play_when_prepared = this.selected().cloned();
                    this.prepare(backend::MinecraftFrom::Install);
                    cx.notify();
                }))
            }
        }
        let folder = self.selected_instance().map(|i| i.home.clone());
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(play)
            // Horizon's versions don't apply to a Java instance.
            .children((!java).then(|| self.picker(cx)))
            .child(
                action("home-mods", Action::Quiet, &t)
                    .child(label(tr("Mods").to_string()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Mods);
                        cx.notify();
                    })),
            )
            .children(folder.map(|home| {
                action("home-folder", Action::Quiet, &t)
                    .child(label(tr("Folder").to_string()))
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&home);
                        cx.open_with_system(&home);
                    })
            }))
    }

    /// The card beside the hero, for a Horizon version: a line from a release's notes, and
    /// joining a friend with their invite code.
    pub(super) fn home_card(&self, tag: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let n = t.night;
        let summary = match &self.releases {
            Load::Ready(list) => list
                .iter()
                .find(|r| r.tag == tag)
                // A Nightly's notes name its build, nothing a player reads for.
                .filter(|r| r.nightly.is_none())
                .and_then(|r| backend::notes_summary(&r.body)),
            _ => None,
        };
        let heading = |s: &str| {
            shell::text(night::CAPTION, c(n.muted))
                .relative()
                .font_weight(FontWeight::SEMIBOLD)
                .child(tr(s).to_string())
        };
        let mut card = crate::plate::plate(&n)
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(8.))
            .w(px(320.))
            .p(px(20.));
        if let Some(line) = summary {
            card = card
                .child(heading("What's new"))
                .child(
                    shell::text(night::UI, c(n.ink))
                        .relative()
                        .font_weight(FontWeight::BOLD)
                        .child(
                            horizon_launcher::nightly::label(tag)
                                .unwrap_or_else(|| tag.trim_start_matches('v').to_string()),
                        ),
                )
                .child(
                    shell::text(night::CAPTION, c(n.ink_soft))
                        .relative()
                        .line_clamp(3)
                        .child(line),
                )
                .child(
                    div()
                        .relative()
                        .my(px(8.))
                        .h(px(1.))
                        .bg(alpha(n.stroke, n.rule_alpha)),
                );
        }
        card.child(heading("Join a friend"))
            .child(self.friend_box(self.can_start(tag), cx))
    }

    /// The invite code field and Join.
    fn friend_box(&self, can_play: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let n = t.night;
        let focused = self.friend_focused;
        let field = div()
            .id("friend-code")
            .track_focus(&self.friend_focus)
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h(px(theme::CONTROL))
            .px(px(12.))
            .overflow_hidden()
            .rounded(px(night::RADIUS_CONTROL))
            .border_1()
            .border_color(if focused {
                c(n.accent_hi)
            } else {
                alpha(n.stroke, 2. * n.stroke_alpha).into()
            })
            .bg(alpha(n.page, 0.6))
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.friend_code,
                &t,
                focused,
                false,
                false,
                "Invite code",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.friend_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.friend_code, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.join_friend(),
                    line_edit::Key::Escape => window.blur(),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.friend_code);
        let valid = horizon_friends::invite::parse(self.friend_code.text()).is_some();
        let mut join =
            action("join-friend", Action::Raised, &t).child(label(tr("Join").to_string()));
        if can_play && valid {
            join = join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_friend();
                cx.notify();
            }));
        } else {
            join = disabled(join);
        }
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(field)
            .child(join)
    }
}
