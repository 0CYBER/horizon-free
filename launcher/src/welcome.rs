//! The first-run welcome: before the data question, the player picks how the launcher looks:
//! theme, accent, each flourish (glow, gradients, the hover shine), motion and the "I'm playing"
//! ping. The rows are Settings' own, so a choice made here is the same one Settings shows later.
use super::*;
use crate::glass_ui::{self as ui, white};
use horizon_theme::glass::{ink, radius};

impl Launcher {
    /// Whether the welcome covers the window: a first run (the data question not answered yet),
    /// not welcomed before, and the opening over.
    pub(crate) fn welcoming(&self) -> bool {
        self.opening.is_none() && !self.settings.welcomed && self.collection.consent.needs_asking()
    }

    pub(crate) fn welcome_screen(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let rows = ui::flat_rows(|| {
            let mut rows = vec![self.launcher_theme_row(cx), self.glass_accent_row(cx)];
            rows.extend(self.effects_rows(cx));
            rows.push(self.motion_row(cx));
            rows.push(index_row(
                0,
                tr("Enable ping"),
                Some(div().child(tr(telemetry::PING)).into_any_element()),
                Some(self.ping_switch("welcome-ping", cx).into_any_element()),
                &t,
            ));
            rows
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
                "Pick a look to start with. Every choice here stays in Settings.",
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
                button("welcome-calm", "Keep it calm", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.effects = horizon_launcher::effects::Effects::CALM;
                        this.save_settings();
                        cx.notify();
                    },
                )),
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
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(ui::scrim().opacity(0.4)),
            )
            .child(card)
    }
}
