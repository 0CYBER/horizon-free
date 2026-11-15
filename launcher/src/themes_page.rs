//! Settings' accent colour and Home background rows (`horizon_launcher::themes`).
use super::*;
use crate::glass_ui as ui;
use crate::servers_page::Field;
use crate::theme::Night;
use gpui::{
    canvas, fill, point, size, Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point,
};
use horizon_launcher::themes::{self, Accent};
use std::path::PathBuf;

/// Settings' two sliders of the glass look (the mockup's `Slider`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LookKnob {
    /// How much white the plates take: 0 to 16 %.
    Glass,
    /// How much they blur the scene: 0 to 48 px.
    Blur,
}

impl LookKnob {
    fn max(self) -> f32 {
        match self {
            LookKnob::Glass => 16.,
            LookKnob::Blur => 48.,
        }
    }

    /// Page Up / Page Down's step: an eighth of the range.
    fn page(self) -> f32 {
        self.max() / 8.
    }
}

/// Where each slider's track is, and the one being dragged.
#[derive(Default)]
pub(super) struct LookSliders {
    areas: [std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>; 2],
    dragging: Option<LookKnob>,
    /// How far from the knob's centre the pointer grabbed it, so the knob does not jump.
    grab: f32,
}

/// A slider's track (`w-52`), in points.
const TRACK: f32 = 208.;
/// The knob's diameter, in points; its centre runs from `THUMB / 2` to `TRACK - THUMB / 2`.
const THUMB: f32 = 18.;

/// The value fraction (0 to 1) for a point `across` points into a track `width` wide: the inverse
/// of [`thumb_centre`], so a press on the knob's drawn centre keeps its value.
fn fraction_at(across: f32, width: f32) -> f32 {
    let run = width - THUMB;
    if run <= 0. {
        return 0.;
    }
    ((across - THUMB / 2.) / run).clamp(0., 1.)
}

/// Where the knob's centre is drawn for a value fraction, in points from the track's left.
fn thumb_centre(f: f32, width: f32) -> f32 {
    THUMB / 2. + f.clamp(0., 1.) * (width - THUMB).max(0.)
}

/// Why Glass and Blur change nothing in `theme`'s look, if they don't: performance mode's plates
/// are a solid colour. The sunset's, a painted sky's and Hypr Neon's night's are frosted glass.
fn look_unused(theme: &crate::theme::Theme, performance: bool) -> Option<&'static str> {
    if performance {
        Some("Performance mode draws solid panels, without glass or blur.")
    } else if !theme.look.own_sky && (!theme.scene || !theme.dark) {
        Some("This theme's night draws solid panels, without glass or blur.")
    } else {
        None
    }
}

/// Each accent's swatch, in points.
const SWATCH: f32 = 26.;
/// The hue strip beside the swatches, in points: 0 to 360 degrees across it.
const STRIP: f32 = 120.;

impl Launcher {
    /// The theme Settings ask for, under the window's own light or dark: the player's look,
    /// under the launcher theme's sky if they picked one.
    pub(super) fn settings_theme(&self, window: &Window) -> Theme {
        crate::theme::from_settings(&self.settings, window.appearance())
    }

    /// Picks a launcher theme: the accent picked under the theme left is kept for it, and the
    /// new one brings back its own (`themes::starting_accent`). The page and its scroll stay.
    fn pick_theme(&mut self, id: Option<&'static str>) {
        let left = themes::accent_key(self.settings.theme.as_deref()).to_string();
        self.settings
            .theme_accents
            .insert(left, self.settings.accent);
        self.settings.theme = id.map(str::to_owned);
        self.settings.accent = themes::starting_accent(id, &self.settings.theme_accents);
        self.save_settings();
        self.restyle = true;
    }

    /// Settings' launcher theme: a card for each, drawn in its own colours (a text sample, a
    /// field and its main button) under the appearance on screen. Free for everyone.
    pub(super) fn launcher_theme_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let current = themes::theme(self.settings.theme.as_deref()).map(|th| th.id);
        let mut options: Vec<(Option<&'static str>, &'static str, &'static str, Night)> = vec![(
            None,
            "Horizon",
            themes::HORIZON_DESCRIPTION,
            theme::night::NIGHT,
        )];
        options.extend(themes::THEMES.iter().map(|th| {
            let night = th.palette.copied().unwrap_or(Night {
                sky: th.sky,
                ..theme::night::NIGHT
            });
            (Some(th.id), th.name, th.description, night)
        }));
        let mut cards = div().flex().flex_wrap().gap(px(10.));
        for (id, name, about, p) in options {
            let accent = themes::starting_accent(id, &self.settings.theme_accents).rgb();
            let label = horizon_theme::glass::Glass::new(accent, 0., 0.).on_accent;
            let chosen = current == id;
            let preview = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(10.))
                .h(px(92.))
                .rounded(px(10.))
                .bg(gpui::rgb(p.page))
                .border_1()
                .border_color(gpui::rgb(theme::night::over(p.stroke, 0.3, p.page)))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(gpui::rgb(p.ink))
                        .child(tr("Aa Text")),
                )
                .child(
                    div()
                        .h(px(14.))
                        .w_full()
                        .rounded(px(4.))
                        .bg(gpui::rgb(theme::night::over(p.card, p.card_alpha, p.page)))
                        .border_1()
                        .border_color(gpui::rgb(theme::night::over(p.stroke, 0.4, p.page))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(22.))
                        .w(px(64.))
                        .rounded(px(6.))
                        .bg(gpui::rgb(accent))
                        .text_size(px(12.))
                        .text_color(gpui::rgb(label))
                        .child(tr("Play")),
                );
            cards = cards.child(
                keyboard(
                    div().id(SharedString::from(format!(
                        "theme-{}",
                        id.unwrap_or(themes::HORIZON)
                    ))),
                    &t,
                )
                .flex()
                .flex_col()
                .gap(px(6.))
                .w(px(136.))
                .p(px(4.))
                .rounded(px(14.))
                .cursor_pointer()
                .border_2()
                .border_color(if chosen {
                    t.accent
                } else {
                    gpui::transparent_black()
                })
                .when(!chosen, |d| d.hover(|s| s.bg(ui::white(0.06))))
                .child(preview)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(4.))
                        .pb(px(2.))
                        .text_size(px(13.))
                        .text_color(t.ink)
                        .child(tr(name))
                        .when(chosen, |d| d.child(ui::icon("tick-02", 14., t.accent))),
                )
                .child(
                    div()
                        .px(px(4.))
                        .pb(px(2.))
                        .text_size(px(11.))
                        .line_height(px(14.))
                        .text_color(t.muted)
                        .child(tr(about)),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if themes::theme(this.settings.theme.as_deref()).map(|th| th.id) != id {
                        this.pick_theme(id);
                        cx.notify();
                    }
                })),
            );
        }
        let note = Some(
            div()
                .child(tr(
                    "Free for everyone. Each theme remembers the accent you pick under it.",
                ))
                .into_any_element(),
        );
        ui::setting_row_stacked(tr("Theme"), note, cards.into_any_element())
    }

    /// Reduced motion: follow the system's, hold everything still, or keep it moving.
    pub(super) fn motion_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let this = cx.entity().downgrade();
        let picker = segments(
            "reduce-motion",
            &[
                (None, "System"),
                (Some(true), "Reduced"),
                (Some(false), "Full"),
            ],
            self.settings.reduce_motion,
            &t,
            move |choice, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.settings.reduce_motion = choice;
                    this.save_settings();
                    cx.notify();
                });
            },
        );
        index_row(
            0,
            tr("Motion"),
            Some(
                div()
                    .child(tr(
                        "Reduced stops the sky, the fades and the slides; buttons still answer at once. System follows your computer's setting.",
                    ))
                    .into_any_element(),
            ),
            Some(picker.into_any_element()),
            &t,
        )
    }

    /// A switch per flourish (`effects::Effects`): glow, gradients, the hover shine, each on or
    /// off alone. Settings' Appearance and the first-run welcome show them.
    pub(super) fn effects_rows(&mut self, cx: &mut Context<Self>) -> Vec<Div> {
        let t = self.theme;
        horizon_launcher::effects::Effects::LIST
            .into_iter()
            .map(|(flag, name, what)| {
                let on = self.settings.effects.get(flag);
                let toggle = crate::switch(format!("effect-{name}"), on, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.settings.effects.set(flag, !on);
                        this.save_settings();
                        cx.notify();
                    },
                ));
                index_row(
                    0,
                    tr(name),
                    Some(div().child(tr(what)).into_any_element()),
                    Some(toggle.into_any_element()),
                    &t,
                )
            })
            .collect()
    }

    /// The picture behind Home, if the player keeps one.
    pub(super) fn home_background(&self) -> Option<PathBuf> {
        themes::background(&backend::data_dir(), self.settings.background.as_deref())
    }

    pub(super) fn accent_row(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut swatches = div().flex().gap(px(6.));
        for accent in Accent::ALL {
            let face = Theme::for_appearance(
                backend::Appearance::Dark,
                accent,
                gpui::WindowAppearance::Dark,
            )
            .face;
            let chosen = self.settings.accent == accent;
            swatches = swatches.child(
                keyboard(
                    div().id(SharedString::from(format!("accent-{}", accent.name()))),
                    &t,
                )
                .size(px(SWATCH))
                .bg(face)
                .border_2()
                .border_color(if chosen { t.ink } else { t.outline })
                .cursor_pointer()
                .when(chosen, |d| {
                    d.flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(8.)).bg(t.on_face))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.settings.accent = accent;
                    this.save_settings();
                    this.theme = this.settings_theme(window);
                    cx.notify();
                })),
            );
        }
        index_row(
            0,
            tr("Accent colour"),
            Some(
                div()
                    .child(tr(
                        "Horizon's violet by default. Every colour keeps text as easy to read.",
                    ))
                    .into_any_element(),
            ),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(swatches)
                    .child(self.hue_strip(&t, cx))
                    .into_any_element(),
            ),
            &t,
        )
    }

    /// The glass look's accent row (the mockup's): its seven swatches, the any-colour swatch
    /// (a click shows the hue strip under them, the browser's colour picker in the mockup) and
    /// the hex field, which takes the colour as soon as it is one.
    pub(super) fn glass_accent_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.sync_accent_hex(cx);
        let current = self.settings.accent;
        let custom = !Accent::ALL.contains(&current);
        let swatch = |id: SharedString, on: bool| {
            keyboard(
                div()
                    .id(id)
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(36.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(ui::white(0.18)))
                    .when(!on, |d| d.hover(|s| s.bg(ui::white(0.08)))),
                &t,
            )
        };
        let chip = |rgb: u32| div().size(px(24.)).rounded(px(7.)).bg(gpui::rgb(rgb));
        let mut row = div().flex().flex_wrap().items_center().gap(px(4.));
        for (accent, rgb) in Accent::ALL.into_iter().zip(themes::SWATCHES) {
            row = row.child(
                swatch(
                    SharedString::from(format!("accent-{}", accent.name())),
                    current == accent,
                )
                .child(chip(rgb))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.settings.accent = accent;
                    this.save_settings();
                    this.restyle = true;
                    cx.notify();
                })),
            );
        }
        row = row
            .child(
                swatch("accent-any".into(), custom)
                    .child(if custom {
                        chip(current.rgb()).into_any_element()
                    } else {
                        ui::icon("color-picker", 18., ui::white(0.7)).into_any_element()
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.accent_strip = !this.accent_strip;
                        cx.notify();
                    })),
            )
            .child(self.field(Field::AccentHex, "#6b4bff", 96., cx).ml(px(4.)));
        let wide = self.window_width >= crate::glass_home::WIDE;
        let control = div()
            .flex()
            .flex_col()
            .when(wide, |d| d.items_end())
            .when(!wide, |d| d.items_start())
            .gap(px(10.))
            .child(row)
            .when(self.accent_strip, |d| {
                d.child(
                    self.hue_strip(&t, cx)
                        .w(px(212.))
                        .h(px(24.))
                        .rounded(px(6.))
                        .overflow_hidden(),
                )
            });
        let note = Some(
            div()
                .child(tr(
                    "Colours the buttons, the switches and the sky. Pick one, or type any hex colour.",
                ))
                .into_any_element(),
        );
        if wide {
            ui::setting_row(tr("Accent colour"), note, Some(control.into_any_element()))
        } else {
            ui::setting_row_stacked(tr("Accent colour"), note, control.into_any_element())
        }
    }

    /// The Glass or Blur row: the mockup's slider (a 6 px track filled in the soft accent, an
    /// 18 px white knob, 36 px of height to hit) and its value. Dragging changes the look at once
    /// and saves on release; Tab focuses it, the arrows step by one, Page Up/Down by an eighth,
    /// Home/End go to the ends, each key saving at once.
    pub(super) fn look_slider_row(&mut self, knob: LookKnob, cx: &mut Context<Self>) -> Div {
        let (title, about, value, shown) = match knob {
            LookKnob::Glass => {
                let v = (self.settings.glass_tint() * 100.).round();
                (
                    "Glass",
                    "How light the frosted panels and menus are.",
                    v,
                    trf("{value}%", &[("value", &v.to_string())]),
                )
            }
            LookKnob::Blur => {
                let v = self.settings.glass_blur().round();
                (
                    "Blur",
                    "How frosted the panels are.",
                    v,
                    trf("{value} px", &[("value", &v.to_string())]),
                )
            }
        };
        let f = (value / knob.max()).clamp(0., 1.);
        let area = self.look_sliders.areas[knob as usize].clone();
        let t = self.theme;
        // A slider the look doesn't use is shown dimmed, says why, and takes no input.
        let unused = self.look_unused();
        let about = unused.unwrap_or(about);
        let base = div().id(SharedString::from(format!("look-slider-{knob:?}")));
        let base = if unused.is_none() {
            keyboard(base, &t).cursor_pointer()
        } else {
            base.cursor_default()
        };
        let track = base
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .w(px(TRACK))
            .h(px(36.))
            .rounded(px(8.))
            .child(
                canvas(move |bounds, _, _| area.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .overflow_hidden()
                    .bg(ui::white(0.12))
                    .child(
                        div()
                            .h_full()
                            .w(relative(f))
                            .rounded_full()
                            .bg(ui::color(ui::look().accent_soft)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(thumb_centre(f, TRACK) - THUMB / 2.))
                    .size(px(THUMB))
                    .rounded_full()
                    .bg(ui::paper()),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if this.look_unused().is_some() {
                        return;
                    }
                    this.grab_look(knob, ev.position);
                    this.look_sliders.dragging = Some(knob);
                    this.pick_look(knob, ev.position, cx);
                }),
            )
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                if this.look_unused().is_some() {
                    return;
                }
                let now = this.look_value(knob);
                let to = match ev.keystroke.key.as_str() {
                    "left" | "down" => now - 1.,
                    "right" | "up" => now + 1.,
                    "pagedown" => now - knob.page(),
                    "pageup" => now + knob.page(),
                    "home" => 0.,
                    "end" => knob.max(),
                    _ => return,
                };
                cx.stop_propagation();
                if this.set_look(knob, to) {
                    this.save_settings();
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left)
                    && this.look_sliders.dragging == Some(knob)
                {
                    this.pick_look(knob, ev.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_look(cx)),
            )
            // A drag released off the track saves too.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_look(cx)),
            );
        let control = div()
            .flex()
            .items_center()
            .gap(px(16.))
            // Dimmed whole, value too, so it reads as off rather than as a slider at rest.
            .when(unused.is_some(), |d| d.opacity(0.4))
            .child(track)
            .child(
                ui::text(13., FontWeight::NORMAL, 0.6)
                    .w(px(48.))
                    .text_right()
                    .child(shown),
            );
        ui::setting_row(
            tr(title),
            Some(div().child(tr(about)).into_any_element()),
            Some(control.into_any_element()),
        )
    }

    /// The Performance mode row: the cheaper scene for older computers (a quarter-resolution
    /// sky, a box blur, resizes baked in the background, no frame per mouse move).
    pub(super) fn performance_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let on = self.settings.performance();
        ui::setting_row(
            tr("Performance mode"),
            Some(
                div()
                    .child(tr(
                        "Smoother on older computers: a lower-resolution sky, a cheaper blur, window resizes baked in the background and no frame per mouse move.",
                    ))
                    .into_any_element(),
            ),
            Some(
                crate::switch("performance", on, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.settings.performance = Some(!this.settings.performance());
                        // A painted theme's drifting sky follows it.
                        this.restyle = true;
                        this.save_settings();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        )
    }

    /// The value under the pointer, in whole steps, as the look; saved when the drag ends.
    fn pick_look(&mut self, knob: LookKnob, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(area) = self.look_sliders.areas[knob as usize].get() else {
            return;
        };
        let width: f32 = area.size.width.into();
        if width <= 0. {
            return;
        }
        let across: f32 = (at.x - area.origin.x).into();
        let f = fraction_at(across - self.look_sliders.grab, width);
        if self.set_look(knob, f * knob.max()) {
            cx.notify();
        }
    }

    /// Why Glass and Blur change nothing in the look on screen, if they don't.
    fn look_unused(&self) -> Option<&'static str> {
        look_unused(&self.theme, self.settings.performance())
    }

    /// The knob's value as the slider shows it (Glass in percent, Blur in px).
    fn look_value(&self, knob: LookKnob) -> f32 {
        match knob {
            LookKnob::Glass => (self.settings.glass_tint() * 100.).round(),
            LookKnob::Blur => self.settings.glass_blur().round(),
        }
    }

    /// Sets the knob to `value`, clamped to its range in whole steps; true when it changed.
    fn set_look(&mut self, knob: LookKnob, value: f32) -> bool {
        let value = Some(value.clamp(0., knob.max()).round());
        let slot = match knob {
            LookKnob::Glass => &mut self.settings.glass,
            LookKnob::Blur => &mut self.settings.blur,
        };
        if *slot == value {
            return false;
        }
        *slot = value;
        self.restyle = true;
        true
    }

    /// A press on the knob keeps its offset from the centre, so the knob does not jump; a press
    /// elsewhere on the track puts the knob's centre under the pointer.
    fn grab_look(&mut self, knob: LookKnob, at: Point<Pixels>) {
        self.look_sliders.grab = 0.;
        let Some(area) = self.look_sliders.areas[knob as usize].get() else {
            return;
        };
        let width: f32 = area.size.width.into();
        let across: f32 = (at.x - area.origin.x).into();
        let centre = thumb_centre(self.look_value(knob) / knob.max(), width);
        if (across - centre).abs() <= THUMB / 2. {
            self.look_sliders.grab = across - centre;
        }
    }

    fn finish_look(&mut self, cx: &mut Context<Self>) {
        if self.look_sliders.dragging.take().is_some() {
            self.save_settings();
            cx.notify();
        }
    }

    /// Reset the look: the theme, accent, glass, blur and Home's background go back
    /// to how Horizon ships. Language, while playing and the opening are not the look and stay.
    pub(super) fn reset_look_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        ui::setting_row(
            tr("Reset the launcher's look"),
            Some(
                div()
                    .child(tr(
                        "Theme, accent colour, glass, blur and Home's background go back to how Horizon ships.",
                    ))
                    .into_any_element(),
            ),
            Some(
                ui::quiet_icon(
                    "look-reset",
                    Some("refresh"),
                    tr("Reset to defaults"),
                    40.,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    reset_look(&mut this.settings);
                    let saved = this.save_settings();
                    titlebar::apply_appearance(window, this.settings.appearance);
                    this.accent_strip = false;
                    this.background_error = None;
                    this.theme = this.settings_theme(window);
                    // A failed write keeps the save notice and its retry; the toast must not
                    // claim the old look is gone for good.
                    crate::glass_toast::say(if saved {
                        tr("The launcher's look is back to how Horizon ships")
                    } else {
                        tr("The launcher's look is reset for now, but it could not be saved")
                    });
                    cx.notify();
                }))
                .into_any_element(),
            ),
        )
    }

    /// The hex field shows the accent, except while typed in: then each whole colour typed is
    /// the accent at once, as in the mockup.
    fn sync_accent_hex(&mut self, cx: &mut Context<Self>) {
        let shown = self.settings.accent.hex();
        if !self.servers.has_focus(Field::AccentHex) {
            if self.servers.value(Field::AccentHex) != shown {
                self.servers.text(Field::AccentHex).set(shown);
            }
            return;
        }
        let typed = self.servers.value(Field::AccentHex).trim();
        let hex = format!("#{}", typed.trim_start_matches('#'));
        if let Some(accent) = Accent::parse(&hex).filter(|a| *a != self.settings.accent) {
            self.settings.accent = accent;
            self.save_settings();
            self.restyle = true;
            cx.notify();
        }
    }

    /// A strip of every hue: a click or a drag picks any of them, the swatches being shortcuts.
    fn hue_strip(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let current = self.settings.accent.hue();
        let area = self.hue_area.clone();
        let ink = t.ink;
        let outline = t.outline;
        // Hues per painted quad: 60 of them, fine enough at this width.
        const STEPS: usize = 60;
        div()
            .relative()
            .flex_none()
            .w(px(STRIP))
            .h(px(SWATCH))
            .border_2()
            .border_color(outline)
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, _| {
                        area.set(Some(bounds));
                        bounds
                    },
                    move |bounds, _, window, _| {
                        let width: f32 = bounds.size.width.into();
                        let step = width / STEPS as f32;
                        for i in 0..STEPS {
                            let left = px(step * i as f32);
                            let colour: Hsla =
                                gpui::rgb(themes::hue_colour(i as f32 * 360. / STEPS as f32))
                                    .into();
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left() + left, bounds.top()),
                                    size(px(step), bounds.size.height),
                                ),
                                colour,
                            ));
                        }
                        if let Some(hue) = current {
                            let at = px(width * hue / 360.);
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left() + at - px(1.5), bounds.top()),
                                    size(px(3.), bounds.size.height),
                                ),
                                ink,
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.pick_hue(ev.position, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                if ev.pressed_button == Some(MouseButton::Left) {
                    this.pick_hue(ev.position, window, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.finish_hue();
                    cx.notify();
                }),
            )
            // `on_mouse_up` fires only while the strip is hovered, so a drag released off it would
            // keep its accent in memory and never write it.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.finish_hue();
                    cx.notify();
                }),
            )
    }

    /// A release ends the drag and writes `settings.json` once.
    fn finish_hue(&mut self) {
        if self.hue_dragging {
            self.hue_dragging = false;
            self.save_settings();
        }
    }

    /// The hue under the pointer, as the accent. Saving waits until the mouse comes up, so a drag
    /// writes `settings.json` once. A move that stays on the same degree changes nothing.
    fn pick_hue(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(area) = self.hue_area.get() else {
            return;
        };
        let width: f32 = area.size.width.into();
        if width <= 0. {
            return;
        }
        let across: f32 = (at.x - area.origin.x).into();
        let accent = Accent::custom(((across / width).clamp(0., 1.) * 360.).round());
        self.hue_dragging = true;
        if accent == self.settings.accent {
            return;
        }
        self.settings.accent = accent;
        self.theme = self.settings_theme(window);
        cx.notify();
    }

    fn choose_background(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Choose").to_string().into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.take_background(&path);
                cx.notify();
            });
        })
        .detach();
    }

    fn take_background(&mut self, path: &std::path::Path) {
        match themes::keep_background(&backend::data_dir(), path) {
            Ok(name) => {
                self.settings.background = Some(name);
                self.save_settings();
                self.background_error = None;
            }
            Err(e) => self.background_error = Some(e),
        }
    }

    pub(super) fn home_background_row(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let custom = self.home_background().is_some();
        let mut actions = div().flex().gap(px(4.)).child(
            button(
                "background-choose",
                "Choose a picture…",
                Kind::Secondary,
                &t,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.choose_background(cx);
                cx.notify();
            })),
        );
        if custom {
            actions = actions.child(
                button("background-reset", "Landscape", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.background = None;
                        this.save_settings();
                        cx.notify();
                    },
                )),
            );
        }
        let said = if custom {
            tr("Your picture, behind Home, cropped to fill it.")
        } else {
            tr("Horizon's landscape. Pick a PNG, JPEG or WebP picture of your own.")
        };
        let detail = div().flex().flex_col().child(said.to_string()).children(
            self.background_error
                .clone()
                .map(|e| div().text_color(t.danger).child(e)),
        );
        index_row(
            0,
            tr("Home background"),
            Some(detail.into_any_element()),
            Some(actions.into_any_element()),
            &t,
        )
    }
}

/// The fields Reset the launcher's look puts back: the look only.
fn reset_look(s: &mut Settings) {
    s.appearance = Default::default();
    s.accent = Accent::VIOLET;
    s.theme = None;
    s.glass = None;
    s.blur = None;
    s.background = None;
    s.theme_accents.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The knob's drawn centre and the pointer's value agree end to end (they used to map
    /// `x / TRACK`, so the minimum knob centre read 2 px of a 48 px blur).
    #[test]
    fn slider_pointer_matches_the_knob() {
        assert_eq!(fraction_at(THUMB / 2., TRACK), 0.);
        assert_eq!(fraction_at(TRACK - THUMB / 2., TRACK), 1.);
        assert_eq!(fraction_at(TRACK / 2., TRACK), 0.5);
        assert_eq!(fraction_at(-50., TRACK), 0.);
        assert_eq!(fraction_at(TRACK + 50., TRACK), 1.);
        for i in 0..=48 {
            let f = i as f32 / 48.;
            let back = fraction_at(thumb_centre(f, TRACK), TRACK);
            assert!((back - f).abs() < 1e-5, "{f} -> {back}");
        }
        assert_eq!(fraction_at(5., THUMB), 0.);
    }

    /// The sliders are live wherever the plates are glass (the sunset's, a painted sky's and
    /// Hypr Neon's night), and off in performance mode.
    #[test]
    fn sliders_are_live_only_over_glass() {
        let unused = |theme: &str, performance: bool| {
            let settings = Settings {
                theme: Some(theme.into()),
                performance: Some(performance),
                ..Default::default()
            };
            let t = crate::theme::from_settings(&settings, gpui::WindowAppearance::Dark);
            look_unused(&t, performance).is_some()
        };
        for painted in [
            "deep_ocean",
            "autumn_forest",
            "cherry_blossom",
            "midnight_aurora",
            "overworld",
            "taiga",
            "first_light",
        ] {
            assert!(!unused(painted, false), "{painted}");
            assert!(unused(painted, true), "{painted} performance");
        }
        assert!(!unused("sunrise", false));
        assert!(!unused("hypr_neon", false));
        assert!(unused("hypr_neon", true));
    }

    #[test]
    fn reset_look_keeps_what_is_not_the_look() {
        let mut s = Settings {
            accent: Accent::custom(120.),
            theme: Some("nord".into()),
            glass: Some(3.),
            blur: Some(9.),
            background: Some("mine.png".into()),
            theme_accents: [("sunrise".to_string(), Accent::TEAL)].into(),
            reduce_motion: Some(true),
            language: Some(i18n::Language::French),
            skip_opening: true,
            game_args: "-Xmx4G".into(),
            while_playing: backend::WhilePlaying::Hide,
            ..Default::default()
        };
        reset_look(&mut s);
        assert_eq!(s.accent, Accent::VIOLET);
        assert!(s.theme_accents.is_empty());
        assert_eq!(
            (s.theme, s.glass, s.blur, s.background),
            (None, None, None, None)
        );
        assert_eq!(s.language, Some(i18n::Language::French));
        assert!(s.skip_opening);
        assert_eq!(s.game_args, "-Xmx4G");
        assert_eq!(s.while_playing, backend::WhilePlaying::Hide);
        // Motion is accessibility, not the look: a reset never turns it back on.
        assert_eq!(s.reduce_motion, Some(true));
    }
}
