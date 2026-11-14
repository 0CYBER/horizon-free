//! The glass look's frame (the mockup's `launcher-app.tsx`): the sidebar plate with the
//! instances, the launcher's update, Sculk's card, Discord and the links; the top bar with the
//! pages in a glass tab strip, the player, Microsoft sign-in and Settings; and the menus they open.
//! Dark appearance only for now (`docs/design-system.md`, "Glass").
use super::*;
use crate::glass_ui::{self as ui, white, Tone};
use crate::instances_page::java_line;
use gpui::{anchored, canvas, deferred, point};
use horizon_launcher::instances::{Icon, Instance, Kind as InstanceKind};
use horizon_theme::glass::{fill, ink, radius};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The rail's distance from the window's edges (`m-3.5`).
pub const MARGIN: f32 = 14.;

/// The rail's width at rest (icons) and open (icons and names), while the pointer is on it.
pub const RAIL: f32 = 80.;
pub const RAIL_OPEN: f32 = 248.;
/// How the rail opens and shuts: quick, settling from a little past its width.
const RAIL_MOTION: horizon_theme::Motion = horizon_theme::Motion {
    name: "rail",
    ms: 320,
    curve: [0.22, 0.9, 0.3, 1.08],
};
/// Discord's own blurple, in every theme, and under the pointer.
const DISCORD: u32 = 0x5865F2;
const DISCORD_HOVER: u32 = 0x4752C4;

/// The rail's state: whether the pointer is on it, and where its last opening or shutting began.
#[derive(Default)]
pub struct Rail {
    pub hovered: bool,
    open: bool,
    since: Option<Instant>,
    from: f32,
    at: f32,
}

impl Rail {
    /// How far open the rail is, 0 to 1 (a little past 1 as it settles). `pinned`: something it
    /// opened is still open (a row's menu), so it stays open without the pointer. Frames are asked
    /// for only while it moves; under reduced motion it jumps.
    pub fn progress(&mut self, window: &mut Window, pinned: bool) -> f32 {
        // `HORIZON_LAUNCHER_RAIL_OPEN=1` holds it open, for screenshots.
        static HELD: std::sync::LazyLock<bool> =
            std::sync::LazyLock::new(|| std::env::var_os("HORIZON_LAUNCHER_RAIL_OPEN").is_some());
        let want = self.hovered || pinned || *HELD;
        if want != self.open {
            self.open = want;
            self.from = self.at;
            self.since = Some(Instant::now());
        }
        let to = if want { 1. } else { 0. };
        let t = match self.since {
            Some(since) if !crate::motion::still() => {
                since.elapsed().as_secs_f32() / RAIL_MOTION.secs()
            }
            _ => 1.,
        };
        if t >= 1. {
            self.since = None;
            self.at = to;
            return to;
        }
        self.at = self.from + (to - self.from) * RAIL_MOTION.ease(t);
        window.request_animation_frame();
        self.at
    }
}

/// The ⋮ menu of a sidebar row while open: whose, and whether Delete… was pressed once.
#[derive(Clone, Default)]
pub struct RowMenu {
    pub open: Option<(String, bool)>,
    /// Where each row was drawn last, by instance id: the menu opens beside its row.
    pub rows: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
}

/// Records where the element it sits in is drawn, under `id`.
fn measure(rows: &Rc<RefCell<HashMap<String, Bounds<Pixels>>>>, id: String) -> impl IntoElement {
    let rows = rows.clone();
    canvas(
        move |bounds, _, _| {
            rows.borrow_mut().insert(id, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Launcher {
    /// An instance's icon (`InstanceIcon`), `size` square with no frame: Horizon's mark tile for
    /// Horizon's own instance; else its picture or the block chosen for it, round at 26 %; else
    /// its loader's or Minecraft's mark; else its initial on its colour (the accent when none was
    /// chosen), as `ColorTile` draws it.
    pub(super) fn glass_instance_icon(&mut self, i: &Instance, size: f32) -> AnyElement {
        if i.kind == InstanceKind::Default && i.icon.is_none() {
            return self.horizon_mark(size);
        }
        if let Some(picture) = self.instance_picture(i, size) {
            return instance_icons::centred(picture, size, self.scale).into_any_element();
        }
        let r = px((size * 0.26).round());
        let mut colour = self.look.accent;
        match &i.icon {
            Some(Icon::Colour(hex)) => {
                colour = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(colour);
            }
            Some(Icon::Texture(name)) => {
                if let Some(image) = self.texture_image(name, size) {
                    return gpui::img(image)
                        .flex_none()
                        .size(px(size))
                        .rounded(r)
                        .into_any_element();
                }
            }
            None => {
                if let Some(mark) = self.instance_mark(i, size) {
                    return instance_icons::centred(mark, size, self.scale).into_any_element();
                }
            }
        }
        // Bricolage at the optical size a browser would pick for the letter.
        let letter = (size * 0.46).round();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded(r)
            .bg(ui::color(colour))
            .font_family(if letter >= 40. {
                horizon_theme::glass::DISPLAY_FONT
            } else {
                horizon_theme::glass::HEADING_FONT
            })
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(letter))
            .line_height(px(letter))
            .text_color(gpui::white())
            .child(instances_page::initial(&i.name))
            .into_any_element()
    }

    /// A sidebar row's state line and its dot (the mockup's `statusOf`), from what the launcher
    /// knows of the instance.
    fn row_state(&self, i: &Instance) -> (Tone, String) {
        let running =
            self.client.is_some() && self.selected_instance().is_some_and(|s| s.id == i.id);
        if running {
            return (Tone::Live, tr("Running").into());
        }
        let starting = self.starting_client.is_some()
            && self.selected_instance().is_some_and(|s| s.id == i.id);
        if starting {
            return (Tone::Busy, tr("Starting").into());
        }
        match i.kind {
            InstanceKind::Default | InstanceKind::Own => {
                let tag = self.version_of(i).cloned();
                let prepared = tag.as_deref().is_some_and(backend::is_prepared);
                match (&i.java, &tag, &i.version) {
                    (Some(java), _, _) => (Tone::Live, java_line(java)),
                    (None, None, Some(own)) => (
                        Tone::Warn,
                        trf("{version} is not installed", &[("version", own)]),
                    ),
                    (None, None, None) => (Tone::Off, tr("No version installed").into()),
                    // Following the launcher's version, which has a newer one waiting.
                    (None, Some(_), None) if self.updates.horizon.is_some() => (
                        Tone::Accent(self.look.accent_soft),
                        tr("Update available").into(),
                    ),
                    (None, Some(_), _) if !prepared => (Tone::Warn, tr("Needs preparing").into()),
                    (None, Some(tag), _) if self.native_build_of(tag).is_some() => {
                        let build = self.native_build_of(tag).unwrap_or_default();
                        (
                            Tone::Live,
                            trf(
                                "Native build · {version}",
                                &[("version", &build.horizon_short())],
                            ),
                        )
                    }
                    (None, Some(tag), Some(_)) => (Tone::Live, format!("{} · {tag}", tr("Ready"))),
                    (None, Some(_), None) => (Tone::Live, tr("Ready").into()),
                }
            }
            InstanceKind::Port => {
                let slug = &i.id["port:".len()..];
                let summary = self.packs.iter().find(|p| p.slug == slug);
                (
                    Tone::Live,
                    summary.map_or_else(|| tr("Modpack").into(), |p| p.summary()),
                )
            }
            InstanceKind::Pack => (Tone::Off, tr("From Horizon's registry").into()),
        }
    }

    /// The instances by group (`sections` in the mockup): those without a group first.
    /// `open`: the rail shows names (else icons alone, the name on hover); `label`: how far the
    /// names have faded in.
    fn glass_instances(
        &mut self,
        open: bool,
        label: f32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = self.theme;
        let selected = self.selected_instance().map(|i| i.id.clone());
        let on_packs = self.page == Page::Mods && self.mods.on_modpacks();
        let mut out = Vec::new();
        let mut group = None;
        let mut list = div().flex().flex_col().gap(px(4.));
        let flush = |list: Div, out: &mut Vec<AnyElement>| {
            out.push(list.into_any_element());
            div().flex().flex_col().gap(px(4.))
        };
        for (order, i) in self.instances.list.clone().into_iter().enumerate() {
            if i.group != group {
                list = flush(list, &mut out);
                group = i.group.clone();
                if let Some(g) = group.as_ref().filter(|_| open) {
                    out.push(
                        ui::text(12., FontWeight::MEDIUM, ink::DIM)
                            .px(px(8.))
                            .pt(px(16.))
                            .pb(px(6.))
                            .opacity(label)
                            .whitespace_nowrap()
                            .child(g.clone())
                            .into_any_element(),
                    );
                }
            }
            // As in the mockup, the chosen instance stays marked on every page, unless a ported
            // pack's page is open: that row is the one shown then.
            let current = match i.kind {
                InstanceKind::Default | InstanceKind::Own => {
                    !(on_packs && self.pack_open.is_some()) && selected.as_ref() == Some(&i.id)
                }
                InstanceKind::Port => {
                    on_packs && self.pack_open.as_deref() == Some(&i.id["port:".len()..])
                }
                InstanceKind::Pack => false,
            };
            let state = self.row_state(&i);
            let icon = self.glass_instance_icon(&i, 38.);
            let clicked = i.clone();
            let menu_open = self
                .row_menu
                .open
                .as_ref()
                .is_some_and(|(id, _)| *id == i.id);
            let menu_id = i.id.clone();
            // A right click on the row opens the same menu the ⋮ does.
            let right_id = i.id.clone();
            let more = div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(6.))
                .flex()
                .items_center()
                .child(
                    ui::icon_button(
                        format!("side-more-{}", i.id),
                        "more-vertical",
                        32.,
                        18.,
                        radius::SMALL,
                        menu_open,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, event: &ClickEvent, _, cx| {
                            if this.closed_by_this_press(event) {
                                return;
                            }
                            this.row_menu.open =
                                row_menu_after_press(this.row_menu.open.take(), &menu_id);
                            cx.notify();
                        },
                    )),
                );
            list = list.child(crate::motion::enter(
                ElementId::named_usize(format!("side-enter-{}", i.id), 0),
                order,
                div()
                    .relative()
                    .child(measure(&self.row_menu.rows, i.id.clone()))
                    .child(
                        ui::row(
                            format!("side-instance-{}", i.id),
                            current,
                            if open { 40. } else { 8. },
                            &t,
                        )
                        .child(div().relative().flex_none().child(icon))
                        .child(
                            ui::row_text(i.name.clone(), state)
                                .overflow_hidden()
                                .opacity(label),
                        )
                        .when(!open, |d| d.tooltip(ui::tip(i.name.clone())))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            match clicked.kind {
                                InstanceKind::Default | InstanceKind::Own => {
                                    this.select_instance(&clicked);
                                    this.go(Page::Play);
                                }
                                InstanceKind::Port => {
                                    this.show_modpack(Some(clicked.id["port:".len()..].into()))
                                }
                                InstanceKind::Pack => this.show_modpack(None),
                            }
                            cx.notify();
                        }))
                        // The ⋮ menu, from a right click anywhere on the row.
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                // The open menu's mouse-down-out already closed it on this
                                // press: reopening here would make the toggle a no-op.
                                if this.menu_closed_at.take() == Some(event.position) {
                                    cx.notify();
                                    return;
                                }
                                this.row_menu.open =
                                    row_menu_after_press(this.row_menu.open.take(), &right_id);
                                cx.notify();
                            }),
                        ),
                    )
                    .when(open, |d| d.child(more)),
            ));
        }
        out.push(list.into_any_element());
        out
    }

    /// The rail: a strip of instance icons that opens into names, states and links while the
    /// pointer is on it (or one of its menus is open). The page beside it gives way as it opens,
    /// so nothing is ever covered. Its foot keeps the launcher's update, Sculk's card, Discord,
    /// the project's links, Settings and the version.
    pub(super) fn glass_rail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let soft = self.look.accent_soft;
        self.load_packs();
        let pinned = self.row_menu.open.is_some();
        let p = self.rail.progress(window, pinned);
        let open = p > 0.5;
        // Names and lines fade in over the second half of the opening, as the width arrives.
        let label = ((p - 0.35) / 0.65).clamp(0., 1.);
        let fade = move |d: Div| d.min_w_0().overflow_hidden().opacity(label);

        // A rail button: an icon, its name beside it once the rail opens.
        let button = |id: SharedString, icon: &'static str, name: SharedString, active: bool| {
            keyboard(
                track_hover(
                    div()
                        .id(id.clone())
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(12.))
                        .w_full()
                        .h(px(42.))
                        .px(px(17.5))
                        .rounded(px(radius::ROW))
                        .border_1()
                        .border_color(if active {
                            white(ui::edge::STRONG)
                        } else {
                            gpui::transparent_black()
                        })
                        .when(active, |d| d.bg(white(fill::ACTIVE)))
                        .cursor_pointer()
                        .child(ui::hover_edge(&id, radius::ROW, fill::ROW_HOVER, true))
                        .child(ui::icon(icon, 19., white(ink::SOFT)).relative())
                        .child(
                            fade(ui::text(14., FontWeight::MEDIUM, ink::STRONG).relative())
                                .whitespace_nowrap()
                                .child(name.clone()),
                        )
                        // Narrow, the rail shows icons alone: the name on hover.
                        .when(!open, |d| d.tooltip(ui::tip(name))),
                    id,
                ),
                &t,
            )
        };

        // Add instance: the accent's outline, not the sun gradient, so Play stays the one bright
        // button on Home.
        let add = keyboard(
            track_hover(
                div()
                    .id("add-instance")
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(12.))
                    .w_full()
                    .h(px(42.))
                    .px(px(17.5))
                    .rounded(px(radius::ROW))
                    .bg(ui::color(self.look.accent).opacity(0.08))
                    .border_1()
                    .border_color(ui::color(self.look.accent).opacity(0.5))
                    .hover(|s| {
                        s.bg(ui::color(self.look.accent).opacity(0.16))
                            .border_color(ui::color(self.look.accent).opacity(0.8))
                    })
                    .cursor_pointer()
                    .child(ui::icon("plus-sign", 19., ui::color(soft)))
                    .child(
                        fade(ui::text(14., FontWeight::SEMIBOLD, ink::FULL))
                            .whitespace_nowrap()
                            .child(tr("Add instance")),
                    )
                    .when(!open, |d| d.tooltip(ui::tip(tr("Add instance"))))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_instance_dialog(None);
                        cx.notify();
                    })),
                "add-instance".into(),
            ),
            &t,
        );

        let mut list = div()
            .id("side-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(self.glass_instances(open, label, cx));

        // The player's hosted servers, one more section under the instances.
        let servers: Vec<_> = self
            .servers
            .sidebar_list()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| self.side_servers.clone());
        if !servers.is_empty() {
            list = list.child(
                div()
                    .h(px(22.))
                    .flex()
                    .items_end()
                    .px(px(8.))
                    .pb(px(4.))
                    .child(
                        fade(ui::text(12., FontWeight::MEDIUM, ink::DIM))
                            .whitespace_nowrap()
                            .child(tr("Servers")),
                    ),
            );
        }
        let mut server_rows = div().flex().flex_col().gap(px(2.));
        for e in &servers {
            let up = self.servers.is_up(&e.dir);
            let shown = self.page == Page::Servers && self.servers.open.as_ref() == Some(&e.dir);
            let dir = e.dir.clone();
            server_rows = server_rows.child(
                ui::row(format!("side-server-{}", e.dir.display()), shown, 8., &t)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(38.))
                            .rounded(px(radius::CONTROL))
                            .bg(white(fill::BUTTON))
                            .child(ui::icon("computer-terminal-01", 18., white(ink::STRONG))),
                    )
                    .child(fade(ui::row_text(
                        e.name.clone(),
                        if up {
                            (Tone::Live, tr("Running").into())
                        } else {
                            (Tone::Off, tr("Stopped").into())
                        },
                    )))
                    .when(!open, |d| d.tooltip(ui::tip(e.name.clone())))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_server(dir.clone());
                        this.go(Page::Servers);
                        cx.notify();
                    })),
            );
        }
        let list = list.child(server_rows);

        // A hairline set in from the rail's sides: between the instances and the buttons, and
        // between Settings and the version.
        let rule = || {
            div()
                .flex_none()
                .h(px(1.))
                .mx(px(10.))
                .my(px(4.))
                .bg(white(0.1))
        };
        let mut foot = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2.))
            .pt(px(4.))
            .child(rule());
        // The launcher's own update: the card while open; a dot on Settings while narrow.
        let updating = match self.self_update {
            SelfUpdate::Downloading(..) | SelfUpdate::Ready(_) => true,
            SelfUpdate::Stuck(_) => !self.self_update_hidden,
            _ => false,
        };
        if open {
            foot = foot.children(self.rail_update_card(soft, cx));
            // Sculk's card (a sponsor), only where the rail is wide enough to show it.
            if !self.settings.partner_hidden && p > 0.98 {
                let rows = self.instances.list.len() + servers.len();
                let free = f32::from(window.viewport_size().height) - 420. - 52. * rows as f32;
                let mut side = div().relative().mb(px(6.)).child(partner::card(
                    "partner-side",
                    RAIL_OPEN - 24.,
                    free < partner::TALL_HEIGHT,
                    &t,
                ));
                if self.settings.partner_seen_secs >= partner::SEEN_BEFORE_HIDE {
                    side = side.child(partner::hide_button("partner-hide", &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.settings.partner_hidden = true;
                            this.save_settings();
                            cx.notify();
                        }),
                    ));
                }
                foot = foot.child(side);
            }
        }

        // Discord keeps its own blurple in every theme: a filled square while the rail is
        // narrow, the square and "Join the Discord" while it is open. No pulse.
        if !self.links.discord.is_empty() {
            let url = self.links.discord.clone();
            let did = SharedString::from("discord");
            let hovered = crate::hover_is(&did);
            foot = foot.child(keyboard(
                track_hover(
                    div()
                        .id(did.clone())
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(12.))
                        .w_full()
                        .h(px(42.))
                        .mb(px(4.))
                        .px(px(17.5))
                        .rounded(px(radius::ROW))
                        .bg(ui::color(if hovered { DISCORD_HOVER } else { DISCORD }))
                        .shadow(ui::glows(vec![
                            gpui::BoxShadow {
                                color: gpui::white().opacity(if hovered { 0.2 } else { 0.12 }),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.5),
                                spread_radius: px(0.5),
                                inset: true,
                            },
                            gpui::BoxShadow {
                                color: ui::color(DISCORD).opacity(if hovered { 0.9 } else { 0.7 }),
                                offset: point(px(0.), px(if hovered { 10. } else { 6. })),
                                blur_radius: px(if hovered { 28. } else { 20. }),
                                spread_radius: px(-6.),
                                inset: false,
                            },
                        ]))
                        .cursor_pointer()
                        .child(ui::icon("discord", 19., gpui::white()))
                        .child(
                            fade(ui::text(14., FontWeight::SEMIBOLD, 1.))
                                .text_color(gpui::white())
                                .whitespace_nowrap()
                                .child(tr("Join the Discord")),
                        )
                        .when(!open, |d| d.tooltip(ui::tip(tr("Join the Discord"))))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                    did,
                ),
                &t,
            ));
        }
        let links = [
            (
                "sponsor",
                tr("Sponsor"),
                "favourite",
                self.links.sponsor.clone(),
            ),
            ("roadmap", tr("Roadmap"), "map", self.links.roadmap.clone()),
            ("docs", tr("Documentation"), "book", self.links.docs.clone()),
            (
                "website",
                tr("Website"),
                "globe-02",
                self.links.website.clone(),
            ),
        ];
        for (id, name, icon, url) in links {
            if url.is_empty() {
                continue;
            }
            foot = foot.child(
                button(SharedString::from(id), icon, name.into(), false)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        let settings_open = self.page == Page::Settings;
        foot = foot.child(
            button(
                "rail-settings".into(),
                "sliders-horizontal",
                tr("Settings").into(),
                settings_open,
            )
            .child(
                // The launcher's update waits: a dot on Settings while the rail is narrow.
                div()
                    .absolute()
                    .top(px(9.))
                    .left(px(27.))
                    .when(!(updating && !open), |d| d.invisible())
                    .child(ui::dot(Tone::Accent(soft))),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.go(Page::Settings);
                cx.notify();
            })),
        );
        // The launcher's version under it: "Launcher 0.2.0" open, the number alone while narrow.
        let version: SharedString = if open {
            trf("Launcher {version}", &[("version", backend::VERSION)]).into()
        } else {
            backend::VERSION.into()
        };
        foot = foot.child(rule()).child(
            div()
                .flex()
                .items_center()
                .h(px(20.))
                .px(px(if open { 12. } else { 0. }))
                .when(!open, |d| d.justify_center())
                .child(
                    ui::text(11.5, FontWeight::NORMAL, ink::DIM)
                        .id("launcher-version")
                        .whitespace_nowrap()
                        .truncate()
                        .child(version),
                ),
        );

        let width = RAIL + (RAIL_OPEN - RAIL) * p;
        crate::glass::plate(radius::PANEL)
            .id("rail")
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(10.))
            .w(px(width))
            .px(px(12.))
            .py(px(12.))
            .overflow_hidden()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.rail.hovered = *hovered;
                cx.notify();
            }))
            .child(add)
            .child(list.relative())
            .child(foot.relative())
    }

    /// The launcher's own update, as a card at the open rail's foot (while one downloads, waits,
    /// or could not be applied).
    fn rail_update_card(&mut self, soft: u32, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let card = |title: String, right: Option<String>| {
            div()
                .flex()
                .flex_col()
                .mb(px(6.))
                .rounded(px(radius::ROW))
                .bg(white(fill::FIELD))
                .border_1()
                .border_color(white(ui::edge::REST))
                .p(px(12.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .child(
                            ui::text(13.5, FontWeight::SEMIBOLD, ink::FULL)
                                .flex_1()
                                .min_w_0()
                                .child(title),
                        )
                        .children(
                            right.map(|r| ui::text(12.5, FontWeight::NORMAL, ink::NOTE).child(r)),
                        ),
                )
        };
        match &self.self_update {
            SelfUpdate::Downloading(version, f) => Some(
                card(
                    trf("Launcher {version}", &[("version", version)]),
                    Some(format!("{:.0}%", f * 100.)),
                )
                .child(div().mt(px(10.)).child(ui::bar(*f, soft))),
            ),
            SelfUpdate::Ready(version) => Some(
                card(tr("A new version is available").to_string(), None)
                    .child(
                        ui::text(12.5, FontWeight::NORMAL, ink::NOTE)
                            .mt(px(4.))
                            .child(trf("Launcher {version}", &[("version", version)])),
                    )
                    .child(
                        ui::solid("update-launcher", tr("Update"), 32., 13., &t)
                            .mt(px(10.))
                            .w_full()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.apply_launcher_update(cx);
                                cx.notify();
                            })),
                    ),
            ),
            SelfUpdate::Stuck(error) if !self.self_update_hidden => {
                let why = self.launcher_update_help(error.as_deref());
                let url = backend::download_page(&self.links.website);
                Some(
                    card(tr("A new version is available").to_string(), None)
                        .child(
                            ui::text(12.5, FontWeight::NORMAL, ink::NOTE)
                                .mt(px(6.))
                                .child(why),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .mt(px(10.))
                                .when(self.launcher_download_available(), |actions| {
                                    actions.child(
                                        ui::solid(
                                            "download-launcher",
                                            tr("Download again"),
                                            32.,
                                            13.,
                                            &t,
                                        )
                                        .w_full()
                                        .on_click(move |_, _, cx| cx.open_url(&url)),
                                    )
                                })
                                .child(
                                    ui::solid(
                                        "dismiss-launcher-update",
                                        tr("Dismiss"),
                                        32.,
                                        13.,
                                        &t,
                                    )
                                    .w_full()
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, _, cx| {
                                            this.self_update_hidden = true;
                                            cx.notify();
                                        },
                                    )),
                                ),
                        ),
                )
            }
            _ => None,
        }
    }

    /// The glass top bar, across the window above the rail and the page: the edition switch over
    /// the rail, the pages, then search (the palette), the last native build's log and the player.
    /// Launcher settings live in the rail.
    pub(super) fn glass_topbar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let wide = self.window_width >= crate::glass_home::TOP_BAR_WIDE;
        let compact = self.window_width < crate::glass_home::COMPACT;
        let mark = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(RAIL))
            .child(self.edition_switch(cx));
        let mut tabs = div()
            .flex()
            .items_center()
            .gap(px(if compact { 2. } else { 4. }))
            .min_w_0();
        for (page, label, _) in Page::in_top_bar(self.wardrobe_shown()) {
            let mut item = ui::nav_tab(format!("nav-{label}"), tr(label), self.page == page, &t);
            if compact {
                item = item.px(px(8.));
            }
            tabs = tabs.child(
                item.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(page);
                    cx.notify();
                })),
            );
        }
        let strip = div()
            .flex()
            .items_center()
            .min_w_0()
            .when(wide, |d| d.flex_none())
            .when(!wide, |d| d.flex_shrink(1.).overflow_hidden())
            .child(tabs.id("nav-pages").relative().when(!wide, |d| {
                d.overflow_x_scroll()
                    .on_scroll(|_, _, cx| cx.stop_propagation())
            }));

        let (name, how) = self.who_plays();
        let face = self.accounts.selected().map(|a| a.uuid.clone());
        let open = self.account_menu;
        // Signed out, the chip is only the face: the Sign in with Microsoft button beside it
        // says it once, and the page tabs keep their room.
        let signed_in = face.is_some();
        let chip =
            div()
                .id("account")
                .relative()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(44.))
                .pl(px(8.))
                .pr(px(12.))
                .rounded(px(radius::ROW))
                .bg(white(if open { fill::ACTIVE } else { fill::ROW_HOVER }))
                .when(!open, |d| d.hover(|s| s.bg(white(0.09))))
                .cursor_pointer()
                .child(
                    div().size(px(26.)).rounded(px(7.)).overflow_hidden().child(
                        self.profile_heads
                            .draw(face.as_deref(), 26., self.scale, &t),
                    ),
                )
                .children(signed_in.then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .max_w(px(if compact { 100. } else { 150. }))
                        .child(
                            ui::text(14., FontWeight::SEMIBOLD, ink::FULL)
                                .line_height(px(17.))
                                .truncate()
                                .child(name),
                        )
                        .children((!compact).then(|| {
                            ui::text(12., FontWeight::NORMAL, ink::QUIET)
                                .line_height(px(15.))
                                .child(how)
                        }))
                }))
                .child(
                    ui::icon("arrow-down-01", 16., white(ink::FAINT))
                        .ml(px(4.))
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            if open { std::f32::consts::PI } else { 0. },
                        ))),
                )
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if this.closed_by_this_press(event) {
                        return;
                    }
                    this.account_menu = !this.account_menu;
                    if this.account_menu {
                        this.accounts = Accounts::load();
                        this.load_profile_heads();
                    }
                    cx.notify();
                }));
        let account = div()
            .relative()
            .child(keyboard(chip, &t))
            .children(self.glass_account_menu(cx));

        // Signing in with Microsoft, while nobody is: the mockup's button.
        let sign_in = self.accounts.selected().is_none().then(|| {
            keyboard(
                div()
                    .id("topbar-sign-in")
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(44.))
                    .px(px(16.))
                    .rounded(px(radius::ROW))
                    .bg(white(fill::BUTTON))
                    .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                    .cursor_pointer()
                    .child(microsoft::logo(16.))
                    .child(
                        ui::text(14., FontWeight::MEDIUM, ink::FULL)
                            .child(tr("Sign in with Microsoft")),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.start_sign_in();
                        cx.notify();
                    })),
                &t,
            )
        });
        // Search: the palette (⌘K or Ctrl+K anywhere), as a field-shaped button.
        let sid = SharedString::from("topbar-search");
        let key: &str = if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            tr("Ctrl K")
        };
        let search = keyboard(
            track_hover(
                div()
                    .id(sid.clone())
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(38.))
                    .w(px(if compact { 38. } else { 230. }))
                    .when(compact, |d| d.justify_center())
                    .when(!compact, |d| d.px(px(12.)))
                    .rounded(px(radius::CONTROL))
                    .bg(white(fill::FIELD))
                    .border_1()
                    .border_color(white(ui::edge::REST))
                    .cursor_pointer()
                    .child(ui::hover_edge(&sid, radius::CONTROL, fill::HOVER, true))
                    .child(ui::icon("search-01", 16., white(ink::QUIET)).relative())
                    .when(!compact, |d| {
                        d.child(
                            ui::text(13.5, FontWeight::NORMAL, ink::QUIET)
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(tr("Search…")),
                        )
                        .child(
                            ui::text(11.5, FontWeight::MEDIUM, ink::DIM)
                                .relative()
                                .flex()
                                .flex_none()
                                .items_center()
                                .h(px(20.))
                                .px(px(6.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(white(ui::edge::REST))
                                .child(key.to_string()),
                        )
                    })
                    .when(compact, |d| d.tooltip(ui::tip(tr("Search…"))))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.palette_toggle(window, cx);
                        cx.notify();
                    })),
                sid,
            ),
            &t,
        );
        // The last native build's log, as a quiet button.
        let build_log = self.native_log.available.then(|| {
            keyboard(
                div()
                    .id("topbar-native-log")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(38.))
                    .px(px(12.))
                    .rounded(px(radius::CONTROL))
                    .bg(white(fill::BUTTON))
                    .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                    .cursor_pointer()
                    .child(ui::icon("computer-terminal-01", 16., white(ink::SOFT)))
                    .when(!compact, |d| {
                        d.child(ui::text(14., FontWeight::MEDIUM, ink::FULL).child(tr("Build log")))
                    })
                    .when(compact, |d| d.tooltip(ui::tip(tr("Build log"))))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.native_log_open();
                        cx.notify();
                    })),
                &t,
            )
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(64.))
            .pl(px(MARGIN))
            .pr(px(MARGIN + 8.))
            .pt(px(4.))
            .child(mark)
            .child(strip)
            .children(self.toast_in_topbar())
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .child(search)
                    .children(build_log)
                    .child(account)
                    .children(sign_in),
            )
    }

    /// The menu under the player (`Switch player`): everyone who can play, then Manage accounts.
    fn glass_account_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.account_menu {
            return None;
        }
        let t = self.theme;
        let soft = self.look.accent_soft;
        let now = self.accounts.selected().map(|a| a.uuid.clone());
        let mut menu = ui::popover(250.)
            .id("account-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.account_menu = false;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }));
        for (i, (who, name, how)) in self.players().into_iter().enumerate() {
            let chosen = Some(&who) == now.as_ref();
            let id = SharedString::from(format!("account-pick-{i}"));
            menu = menu.child(keyboard(
                track_hover(
                    div()
                        .id(id.clone())
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .h(px(48.))
                        .px(px(10.))
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .child(hover_layer(&id, white(0.07), radius::CONTROL, 0.))
                        .child(
                            div()
                                .relative()
                                .size(px(30.))
                                .rounded(px(8.))
                                .overflow_hidden()
                                .child(self.profile_heads.draw(
                                    Some(who.as_str()),
                                    30.,
                                    self.scale,
                                    &t,
                                )),
                        )
                        .child(
                            div()
                                .relative()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    ui::text(14., FontWeight::MEDIUM, ink::FULL)
                                        .truncate()
                                        .child(name),
                                )
                                .child(ui::text(12., FontWeight::NORMAL, ink::FAINT).child(how)),
                        )
                        .when(chosen, |d| {
                            d.child(ui::icon("tick-02", 16., ui::color(soft)).relative())
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(&who);
                            cx.notify();
                        })),
                    id,
                ),
                &t,
            ));
        }
        menu = menu.child(div().mt(px(4.)).child(
            ui::menu_row("account-manage", "user", tr("Manage accounts"), false, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.account_menu = false;
                    this.go(Page::Settings);
                    cx.notify();
                }),
            ),
        ));
        Some(deferred(
            div().absolute().top(px(52.)).right_0().child(menu),
        ))
    }

    /// The ⋮ menu of a sidebar row, beside it: Edit…, Duplicate…, Open folder, Export…,
    /// launch settings, the desktop shortcut, then Delete… (asked twice). Each acts on that instance.
    pub(super) fn row_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (id, confirm) = self.row_menu.open.clone()?;
        let Some(instance) = self.instances.list.iter().find(|i| i.id == id).cloned() else {
            self.row_menu.open = None;
            return None;
        };
        let row = self.row_menu.rows.borrow().get(&id).copied()?;
        let t = self.theme;
        let viewport = window.viewport_size();
        let height = f32::from(viewport.height);
        // The mockup's `rect.right + 12`, `min(rect.top - 8, innerHeight - 330)`: its ⋮ button
        // ends 6 px inside the row and sits in the row's middle.
        let x = f32::from(row.origin.x + row.size.width) - 6. + 12.;
        let button_top = f32::from(row.origin.y) + (f32::from(row.size.height) - 32.) / 2.;
        let y = (button_top - 8.).min(height - 330.).max(8.);
        let item = |key: &'static str, icon: &'static str, label: String| {
            ui::menu_row(format!("row-menu-{key}"), icon, label, false, &t)
        };
        let pick = |this: &mut Launcher, i: &Instance| {
            this.row_menu.open = None;
            this.select_instance(i);
            this.go(Page::Play);
        };
        let (edit, copy, export, launch, shortcut) = (
            instance.clone(),
            instance.clone(),
            instance.clone(),
            instance.clone(),
            instance.clone(),
        );
        let home = instance.home.clone();
        let labels = [
            tr("Edit…"),
            tr("Duplicate…"),
            tr("Open folder"),
            tr("Export…"),
            tr("Launch…"),
            if self.has_shortcut(&instance.id) {
                tr("Remove the desktop shortcut")
            } else {
                tr("Create a desktop shortcut")
            },
            if confirm {
                tr("Click again to delete")
            } else {
                tr("Delete…")
            },
        ];
        let [edit_label, copy_label, folder_label, export_label, launch_label, shortcut_label, delete_label] =
            labels;
        let viewport_width = f32::from(viewport.width);
        let width = ui::menu_content_width(window, &labels)
            .max(250.)
            .min((viewport_width - 16.).max(0.));
        let x = x.min(viewport_width - width - 8.).max(8.);
        let mut menu = ui::popover(width)
            .id("row-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.row_menu.open = None;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }))
            .child(
                item("edit", "pencil-edit-02", edit_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.row_menu.open = None;
                        this.open_instance_sheet(edit.clone(), glass_sheet::SheetTab::General);
                        cx.notify();
                    },
                )),
            )
            .child(
                item("duplicate", "copy-01", copy_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        pick(this, &copy);
                        this.start_copy();
                        cx.notify();
                    },
                )),
            )
            .child(
                item("folder", "folder-01", folder_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.row_menu.open = None;
                        let _ = std::fs::create_dir_all(&home);
                        crate::glass_toast::open(cx, &home);
                        cx.notify();
                    },
                )),
            )
            .child(
                item("export", "upload-04", export_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        pick(this, &export);
                        this.open_instance_sheet(export.clone(), glass_sheet::SheetTab::Storage);
                        cx.notify();
                    },
                )),
            )
            .child(
                item("launch", "sliders-horizontal", launch_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        pick(this, &launch);
                        this.open_instance_sheet(launch.clone(), glass_sheet::SheetTab::Launch);
                        cx.notify();
                    },
                )),
            )
            .child(
                item("shortcut", "link-01", shortcut_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.row_menu.open = None;
                        this.toggle_shortcut(shortcut.id.clone(), shortcut.name.clone());
                        cx.notify();
                    },
                )),
            );
        // Horizon's own instance can't be deleted, nor one while a game runs.
        let can_delete = instance.kind != InstanceKind::Default
            && self.client.is_none()
            && self.starting_client.is_none();
        let gone = instance.id.clone();
        let delete = ui::menu_row("row-menu-delete", "delete-02", delete_label, true, &t);
        let delete = if can_delete {
            delete.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some((_, confirm)) = this.row_menu.open.as_mut().filter(|(_, c)| !*c) {
                    *confirm = true;
                } else {
                    this.row_menu.open = None;
                    this.delete_instance(&gone);
                }
                cx.notify();
            }))
        } else {
            delete.opacity(0.4).cursor_default()
        };
        menu = menu.child(div().mt(px(4.)).child(delete));
        Some(
            deferred(anchored().position(point(px(x), px(y))).child(menu))
                .with_priority(1)
                .into_any_element(),
        )
    }
}
