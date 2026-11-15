//! The first-run welcome: before the data question, the player picks how the launcher looks:
//! theme, accent, each flourish (glow, gradients, the hover shine), motion and the "I'm playing"
//! ping (pinned above Continue, with what it sends one click away). The rows are Settings' own,
//! so a choice made here is the same one Settings shows later.
use super::*;
use crate::glass_ui::{self as ui, white};
use horizon_theme::glass::{ink, radius};

impl Launcher {
    /// Whether the welcome covers the window: a first run (the data question not answered yet),
    /// not welcomed before, and the opening over.
    pub(crate) fn welcoming(&self) -> bool {
        self.opening.is_none() && !self.settings.welcomed && self.collection.consent.needs_asking()
    }

    /// Escape answers the welcome as Continue does: the look as it is now stays.
    pub(crate) fn welcome_escape(&mut self) -> bool {
        if self.welcoming() {
            self.settings.welcomed = true;
            self.save_settings();
            return true;
        }
        false
    }

    pub(crate) fn welcome_screen(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let (rows, ping) = ui::flat_rows(|| {
            let mut rows = vec![self.launcher_theme_row(cx), self.glass_accent_row(cx)];
            rows.push(self.motion_row(cx));
            rows.extend(self.effects_rows(cx));
            // Not in the scrolling list: it stays in view above Continue.
            let detail = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(tr(
                    "While you play, the game tells Horizon you're playing, so the launcher can show how many people play right now. It's anonymous, it's on now, and you can turn it off here or in Settings.",
                ))
                .child(
                    div()
                        .id("welcome-ping-detail")
                        .text_color(t.accent)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child(tr(if self.collection.welcome_detail {
                            "Hide what's sent"
                        } else {
                            "What's sent?"
                        }))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.collection.welcome_detail = !this.collection.welcome_detail;
                            cx.notify();
                        })),
                )
                .children(
                    self.collection
                        .welcome_detail
                        .then(|| div().child(tr(telemetry::PING))),
                );
            let ping = index_row(
                0,
                tr("Count me in as \"playing now\""),
                Some(detail.into_any_element()),
                Some(self.ping_switch("welcome-ping", cx).into_any_element()),
                &t,
            );
            (rows, ping)
        });
        let head = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(6.))
            .px(px(28.))
            .pt(px(26.))
            .pb(px(14.))
            .child(
                div()
                    .font_family(horizon_theme::glass::DISPLAY_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(28.))
                    .line_height(px(34.))
                    .text_color(white(ink::FULL))
                    .child(tr("Make Horizon yours")),
            )
            .child(ui::text(14., FontWeight::NORMAL, ink::BODY).child(tr(
                "Pick a look to start with. The \"I'm playing\" ping is already on; you can turn it off below. You can change all of this later in Settings.",
            )));
        let body = div()
            .id("welcome-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(px(16.))
            .overflow_y_scroll()
            .children(rows);
        // A fade at the foot says the list goes on under it.
        let fade = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(36.))
            .bg(gpui::linear_gradient(
                180.,
                gpui::linear_color_stop(ui::scrim().opacity(0.), 0.),
                gpui::linear_color_stop(
                    ui::scrim().opacity(if ui::light() { 0.2 } else { 0.5 }),
                    1.,
                ),
            ));
        let body = self
            .collection
            .welcome_scroll
            .wrap(body, &t)
            .flex_1()
            .min_h_0()
            .child(fade);
        // Pinned between the scrolling list and the buttons, so it is seen before Continue.
        let ping = div()
            .flex()
            .flex_col()
            .flex_none()
            .max_h(px(280.))
            .overflow_hidden()
            .px(px(16.))
            .py(px(8.))
            .border_t_1()
            .border_color(white(0.1))
            .child(ping);
        let foot = div()
            .flex()
            .flex_none()
            .justify_end()
            .gap(px(12.))
            .px(px(28.))
            .py(px(18.))
            .border_t_1()
            .border_color(white(0.1))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .justify_center()
                    .gap(px(2.))
                    .children(self.installing.as_ref().map(|(_, p)| {
                        ui::text(13., FontWeight::NORMAL, ink::BODY).child(trf(
                            "Installing Horizon… {percent}%",
                            &[(
                                "percent",
                                &((p.clamp(0., 1.) * 100.).round() as u32).to_string(),
                            )],
                        ))
                    }))
                    .child(
                        ui::text(12., FontWeight::NORMAL, ink::BODY)
                            .child(tr("Fewer effects turns off glow, gradients and shine.")),
                    ),
            )
            .child(
                button("welcome-calm", "Fewer effects", Kind::Secondary, &t)
                    .tooltip(ui::tip(tr("Turns off glow, gradients and shine")))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.settings.effects = horizon_launcher::effects::Effects::CALM;
                        this.save_settings();
                        cx.notify();
                    })),
            )
            .child(
                button("welcome-done", "Continue", Kind::Primary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.welcomed = true;
                        this.save_settings();
                        cx.notify();
                    },
                )),
            );
        let card = ui::dialog_plate(radius::MODAL)
            .id("welcome-card")
            .occlude()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(820.))
            .h_full()
            .max_h(px(700.))
            .child(head)
            .child(body)
            .child(ping)
            .child(foot);
        div()
            .id("welcome")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            // The same frosted dimming as the Settings dialog's.
            .when(!self.settings.performance(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(0.62)
                        .child(ui::scrim_backdrop()),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(ui::scrim().opacity(if ui::light() { 0.2 } else { 0.4 })),
            )
            .child(card)
    }
}
