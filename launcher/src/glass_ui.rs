//! The glass look's controls ("Glass" in `docs/design-system.md`): the parts of the mockup's
//! `components/launcher/launcher-ui.tsx` and its shell (`launcher-app.tsx`), at its sizes. White
//! text and fills at a few opacities over [`glass`](crate::glass) plates; Geist throughout.
use crate::{hover_fade, hover_is, keyboard, note_hover, theme::Theme, track_hover, Kind};
use gpui::{
    canvas, div, prelude::*, px, rgb, rgba, svg, AnimationExt, AnyElement, App, ClickEvent, Div,
    ElementId, FontWeight, Hsla, Pixels, SharedString, Stateful, Svg, TextRun, Window,
};
use horizon_launcher::effects::Effects;
use horizon_theme::glass::{fill, ink, radius, Glass};
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// Whether this frame's pages draw the glass look (the dark appearance).
    static ON: Cell<bool> = const { Cell::new(false) };
    /// The look's colours for the player's accent.
    static LOOK: Cell<Glass> = Cell::new(Glass::default_look());
    /// Whether the rows being built go in a Settings card ([`flat_rows`]).
    static FLAT: Cell<bool> = const { Cell::new(false) };
    /// The light appearance: the glass look's white ink and fills turn to [`INK`].
    static LIGHT: Cell<bool> = const { Cell::new(false) };
    /// The player's flourishes ([`effects`]).
    static EFFECTS: Cell<Effects> = const { Cell::new(Effects::ALL) };
    /// How many [`hover_back`] layers this frame has built so far: the next one's name.
    static BACKS: Cell<usize> = const { Cell::new(0) };
    /// Each [`hover_back`] layer's pointer: on or off, and since when.
    static BACK_HOVER: RefCell<HashMap<SharedString, (bool, std::time::Instant)>> =
        RefCell::new(HashMap::new());
}

/// The light appearance's ink: the mockup's text on white (`#100a1f`), at the opacities the dark
/// look gives white.
const INK: u32 = horizon_theme::glass::ON_LIGHT;

/// Set as each frame's pages start being built: whether the appearance is light.
pub fn set_light(light: bool) {
    LIGHT.set(light);
}

/// Whether the light appearance is drawn.
pub fn light() -> bool {
    LIGHT.get()
}

/// A solid control's fill (`SOLID`'s white, the sliders' knob): white, or the ink in light.
pub fn paper() -> Hsla {
    if light() {
        color(INK)
    } else {
        gpui::white()
    }
}

/// Text on [`paper`].
pub fn on_paper() -> Hsla {
    if light() {
        gpui::white()
    } else {
        color(INK)
    }
}

/// The look's green (`--live`): darker in light, to read on white plates.
pub fn live() -> Hsla {
    color(if light() {
        0x15803d
    } else {
        horizon_theme::glass::LIVE
    })
}

/// The look's amber (`--warn`).
pub fn warn() -> Hsla {
    color(if light() {
        0xb45309
    } else {
        horizon_theme::glass::WARN
    })
}

/// The look's red (`--danger`).
pub fn danger() -> Hsla {
    color(if light() {
        0xdc2626
    } else {
        horizon_theme::glass::DANGER
    })
}

/// Builds `f`'s rows as the mockup's `SettingRow`s, for a Settings card: `index_row` and
/// `section_title` ask [`flat`].
pub fn flat_rows<R>(f: impl FnOnce() -> R) -> R {
    let was = FLAT.replace(true);
    let built = f();
    FLAT.set(was);
    built
}

/// Whether rows are being built for a Settings card.
pub fn flat() -> bool {
    FLAT.get()
}

/// A Settings row (`SettingRow`): the name in 15 px semibold, its note in 13 px on 1.5 at 60 %,
/// the control on the right, 40 px away; 16 px around, no fill. On a window too narrow for both,
/// the control wraps under the name (12 px below it), which keeps at least 240 px.
pub fn setting_row(
    title: impl IntoElement,
    note: Option<gpui::AnyElement>,
    control: Option<gpui::AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(40.))
        .gap_y(px(12.))
        .px(px(14.))
        .py(px(13.))
        .rounded(px(12.))
        .border_1()
        .border_color(gpui::transparent_black())
        .relative()
        .child(back_edge(12., 0.03, true))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(240.))
                .gap(px(4.))
                .child(
                    text(14.5, FontWeight::SEMIBOLD, ink::FULL)
                        .line_height(px(20.))
                        .child(title),
                )
                .children(note.map(|n| {
                    text(13., FontWeight::NORMAL, 0.6)
                        .line_height(px(19.5))
                        .max_w(px(560.))
                        .child(n)
                })),
        )
        .children(control.map(|c| div().flex().flex_none().items_center().gap(px(8.)).child(c)))
}

/// A stacked Settings row (`SettingRow stacked`): the name and note, the control under them at
/// full width.
pub fn setting_row_stacked(
    title: impl IntoElement,
    note: Option<gpui::AnyElement>,
    control: gpui::AnyElement,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .px(px(14.))
        .py(px(13.))
        .rounded(px(12.))
        .border_1()
        .border_color(gpui::transparent_black())
        .relative()
        .child(back_edge(12., 0.03, true))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    text(14.5, FontWeight::SEMIBOLD, ink::FULL)
                        .line_height(px(20.))
                        .child(title),
                )
                .children(note.map(|n| {
                    text(13., FontWeight::NORMAL, 0.6)
                        .line_height(px(19.5))
                        .max_w(px(580.))
                        .child(n)
                })),
        )
        .child(div().w_full().child(control))
}

/// The mockup's `WIDE_FIELD` look on a field built for the old one: 44 px, up to 560 px wide,
/// 12 px corners, white at 7 % (11 % while typed in), 14 px Geist.
pub fn wide_field(field: Stateful<Div>, focused: bool) -> Stateful<Div> {
    field
        .w_full()
        .max_w(px(560.))
        .h(px(44.))
        .px(px(16.))
        .border_0()
        .rounded(px(radius::ROW))
        .bg(white(if focused { 0.11 } else { 0.07 }))
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(14.))
        .relative()
        .when(focused, |d| d.child(focus_ring(radius::ROW)))
}

/// The mockup's focus ring (`.hzl :focus-visible`: 2 px of the accent's soft shade) over a
/// focused field, inside its edge with the field's corners: GPUI has no outline, and a field
/// clips what lies outside it. Put it in a `relative` field round at `radius`.
pub fn focus_ring(radius: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(radius))
        .border_2()
        .border_color(color(look().accent_soft))
}

/// Whether the pages being built draw the glass look: helpers with no theme at hand ask.
pub fn on() -> bool {
    ON.get()
}

/// Set as each frame's pages start being built.
pub fn set_on(on: bool) {
    ON.set(on);
    BACKS.set(0);
}

/// Set as each frame's pages start being built: the colours for the player's accent.
pub fn set_look(look: Glass) {
    LOOK.set(look);
}

/// The look's colours.
pub fn look() -> Glass {
    LOOK.get()
}

/// Set as each frame's pages start being built: the flourishes the player keeps.
pub fn set_effects(effects: Effects) {
    EFFECTS.set(effects);
}

/// The flourishes the player keeps (Settings > Appearance).
pub fn effects() -> Effects {
    EFFECTS.get()
}

/// A two-colour fill from `from` to `to` at `angle`, or `from` alone with Gradients off.
pub fn grad(angle: f32, from: Hsla, to: Hsla) -> gpui::Background {
    if effects().gradients {
        gpui::linear_gradient(
            angle,
            gpui::linear_color_stop(from, 0.),
            gpui::linear_color_stop(to, 1.),
        )
    } else {
        from.into()
    }
}

/// `shadows` with Glow on; with it off only the inset ones (a control's edge light, not a halo).
pub fn glows(shadows: Vec<gpui::BoxShadow>) -> Vec<gpui::BoxShadow> {
    if effects().glow {
        shadows
    } else {
        shadows.into_iter().filter(|s| s.inset).collect()
    }
}

/// White at `a`: the dark look's ink and fills. In light, the ink at `a`, so every part keeps
/// its weight on the light plates.
pub fn white(a: f32) -> Hsla {
    if light() {
        color(INK).opacity(a)
    } else {
        gpui::white().opacity(a)
    }
}

/// A colour of the look (`0xrrggbb`).
pub fn color(c: u32) -> Hsla {
    rgb(c).into()
}

/// Text at `size` px in Geist, white at `a`, at `weight`.
pub fn text(size: f32, weight: FontWeight, a: f32) -> Div {
    div()
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(size))
        .line_height(px((size * 1.4).round()))
        .font_weight(weight)
        .text_color(white(a))
}

/// The widths text was laid out in, by key, as [`room`] recorded them on the last frame: what
/// [`cut`] compares a text's own width against.
pub type Rooms = Rc<RefCell<HashMap<SharedString, f32>>>;

/// Fills its (relative) parent and records the parent's laid-out width in `rooms` under `key`.
pub fn room(rooms: &Rooms, key: SharedString) -> impl IntoElement {
    let rooms = rooms.clone();
    gpui::canvas(
        move |bounds, _, _| {
            rooms.borrow_mut().insert(key, f32::from(bounds.size.width));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Whether `text` in `family` at `size` and `weight` is wider than the room recorded under `key`,
/// so a truncating parent cuts it: decides a full-text hover label. False until a room is known.
pub fn cut(
    cx: &App,
    rooms: &Rooms,
    key: &str,
    text: &str,
    family: &'static str,
    size: f32,
    weight: FontWeight,
) -> bool {
    let Some(room) = rooms.borrow().get(key).copied() else {
        return false;
    };
    let system = cx.text_system();
    let font = system.resolve_font(&gpui::Font {
        weight,
        ..gpui::font(family)
    });
    let width: f32 = text
        .chars()
        .filter_map(|c| system.advance(font, px(size), c).ok())
        .map(|a| f32::from(a.width))
        .sum();
    width > room + 0.5
}

/// One of the mockup's Hugeicons, stroked in `color`.
pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/glass/{name}.svg"))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// A layer filling its parent, round at `r`, for hover fills.
fn layer(r: f32) -> impl Fn() -> Div {
    move || div().absolute().top_0().left_0().size_full().rounded(px(r))
}

/// `id`'s fill eased between `from` (white at that opacity) and `to` while hovered, as the
/// mockup's `transition-colors` does.
fn hover_fill(id: &SharedString, r: f32, from: f32, to: f32) -> gpui::AnyElement {
    hover_edge(id, r, to - from, false)
}

/// How opaque white outlines are: every control and inner card at rest, under the pointer, and
/// open or chosen (redesign pass 2: outlines everywhere, stronger as the pointer comes).
pub mod edge {
    pub const REST: f32 = 0.10;
    pub const HOVER: f32 = 0.24;
    pub const STRONG: f32 = 0.30;
}

/// `id`'s hover layer: a white fill at `fill` and an outline at [`edge::HOVER`], both eased in
/// and out. `bordered`: the element has its own 1 px outline, which the layer's covers.
pub(crate) fn hover_edge(id: &SharedString, r: f32, fill: f32, bordered: bool) -> gpui::AnyElement {
    let inset = if bordered { -1. } else { 0. };
    hover_fade(
        id,
        move || {
            div()
                .absolute()
                .top(px(inset))
                .left(px(inset))
                .right(px(inset))
                .bottom(px(inset))
                .rounded(px(r))
        },
        move |d, t| {
            d.bg(white(fill * t))
                .border_1()
                .border_color(white(edge::HOVER * t))
        },
    )
}

/// A hover layer that hears the pointer itself, for an element with no id, or with hoverable
/// parts in it ([`track_hover`] lights one element at a time, so a row would go dark under its own
/// button): `paint` draws it at 0 (rest) to 1 (hovered), eased in and out on QUICK, at once under
/// reduced motion. Put it first in a `relative` element, so the content is drawn over it. At rest
/// it draws nothing and plays nothing.
fn hover_back(inset: f32, r: f32, paint: impl Fn(Div, f32) -> Div + 'static) -> AnyElement {
    let n = BACKS.get();
    BACKS.set(n + 1);
    // Named by its place in the frame's build: the pages are built in the same order each frame.
    // shortcut: a row hovered as its page is swapped by the keyboard may stay lit until the
    // pointer moves over it; give the callers ids if that shows.
    let id = SharedString::from(format!("hover-back-{n}"));
    let state = BACK_HOVER.with_borrow(|all| all.get(&id).copied());
    let shown = back_phase(state, horizon_theme::QUICK.duration()).map(|(on, at)| {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(std::time::Instant::now);
        let key = ElementId::named_usize(
            format!("{id}-{on}"),
            at.saturating_duration_since(start).as_micros() as usize,
        );
        div()
            .absolute()
            .top(px(inset))
            .left(px(inset))
            .right(px(inset))
            .bottom(px(inset))
            .rounded(px(r))
            .with_animation(
                key,
                crate::motion::eased(horizon_theme::QUICK),
                move |d, t| paint(d, if on { t } else { 1. - t }),
            )
    });
    let key = id.clone();
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_hover(move |hovered, window, _| {
            BACK_HOVER.with_borrow_mut(|all| {
                all.insert(key.clone(), (*hovered, std::time::Instant::now()));
            });
            window.refresh();
        })
        .children(shown)
        .into_any_element()
}

/// What a [`hover_back`] layer plays from its last pointer change (`state`): easing in while on,
/// out for `len` after the pointer left, nothing at rest.
fn back_phase(
    state: Option<(bool, std::time::Instant)>,
    len: std::time::Duration,
) -> Option<(bool, std::time::Instant)> {
    state.filter(|&(on, at)| on || at.elapsed() < len)
}

/// [`hover_back`] as [`hover_edge`]: white at `fill` and the hover outline. `bordered`: the
/// element has its own 1 px outline, which the layer's covers.
pub fn back_edge(r: f32, fill: f32, bordered: bool) -> AnyElement {
    hover_back(if bordered { -1. } else { 0. }, r, move |d, t| {
        d.bg(white(fill * t))
            .border_1()
            .border_color(white(edge::HOVER * t))
    })
}

/// [`hover_back`] in one colour (white at a few %, a dark tint, the danger red), no outline.
pub fn back_tint(r: f32, c: Hsla) -> AnyElement {
    hover_back(0., r, move |d, t| d.bg(c.opacity(c.a * t)))
}

/// `a` towards `b` by `t`, for hover-eased colours.
pub fn towards(a: Hsla, b: Hsla, t: f32) -> Hsla {
    Hsla {
        h: a.h + (b.h - a.h) * t,
        s: a.s + (b.s - a.s) * t,
        l: a.l + (b.l - a.l) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// A lifting card's glow under it, at `t` of its strength (the site's `0 14px 36px -12px`).
fn lift_glow(t: f32) -> Vec<gpui::BoxShadow> {
    glows(vec![gpui::BoxShadow {
        color: color(look().accent).opacity(0.6 * t),
        offset: gpui::point(px(0.), px(14.)),
        blur_radius: px(36.),
        spread_radius: px(-12.),
        inset: false,
    }])
}

/// The accent's lighter end, for the primary button's gradient (the mockup's `--accent2`).
pub fn accent2() -> u32 {
    horizon_theme::glass::mix_oklab(look().accent, 0xffffff, 0.25)
}

/// `d`, a card built by [`card`] with its children in, wrapped to rise [`crate::motion::LIFT`] px
/// on the spring while the pointer is on it: its outline warms from white to the accent and a glow
/// opens under it, as the site's cards do. Pressing sinks it [`crate::motion::PRESS`] px instead.
pub fn lift(id: &SharedString, d: Stateful<Div>) -> AnyElement {
    let rest = white(edge::REST + 0.03);
    let warm = color(look().accent).opacity(0.65);
    crate::hover_move(
        id,
        d.border_1().active(|s| s.top(px(crate::motion::PRESS))),
        move |d, t| {
            d.top(px(-crate::motion::LIFT * t))
                .border_color(towards(rest, warm, t))
                .shadow(lift_glow(t))
        },
    )
}

/// The site's card: white at 3.5 %, a quiet 1 px outline, `r` corners, its hover eased by [`lift`].
/// The look (outline, rise, glow) is [`lift`]'s; a card that should not lift takes this alone.
pub fn card(id: impl Into<SharedString>, r: f32, t: &Theme) -> Stateful<Div> {
    let id = id.into();
    keyboard(
        track_hover(
            div()
                .id(id.clone())
                .relative()
                .rounded(px(r))
                .bg(white(0.035))
                .border_1()
                .border_color(white(edge::REST + 0.03))
                .cursor_pointer(),
            id,
        ),
        t,
    )
}

/// One pass of light across a primary button while the pointer is on it (the site's sheen): a
/// narrow band of white, sliding through on the spring. Put it inside the button, before its
/// label; it is gone again by the time the spring settles. Reduced motion shows nothing.
pub fn sheen(id: &SharedString) -> AnyElement {
    // Out of the flow in every state: an in-flow rest div would sit in the button's gap and
    // widen it, so the button would narrow the moment the pointer arrives.
    let nothing = || div().absolute().into_any_element();
    if crate::motion::still() || !effects().sheen {
        return nothing();
    }
    let hovered = crate::hover_is(id);
    let since = HOVER_SINCE.with_borrow(|times| times.get(id).copied());
    if !hovered {
        return nothing();
    }
    let Some(at) = since else {
        return nothing();
    };
    let band = div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(gpui::relative(0.55))
        // shortcut: GPUI gradients stop at two colours, so the sheen fades in but not out
        // again; a second band mirrored would restore the full pass.
        .bg(gpui::linear_gradient(
            105.,
            gpui::linear_color_stop(gpui::white().opacity(0.), 0.),
            gpui::linear_color_stop(gpui::white().opacity(0.3), 1.),
        ));
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let key =
        gpui::ElementId::named_usize(format!("{id}-sheen"), (at - start).as_micros() as usize);
    // One slow pass of light, as the site plays it (750 ms on a long ease-in-out).
    const SHEEN: horizon_theme::Motion = horizon_theme::Motion {
        name: "sheen",
        ms: 750,
        curve: [0.3, 0.7, 0.2, 1.],
    };
    band.with_animation(key, crate::motion::eased(SHEEN), |d, t| {
        d.left(gpui::relative(-0.6 + 1.65 * t))
    })
    .into_any_element()
}

thread_local! {
    /// When each element's hover began, for one-shot animations like [`sheen`].
    static HOVER_SINCE: RefCell<HashMap<SharedString, std::time::Instant>> =
        RefCell::new(HashMap::new());
}

/// Records hover times for [`sheen`]; called from [`crate::note_hover`].
pub(crate) fn note_hover_since(id: &SharedString, at: std::time::Instant) {
    HOVER_SINCE.with_borrow_mut(|times| {
        times.insert(id.clone(), at);
    });
}

/// The state dot before a row's second line (`Dot`): live green, amber for busy (the mockup
/// pulses it) and for a warning, the accent's soft shade (given) for an update, white at 35 % for
/// nothing yet.
#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Live,
    Busy,
    Warn,
    Accent(u32),
    Off,
}

pub fn dot(tone: Tone) -> Div {
    let c = match tone {
        Tone::Live => live(),
        Tone::Busy | Tone::Warn => warn(),
        Tone::Accent(soft) => color(soft),
        Tone::Off => white(0.35),
    };
    div().flex_none().size(px(7.)).rounded_full().bg(c)
}

/// The progress bar (`Bar`): 4 px, white at 12 %, filled in the accent's soft shade.
pub fn bar(value: f32, accent_soft: u32) -> Div {
    div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .overflow_hidden()
        .bg(white(fill::ACTIVE))
        .child(
            div()
                .h_full()
                .rounded_full()
                .w(gpui::relative(value.clamp(0., 1.)))
                .bg(color(accent_soft)),
        )
}

/// A sidebar row (an instance, a server): `rounded-[12px] p-2`, white at 10 % when it is the one
/// shown, 5 % under the pointer. Its icon and lines go in as children; `pr` leaves room for a
/// button laid over its right end.
pub fn row(id: impl Into<SharedString>, current: bool, pr: f32, t: &Theme) -> Stateful<Div> {
    let id = id.into();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .items_center()
        .gap(px(12.))
        .w_full()
        .p(px(8.))
        .pr(px(pr))
        .rounded(px(radius::ROW))
        .border_1()
        .cursor_pointer();
    // The one shown: tinted with the accent and outlined in it.
    let d = if current {
        d.bg(color(look().accent).opacity(0.16))
            .border_color(color(look().accent_soft).opacity(0.55))
    } else {
        d.border_color(gpui::transparent_black()).child(hover_edge(
            &id,
            radius::ROW,
            fill::ROW_HOVER,
            true,
        ))
    };
    keyboard(track_hover(d, id), t)
}

/// A row's two lines: the name (14 px semibold) and its state after a dot (12.5 px, white at
/// 65 %), both at the page's 1.5 line height.
pub fn row_text(name: impl Into<SharedString>, state: (Tone, String)) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            text(14., FontWeight::SEMIBOLD, ink::FULL)
                .line_height(px(21.))
                .truncate()
                .child(name.into()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .child(dot(state.0))
                .child(
                    text(12.5, FontWeight::NORMAL, ink::MUTED)
                        .line_height(px(18.75))
                        .truncate()
                        .child(state.1),
                ),
        )
}

/// A square button with one icon (`ICON_BUTTON`): white at 65 %, white with a 8 % fill under
/// the pointer, white with 12 % while `active`.
pub fn icon_button(
    id: impl Into<SharedString>,
    name: &'static str,
    size: f32,
    icon_size: f32,
    r: f32,
    active: bool,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let group = SharedString::from(format!("{id}-group"));
    let d = div()
        .id(id.clone())
        .group(group.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(r))
        .cursor_pointer();
    let d = if active {
        d.bg(white(fill::ACTIVE))
            .border_1()
            .border_color(white(edge::STRONG))
    } else {
        d.child(hover_fill(&id, r, 0., fill::BUTTON))
    };
    let glyph = icon(
        name,
        icon_size,
        white(if active { ink::FULL } else { ink::MUTED }),
    )
    .relative()
    .group_hover(group, |s| s.text_color(white(ink::FULL)));
    keyboard(track_hover(d.child(glyph), id), t)
}

/// A top-bar page, as the site's nav: a naked tab — no plate, no pill — its label brightening and
/// a faint fill arriving under the pointer, with the sunrise bar growing under it: a gradient
/// line, half out on hover and all the way out with a glow on the page it names (`open`).
pub fn nav_tab(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    open: bool,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let look = look();
    let bar = div()
        .absolute()
        .bottom(px(1.))
        .h(px(2.))
        .rounded(px(2.))
        .bg(grad(90., color(look.accent_soft), color(look.accent)));
    let bar = if open {
        bar.left(px(12.))
            .right(px(12.))
            .shadow(glows(vec![gpui::BoxShadow {
                color: color(look.accent).opacity(0.9),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(10.),
                spread_radius: px(0.),
                inset: false,
            }]))
            .into_any_element()
    } else {
        // Grows from the middle out to 80 % of the tab, at 70 % strength, on the spring.
        crate::hover_ease(
            &id,
            move || {
                div()
                    .absolute()
                    .bottom(px(1.))
                    .h(px(2.))
                    .rounded(px(2.))
                    .bg(grad(90., color(look.accent_soft), color(look.accent)))
            },
            move |d, t| {
                d.left(gpui::relative(0.5 - 0.4 * t))
                    .right(gpui::relative(0.5 - 0.4 * t))
                    .opacity(0.7 * t)
            },
            crate::motion::SPRING,
        )
    };
    let label: SharedString = label.into();
    keyboard(
        track_hover(
            div()
                .id(id.clone())
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .h(px(36.))
                .px(px(13.))
                .rounded(px(radius::CONTROL))
                .cursor_pointer()
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(14.5))
                .line_height(px(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white(if open { ink::FULL } else { ink::MUTED }))
                .when(!open, |d| {
                    d.hover(|s| s.text_color(white(ink::FULL)))
                        .child(back_tint(radius::CONTROL, white(0.04)))
                })
                .child(div().relative().child(label))
                .child(bar),
            id,
        ),
        t,
    )
}
const MENU_FONT_SIZE: f32 = 14.;
const MENU_ICON_SIZE: f32 = 17.;
const MENU_ROW_GAP: f32 = 12.;
const MENU_ROW_PADDING: f32 = 12.;
const POPOVER_PADDING: f32 = 6.;

/// Width needed to fit the longest translated row, including the icon and both paddings.
pub fn menu_content_width(window: &Window, labels: &[&str]) -> f32 {
    let font = gpui::font(horizon_theme::glass::FONT);
    let widest = labels
        .iter()
        .map(|&label| {
            let run = TextRun {
                len: label.len(),
                font: font.clone(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                SharedString::from(label.to_owned()),
                px(MENU_FONT_SIZE),
                &[run],
                None,
            );
            f32::from(line.width)
        })
        .fold(0., f32::max);
    widest.ceil() + MENU_ICON_SIZE + MENU_ROW_GAP + 2. * (MENU_ROW_PADDING + POPOVER_PADDING)
}

/// A popover's row (`MENU_ROW`): 40 px, 14 px text after a 17 px icon at 60 %, white at 7 %
/// under the pointer. `danger` colours it as the mockup's Delete.
pub fn menu_row(
    id: impl Into<SharedString>,
    name: &'static str,
    label: impl Into<SharedString>,
    danger: bool,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let (fg, icon_color) = if danger {
        let c = self::danger();
        (c, c)
    } else {
        (white(ink::FULL), white(ink::NOTE))
    };
    let hover_color = if danger {
        rgba(horizon_theme::glass::DANGER_HOVER << 8 | 0x1f).into()
    } else {
        white(0.07)
    };
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .items_center()
        .gap(px(MENU_ROW_GAP))
        .h(px(40.))
        .w_full()
        .px(px(MENU_ROW_PADDING))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .child(hover_fade(&id, layer(radius::CONTROL), move |d, a| {
            d.bg(hover_color.opacity(hover_color.a * a))
        }))
        .child(icon(name, MENU_ICON_SIZE, icon_color).relative())
        .child(
            text(MENU_FONT_SIZE, FontWeight::NORMAL, 1.)
                .relative()
                .min_w_0()
                .truncate()
                .text_color(fg)
                .child(label.into()),
        );
    keyboard(track_hover(d, id), t)
}

/// A popover (`POPOVER`): the cards' glass (the mockup's darker `hzl-float` read as a different
/// material beside them), round at 16 with 6 px around its rows, a shadow setting it above the
/// card under it.
pub fn popover(width: f32) -> Div {
    crate::glass::plate(radius::POPOVER)
        .shadow_lg()
        .w(px(width))
        .p(px(POPOVER_PADDING))
        .flex()
        .flex_col()
}

/// What a dialog or the palette lays over the launcher (`bg-[#07050f]/50 backdrop-blur-xl`):
/// GPUI can't blur what it drew, so the scene's blurred copy (a menu's, `hzl-float`) covers the
/// page. The scrim goes over it.
pub fn scrim_backdrop() -> impl IntoElement {
    crate::glass::backdrop(crate::glass::Kind::Float, 0.)
}

/// The scrim over [`scrim_backdrop`]: 50 % in the dark look, 20 % in light, so the dimmed
/// launcher stays day-lit.
pub fn scrim() -> Hsla {
    color(horizon_theme::glass::SCRIM).opacity(if light() { 0.2 } else { 0.5 })
}

/// A dialog's plate, round at `radius` (`hzl-glass` over the scrim): its piece of the scene
/// under the scrim's dimming, then the plates' white. The content goes after it.
pub fn dialog_plate(radius: f32) -> Div {
    let layer = |c: Hsla| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(px(radius))
            .bg(c)
    };
    crate::glass::plate(radius)
        // Light plates are already day-lit: no dimming under them.
        .child(layer(
            color(horizon_theme::glass::SCRIM).opacity(if light() { 0. } else { 0.5 }),
        ))
        .child(layer(white(horizon_theme::glass::TINT)))
}

/// Text on a white button (`SOLID`): `#100a1f`, medium.
pub fn solid(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    h: f32,
    size: f32,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    keyboard(
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(8.))
            // `h` is the least height: a label that wraps in a narrow button (a long translation)
            // grows it instead of spilling out of it.
            .min_h(px(h))
            .py(px(4.))
            .px(px(16.))
            .rounded(px(radius::CONTROL))
            .text_center()
            .bg(paper())
            .border_1()
            .border_color(white(edge::REST))
            .cursor_pointer()
            .hover(|s| {
                s.opacity(0.94).shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.25),
                    offset: gpui::point(px(0.), px(4.)),
                    blur_radius: px(14.),
                    spread_radius: px(0.),
                    inset: false,
                }])
            })
            .active(|s| s.opacity(0.85))
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(size))
            .font_weight(FontWeight::MEDIUM)
            .text_color(on_paper())
            .child(label.into()),
        t,
    )
}

/// A big square button with one icon (`BIG_ICON`): 56 px, white at 8 %, 13 % under the pointer;
/// the glyph white at 80 %, white under the pointer.
pub fn big_icon(id: impl Into<SharedString>, name: &'static str, t: &Theme) -> Stateful<Div> {
    let id = id.into();
    let group = SharedString::from(format!("{id}-group"));
    let glyph = icon(name, 21., white(ink::STRONG))
        .group_hover(group.clone(), |s| s.text_color(white(ink::FULL)));
    keyboard(
        div()
            .id(id)
            .group(group)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(56.))
            .rounded(px(radius::LARGE))
            .bg(white(fill::BUTTON))
            .border_1()
            .border_color(white(edge::REST))
            .active(|s| s.bg(white(0.18)))
            .cursor_pointer()
            .child(back_edge(radius::LARGE, 0.05, true))
            .child(glyph.relative()),
        t,
    )
}

/// A menu row with a note under its label (`MENU_ITEM`): 14 px medium over 12 px at 55 %, white
/// at 7 % under the pointer; at 40 % when `locked`. A mark (a tick) can follow on the right.
pub fn menu_item(
    id: impl Into<SharedString>,
    label: SharedString,
    note: String,
    locked: bool,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .w_full()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(radius::CONTROL))
        .when(!locked, |d| {
            d.cursor_pointer()
                .child(hover_fade(&id, layer(radius::CONTROL), |d, a| {
                    d.bg(white(0.07 * a))
                }))
        })
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    text(14., FontWeight::MEDIUM, ink::FULL)
                        .line_height(px(21.))
                        .child(label),
                )
                .child(
                    text(12., FontWeight::NORMAL, ink::QUIET)
                        .line_height(px(18.))
                        .truncate()
                        .child(note),
                ),
        );
    if locked {
        return d.opacity(0.4);
    }
    keyboard(track_hover(d, id), t)
}

/// A dropdown's choice: its name on one line, 36 tall, the accent's tick on the one `picked`.
pub fn pick_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    picked: bool,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .h(px(36.))
        .w_full()
        .px(px(12.))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .child(hover_fade(&id, layer(radius::CONTROL), |d, a| {
            d.bg(white(0.07 * a))
        }))
        .child(
            text(14., FontWeight::NORMAL, ink::FULL)
                .relative()
                .truncate()
                .child(label.into()),
        )
        .children(picked.then(|| icon("tick-02", 17., color(look().accent_soft)).relative()));
    keyboard(track_hover(d, id), t)
}

/// A quiet button (`QUIET`): white at 8 %, 14 % under the pointer, 14 px medium, `h` tall.
pub fn quiet(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    h: f32,
    t: &Theme,
) -> Stateful<Div> {
    quiet_icon(id, None, label, h, t)
}

/// [`quiet`] with a 16 px icon before its label.
pub fn quiet_icon(
    id: impl Into<SharedString>,
    name: Option<&'static str>,
    label: impl Into<SharedString>,
    h: f32,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(h))
        .px(px(14.))
        .rounded(px(radius::CONTROL))
        .bg(white(fill::BUTTON))
        .border_1()
        .border_color(white(edge::REST))
        .active(|s| s.bg(white(0.18)))
        .cursor_pointer()
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white(ink::FULL))
        .child(hover_edge(&id, radius::CONTROL, 0.06, true))
        .children(name.map(|n| icon(n, 16., white(ink::FULL)).relative()))
        .child(div().relative().child(label.into()));
    keyboard(track_hover(d, id), t)
}

/// A list row (`ROW`): `rounded-[12px] px-3 py-2.5`, white at 6 % under the pointer. `on_hover`
/// hears the pointer come and go too, for rows that show their actions then. `held`: the row
/// keeps its fill while the pointer is on one of its buttons or its menu is open, as CSS's
/// `:hover` stays on a parent.
pub fn list_row(
    id: impl Into<SharedString>,
    held: bool,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let key = id.clone();
    let fill = if held && !hover_is(&id) {
        layer(radius::ROW)()
            .bg(white(fill::ROW_HOVER))
            .border_1()
            .border_color(white(edge::HOVER))
            .into_any_element()
    } else {
        hover_fill(&id, radius::ROW, 0., fill::ROW_HOVER)
    };
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .items_center()
        .gap(px(12.))
        .w_full()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(radius::ROW))
        .child(fill)
        .on_hover(move |hovered, window, cx| {
            note_hover(&key, *hovered, window);
            on_hover(hovered, window, cx);
        });
    keyboard(d, t)
}

/// A list row's two lines: the name (14 px medium) and a note (13 px, white at 55 %).
pub fn list_text(name: impl Into<SharedString>, note: impl Into<SharedString>) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            text(14., FontWeight::MEDIUM, ink::FULL)
                .line_height(px(21.))
                .truncate()
                .child(name.into()),
        )
        .child(
            text(13., FontWeight::NORMAL, ink::QUIET)
                .line_height(px(19.5))
                .truncate()
                .child(note.into()),
        )
}

/// A tab's line while it has nothing to list (`Empty`): 14 px, white at 60 %, `px-3 py-10`.
pub fn empty() -> Div {
    text(14., FontWeight::NORMAL, ink::NOTE)
        .line_height(px(21.))
        .px(px(12.))
        .py(px(40.))
}

/// The line over a tab's list (`px-3 pb-2`): its note at 13 px, white at 60 %, then buttons.
pub fn list_head(note: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .px(px(12.))
        .pb(px(8.))
        .child(
            text(13., FontWeight::NORMAL, ink::NOTE)
                .line_height(px(19.5))
                .min_w_0()
                .truncate()
                .child(note.into()),
        )
}

/// A link in a line of text (`font-medium text-white underline-offset-4 hover:underline`).
pub fn link(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id.into())
        .font_weight(FontWeight::MEDIUM)
        .text_color(white(ink::FULL))
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(label.into())
}

/// A field (`WIDE_FIELD`): 44 px, round at 12, white at 7 %, 11 % with the focus, 14 px. Red at
/// 12 % while what it holds is refused.
pub fn field(focused: bool, invalid: bool) -> Div {
    let bg: Hsla = if invalid {
        rgba(horizon_theme::glass::DANGER_HOVER << 8 | 0x1f).into()
    } else {
        white(if focused {
            fill::FIELD_FOCUS
        } else {
            fill::FIELD
        })
    };
    div()
        .relative()
        .flex()
        .items_center()
        .h(px(44.))
        .px(px(16.))
        .overflow_hidden()
        .rounded(px(radius::ROW))
        .bg(bg)
        .border_1()
        .border_color(if invalid {
            color(horizon_theme::glass::DANGER_HOVER).opacity(0.6)
        } else if focused {
            color(look().accent_soft).opacity(0.7)
        } else {
            white(edge::REST)
        })
        .child(back_edge(radius::ROW, 0., true))
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(14.))
}

/// A switch (`Toggle`): 40×24, round; the accent with an `on_accent` knob on the right while on,
/// white at 14 % with a white knob at 70 % on the left while off.
pub fn toggle(
    id: impl Into<SharedString>,
    on: bool,
    accent: u32,
    on_accent: u32,
    t: &Theme,
) -> Stateful<Div> {
    keyboard(toggle_face(id, on, accent, on_accent), t)
}

/// The switch without a tab stop, for use inside a focusable label.
pub(super) fn toggle_face(
    id: impl Into<SharedString>,
    on: bool,
    accent: u32,
    on_accent: u32,
) -> Stateful<Div> {
    let id = id.into();
    let knob = div()
        .absolute()
        .top(px(4.))
        .size(px(16.))
        .rounded_full()
        .bg(if on { color(on_accent) } else { white(0.7) });
    let knob = crate::motion::knob(&id, on, knob, 4., 16.);
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(40.))
        .h(px(24.))
        .rounded_full()
        .bg(if on {
            color(accent)
        } else {
            white(fill::BUTTON_HOVER)
        })
        .cursor_pointer()
        .child(knob)
}

/// The generic [`button`](crate::button) in the glass look, 36 px: `Primary` is the mockup's
/// `PLAY` (the accent, semibold), `Secondary` its `QUIET` (white at 8 %, 14 % under the pointer),
/// `Ghost` its text button (white at 70 %, white on 6 % under the pointer).
pub fn button(id: SharedString, label: SharedString, kind: Kind, t: &Theme) -> Stateful<Div> {
    let look = look();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(36.))
        .px(px(14.))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .font_family(horizon_theme::glass::FONT)
        .text_size(px(14.))
        .line_height(px(20.))
        .font_weight(FontWeight::MEDIUM);
    let d = match kind {
        Kind::Primary => {
            let id2 = id.clone();
            let accent2 = self::accent2();
            d.overflow_hidden()
                .bg(grad(135., color(look.accent), color(accent2)))
                .border_1()
                .border_color(white(0.16))
                .text_color(color(look.on_accent))
                .font_weight(FontWeight::SEMIBOLD)
                .shadow(glows(vec![
                    // The site's primary: a light line along the top edge, a glow under it.
                    gpui::BoxShadow {
                        color: gpui::white().opacity(0.22),
                        offset: gpui::point(px(0.), px(1.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                        inset: true,
                    },
                    gpui::BoxShadow {
                        color: color(look.accent).opacity(0.7),
                        offset: gpui::point(px(0.), px(6.)),
                        blur_radius: px(22.),
                        spread_radius: px(-8.),
                        inset: false,
                    },
                ]))
                .hover(move |s| {
                    s.border_color(white(0.34)).shadow(glows(vec![
                        gpui::BoxShadow {
                            color: gpui::white().opacity(0.28),
                            offset: gpui::point(px(0.), px(1.)),
                            blur_radius: px(0.),
                            spread_radius: px(0.),
                            inset: true,
                        },
                        gpui::BoxShadow {
                            color: color(look.accent).opacity(0.95),
                            offset: gpui::point(px(0.), px(12.)),
                            blur_radius: px(34.),
                            spread_radius: px(-8.),
                            inset: false,
                        },
                        gpui::BoxShadow {
                            color: color(look.accent).opacity(0.5),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                            inset: false,
                        },
                    ]))
                })
                .active(|s| s.opacity(0.92).shadow(Vec::new()))
                .child(sheen(&id2))
        }
        Kind::Secondary => d
            .bg(white(fill::BUTTON))
            .border_1()
            .border_color(white(edge::REST))
            .text_color(white(ink::FULL))
            .active(|s| s.bg(white(0.18)))
            .child(hover_edge(&id, radius::CONTROL, 0.06, true)),
        Kind::Ghost => d
            .text_color(white(ink::BODY))
            .hover(|s| s.text_color(white(ink::FULL)))
            .active(|s| s.bg(white(0.1)))
            .child(hover_edge(&id, radius::CONTROL, 0.06, false)),
    };
    keyboard(track_hover(d.child(div().relative().child(label)), id), t)
}

/// A pill's resting place (left, width) and the slide it plays: when it began, and from where.
type PillState = ((f32, f32), Option<(std::time::Instant, (f32, f32))>);

thread_local! {
    /// Where each segmented control's options were last drawn, and its pill: the option's place
    /// and width from the control's left edge; where the pill rests, and the slide it is playing
    /// (when it started, and where it came from) while one runs.
    static SEG_BOUNDS: RefCell<HashMap<String, Vec<(f32, f32)>>> = RefCell::new(HashMap::new());
    static SEG_PILL: RefCell<HashMap<String, PillState>> = RefCell::new(HashMap::new());
}

/// The pill's slide between a segmented control's options (the site's, a hair short of the
/// card spring).
const SEG_SLIDE: horizon_theme::Motion = horizon_theme::Motion {
    name: "seg-slide",
    ms: 280,
    curve: [0.22, 0.9, 0.3, 1.12],
};

/// Where a row of options with a sliding pill starts: its options' places are kept from it.
pub type PillOrigin = Rc<Cell<Option<gpui::Point<Pixels>>>>;

/// Records the row's origin (an absolute layer filling the row).
pub fn pill_origin(origin: &PillOrigin) -> impl IntoElement {
    let origin = origin.clone();
    canvas(
        move |bounds, _, _| origin.set(Some(bounds.origin)),
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Records option `i`'s place and width in row `id` (an absolute layer filling the option).
pub fn pill_probe(id: impl Into<String>, i: usize, origin: PillOrigin) -> impl IntoElement {
    let key = id.into();
    canvas(
        move |bounds, _, _| {
            if let Some(o) = origin.get() {
                let left = f32::from(bounds.origin.x - o.x);
                let width = f32::from(bounds.size.width);
                SEG_BOUNDS.with_borrow_mut(|all| {
                    let row = all.entry(key.clone()).or_default();
                    if row.len() <= i {
                        row.resize(i + 1, (0., 0.));
                    }
                    row[i] = (left, width);
                });
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The pill of row `id`, drawn by `base` (absolute, its height set) at option `active`'s
/// recorded place, sliding there on [`SEG_SLIDE`] when it changes. None until the row has been
/// drawn once.
pub fn sliding_pill(id: &str, active: usize, base: impl Fn() -> Div) -> Option<AnyElement> {
    let at = SEG_BOUNDS.with_borrow(|all| {
        all.get(id)
            .and_then(|row| row.get(active).copied())
            .filter(|&(_, w)| w > 0.)
    })?;
    // A new place starts a slide; while one runs the pill keeps being built on its animation
    // (a rebuild that drops it would snap it to the end), and once it is done the pill rests.
    // As `motion::knob` does for a switch.
    let running = SEG_PILL.with_borrow_mut(|pills| {
        let entry = pills.entry(id.to_string()).or_insert((at, None));
        if entry.0 != at {
            entry.1 = Some((std::time::Instant::now(), entry.0));
            entry.0 = at;
        }
        match entry.1 {
            Some((since, from)) if since.elapsed() < SEG_SLIDE.duration() => Some((since, from)),
            _ => {
                entry.1 = None;
                None
            }
        }
    });
    Some(match running {
        None => base().left(px(at.0)).w(px(at.1)).into_any_element(),
        Some((since, from)) => {
            static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let start = *START.get_or_init(std::time::Instant::now);
            let key = ElementId::named_usize(
                format!("{id}-pill"),
                since.saturating_duration_since(start).as_micros() as usize,
            );
            base()
                .with_animation(key, crate::motion::eased(SEG_SLIDE), move |d, p| {
                    d.left(px(from.0 + (at.0 - from.0) * p))
                        .w(px(from.1 + (at.1 - from.1) * p))
                })
                .into_any_element()
        }
    })
}

/// A tab of a [`pill_strip`], `h` tall: one weight open or not (the pill and the ink mark the
/// open one, so no name changes width), a faint fill under the pointer otherwise.
pub fn pill_tab(id: impl Into<SharedString>, on: bool, h: f32, t: &Theme) -> Stateful<Div> {
    let id = id.into();
    keyboard(
        track_hover(
            div()
                .id(id.clone())
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(7.))
                .h(px(h))
                .px(px(12.))
                .rounded(px(radius::CONTROL))
                .cursor_pointer()
                .font_family(horizon_theme::glass::FONT)
                .font_weight(FontWeight::MEDIUM)
                .text_color(white(if on { ink::FULL } else { ink::BODY }))
                .when(!on, |d| {
                    d.child(hover_edge(&id, radius::CONTROL, 0.05, false))
                        .hover(|s| s.text_color(white(ink::FULL)))
                }),
            id,
        ),
        t,
    )
}

/// A row of [`pill_tab`]s (`h` tall, `top` below the row's top edge) whose open one, `active`,
/// sits on a lit pane of glass that slides to the tab chosen. `row` lays the tabs out (its flex
/// and padding); the strip records where each one is drawn.
pub fn pill_strip(
    id: &'static str,
    active: usize,
    h: f32,
    top: f32,
    row: Div,
    tabs: impl IntoIterator<Item = Stateful<Div>>,
) -> Div {
    let origin = PillOrigin::default();
    let row = row.relative().child(pill_origin(&origin)).children(
        tabs.into_iter()
            .enumerate()
            .map(|(i, tab)| tab.child(pill_probe(id, i, origin.clone()))),
    );
    let accent = look().accent;
    let pill = sliding_pill(id, active, move || {
        div()
            .absolute()
            .top(px(top))
            .h(px(h))
            .rounded(px(radius::CONTROL))
            .bg(grad(180., white(0.16), white(0.07)))
            .border_1()
            .border_color(white(edge::STRONG))
            .shadow(glows(vec![
                gpui::BoxShadow {
                    color: gpui::white().opacity(0.18),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.),
                    inset: true,
                },
                gpui::BoxShadow {
                    color: color(accent).opacity(0.45),
                    offset: gpui::point(px(0.), px(6.)),
                    blur_radius: px(18.),
                    spread_radius: px(-8.),
                    inset: false,
                },
            ]))
    });
    // The pill first, so the names are drawn over it.
    div().relative().children(pill).child(row)
}

/// A segmented control in the glass look (the site's): white at 4 % round at 12, 3 px around
/// 32 px options round at 9; the chosen one's place is held by a pill in the accent's tint that
/// slides to it when the choice changes, and the labels stay put.
pub fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let origin = PillOrigin::default();
    let mut seg = div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .gap(px(2.))
        .p(px(3.))
        .rounded(px(radius::LARGE))
        .bg(white(0.04))
        .border_1()
        .border_color(white(edge::REST))
        .child(pill_origin(&origin));
    for (i, &(value, label)) in options.iter().enumerate() {
        let active = current == value;
        let pick = on_pick.clone();
        let origin = origin.clone();
        let key = id.to_string();
        let d = div()
            .id(SharedString::from(format!("{id}-{label}")))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(32.))
            .px(px(12.))
            .rounded(px(radius::CONTROL - 1.))
            .cursor_pointer()
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(13.5))
            .line_height(px(20.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white(if active { ink::FULL } else { ink::MUTED }))
            .when(!active, |d| d.hover(|s| s.text_color(white(ink::FULL))))
            .child(crate::tr(label))
            .child(pill_probe(key, i, origin))
            .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx));
        seg = seg.child(keyboard(d, t));
    }
    let active = options
        .iter()
        .position(|&(value, _)| value == current)
        .unwrap_or(0);
    let pill = sliding_pill(id, active, || {
        div()
            .absolute()
            .top(px(3.))
            .bottom(px(3.))
            .rounded(px(radius::CONTROL - 1.))
            .border_1()
            .border_color(color(look().accent).opacity(0.6))
            .bg(color(look().accent).opacity(0.2))
            .shadow(glows(vec![gpui::BoxShadow {
                color: color(look().accent).opacity(0.8),
                offset: gpui::point(px(0.), px(4.)),
                blur_radius: px(14.),
                spread_radius: px(-6.),
                inset: false,
            }]))
    });
    seg.children(pill)
}

/// A page's head (`PageHead`): its title in Bricolage, 46 px at this width
/// (`clamp(2.25rem, 3.6vw, 3rem)`), its note under it at 15 px on 1.55, white at 70 %, at most
/// 62 characters wide; 40 px above, 28 below.
pub fn page_head(title: impl Into<SharedString>, note: impl Into<SharedString>) -> Div {
    let note: SharedString = note.into();
    div()
        .flex()
        .flex_col()
        .pt(px(28. - crate::glass::CONTENT_INSET))
        .mb(px(20.))
        .child(
            // A compact title: the page's content, not its name, gets the height.
            div()
                .font_family(horizon_theme::glass::DISPLAY_FONT)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(32.))
                .line_height(px(36.))
                .text_color(white(ink::FULL))
                .child(title.into()),
        )
        .when(!note.is_empty(), |d| {
            d.child(
                text(14., FontWeight::NORMAL, ink::BODY)
                    .mt(px(8.))
                    .max_w(px(615.))
                    .line_height(px(23.25))
                    .child(note),
            )
        })
}

/// A small heading in a card (`h3`): 13 px, medium, white at 70 %.
pub fn card_heading(label: impl Into<SharedString>) -> Div {
    text(13., FontWeight::MEDIUM, ink::BODY)
        .line_height(px(19.5))
        .child(label.into())
}

/// A card's note: 13 px on 1.5, white at 55 %.
pub fn card_note(label: impl Into<SharedString>) -> Div {
    text(13., FontWeight::NORMAL, ink::QUIET)
        .line_height(px(19.5))
        .child(label.into())
}

/// A tooltip's view: one line on a solid float, for controls shown by their icon alone.
pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(360.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(radius::CONTROL))
            .bg(rgb(look().base))
            .border_1()
            .border_color(white(0.12))
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(13.))
            .text_color(white(ink::FULL))
            .child(self.0.clone())
    }
}

/// `.tooltip(ui::tip(label))`: the label of an icon-only control, on hover.
pub fn tip(label: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    let label: SharedString = label.into();
    move |_, cx| cx.new(|_| Tip(label.clone())).into()
}

#[cfg(test)]
mod hover_back_tests {
    use super::back_phase;
    use std::time::{Duration, Instant};

    #[test]
    fn plays_while_on_and_briefly_after() {
        let len = Duration::from_millis(150);
        assert_eq!(back_phase(None, len), None);
        let now = Instant::now();
        assert!(back_phase(Some((true, now)), len).is_some());
        assert!(back_phase(Some((false, now)), len).is_some());
        let old = now - Duration::from_secs(1);
        assert!(back_phase(Some((true, old)), len).is_some());
        assert_eq!(back_phase(Some((false, old)), len), None);
    }
}
