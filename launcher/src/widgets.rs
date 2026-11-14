//! The blocky look's building blocks and the hover tracking every look shares.
use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Primary,
    Secondary,
    Ghost,
}

/// The element under the pointer and the one it just left, with when that happened. Hover
/// backgrounds ease in and out from these rather than snapping.
pub(crate) struct HoverState {
    on: Option<(SharedString, Instant)>,
    off: Option<(SharedString, Instant)>,
}

pub(crate) static HOVER: Mutex<HoverState> = Mutex::new(HoverState {
    on: None,
    off: None,
});

pub(crate) fn track_hover(el: Stateful<Div>, id: SharedString) -> Stateful<Div> {
    el.on_hover(move |hovered, window, _| note_hover(&id, *hovered, window))
}

/// Whether `id` is the element [`hover_fade`] shows hovered now.
pub(crate) fn hover_is(id: &SharedString) -> bool {
    HOVER
        .lock()
        .unwrap()
        .on
        .as_ref()
        .is_some_and(|(h, _)| h == id)
}

/// Records that `id` is now hovered or not, for [`hover_fade`]: an element whose own hover
/// listener does more calls it.
pub(crate) fn note_hover(id: &SharedString, hovered: bool, window: &mut Window) {
    let mut state = HOVER.lock().unwrap();
    let is_on = state.on.as_ref().is_some_and(|(h, _)| h == id);
    if hovered && !is_on {
        state.off = state.on.take();
        let at = Instant::now();
        state.on = Some((id.clone(), at));
        glass_ui::note_hover_since(id, at);
    } else if !hovered && is_on {
        state.off = Some((id.clone(), Instant::now()));
        state.on = None;
    }
    window.refresh();
}

/// A layer under an element's content that fades `color` in while it is hovered. `bleed` lets
/// it reach past the element on the left and right, as the hover plate of a list row does.
pub(crate) fn hover_layer(id: &SharedString, color: Hsla, radius: f32, bleed: f32) -> AnyElement {
    let layer = || {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-bleed))
            .right(px(-bleed))
            .rounded(px(radius))
    };
    hover_fade(id, layer, move |d, a| d.bg(color.opacity(color.a * a)))
}

/// The `layer` made while the element `id` (see [`track_hover`]) is hovered, eased in and out:
/// `paint` draws it at a strength from 0 (not shown) to 1. An element not hovered draws nothing,
/// so a long list costs no layers.
pub(crate) fn hover_fade(
    id: &SharedString,
    layer: impl FnOnce() -> Div,
    paint: impl Fn(Div, f32) -> Div + 'static,
) -> AnyElement {
    hover_ease(id, layer, paint, horizon_theme::QUICK)
}

/// [`hover_fade`] on any motion: the spring for the site's moving parts, QUICK for colour.
pub(crate) fn hover_ease(
    id: &SharedString,
    layer: impl FnOnce() -> Div,
    paint: impl Fn(Div, f32) -> Div + 'static,
    on: horizon_theme::Motion,
) -> AnyElement {
    let state = HOVER.lock().unwrap();
    // Each transition gets its own animation id, from when it started, so it plays once.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let key = |dir: &str, at: Instant| {
        ElementId::named_usize(format!("{id}-{dir}"), (at - start).as_micros() as usize)
    };
    let ease = move || motion::eased(on);
    if let Some((_, at)) = state.on.as_ref().filter(|(h, _)| h == id) {
        return layer()
            .with_animation(key("in", *at), ease(), paint)
            .into_any_element();
    }
    if let Some((_, at)) = state
        .off
        .as_ref()
        .filter(|(h, at)| h == id && at.elapsed() < on.duration())
    {
        return layer()
            .with_animation(key("out", *at), ease(), move |d, t| paint(d, 1. - t))
            .into_any_element();
    }
    // Not hovered: nothing to draw (out of the flow, so it takes no gap either).
    div().absolute().into_any_element()
}

/// The element `el` itself — not a layer inside it — eased between its rest and hovered looks on
/// the spring, as the site's cards lift: `paint` draws it at a strength from 0 (rest) to 1
/// (hovered), and the change plays out in reverse when the pointer leaves. `el` is made
/// `relative()`, so a `top` offset moves it without moving its neighbours.
pub(crate) fn hover_move(
    id: &SharedString,
    el: Stateful<Div>,
    paint: impl Fn(Stateful<Div>, f32) -> Stateful<Div> + 'static,
) -> AnyElement {
    let state = HOVER.lock().unwrap();
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let key = |dir: &str, at: Instant| {
        ElementId::named_usize(
            format!("{id}-move-{dir}"),
            (at - start).as_micros() as usize,
        )
    };
    let spring = motion::eased(motion::SPRING);
    if let Some((_, at)) = state.on.as_ref().filter(|(h, _)| h == id) {
        return el
            .relative()
            .with_animation(key("in", *at), spring, paint)
            .into_any_element();
    }
    if let Some((_, at)) = state
        .off
        .as_ref()
        .filter(|(h, at)| h == id && at.elapsed() < motion::SPRING.duration())
    {
        return el
            .relative()
            .with_animation(key("out", *at), spring, move |d, t| paint(d, 1. - t))
            .into_any_element();
    }
    el.into_any_element()
}

/// Text set in one step of the type scale: headings (heading, title, display) in the pixel face.
pub(crate) fn text(step: theme::Step) -> Div {
    let d = div().text_size(px(step.px)).line_height(px(step.line_px()));
    let d = if step.mono {
        d.font_family(theme::MONO)
    } else if step.px >= theme::HEADING.px {
        d.font_family(theme::PIXEL)
    } else {
        d
    };
    if step.weight >= 500 {
        d.font_weight(FontWeight::MEDIUM)
    } else {
        d
    }
}

/// A release's tag as a title: mono, so its digits read at a glance.
pub(crate) fn version_tag(tag: &str) -> Div {
    div()
        .font_family(theme::MONO)
        .text_size(px(22.))
        .line_height(px(28.))
        .font_weight(FontWeight::MEDIUM)
        .child(
            horizon_launcher::nightly::label(tag)
                .or_else(|| build_import::label(tag))
                .unwrap_or_else(|| tag.trim_start_matches('v').to_string()),
        )
}

/// Horizon's mark: two stems, their reflection below, and the horizon across in the accent.
pub(crate) fn mark(size: f32, t: &Theme) -> Div {
    let u = size / 16.;
    let stem = |x: f32, y: f32, color: Hsla| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(2. * u))
            .h(px(4. * u))
            .bg(color)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(stem(4., 3., t.ink))
        .child(stem(10., 3., t.ink))
        .child(stem(4., 8., t.ink.opacity(0.36)))
        .child(stem(10., 8., t.ink.opacity(0.36)))
        .child(
            div()
                .absolute()
                .left(px(u))
                .top(px(7. * u))
                .w(px(14. * u))
                .h(px(u))
                .bg(t.accent),
        )
}

/// The last thing the player did was press Tab: focused controls show a ring (a click focuses them
/// too, without one).
pub(crate) static KEYBOARD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Makes a control reachable with Tab, pressed with Enter or Space, with a ring when the keyboard
/// put it in focus.
pub(crate) fn keyboard(el: Stateful<Div>, t: &Theme) -> Stateful<Div> {
    let (ring, gap) = (t.accent, t.page);
    // A ring with a gap of page colour, so it shows around filled buttons too. GPUI draws nothing
    // for a shadow without blur: half a pixel keeps it crisp.
    let shadow = |color, spread: f32| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.5),
        spread_radius: px(spread),
        inset: false,
    };
    el.tab_index(0).focus(move |s| {
        if KEYBOARD.load(Ordering::Relaxed) {
            s.shadow(vec![shadow(ring, 4.), shadow(gap, 1.5)])
        } else {
            s
        }
    })
}

/// A button: a block with a light bevel on top and a darker lip below, its label in the pixel
/// face. Primary (the one main action) is the violet face; Secondary a raised block; Ghost only
/// its label until the pointer is over it.
pub(crate) fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label: SharedString = label.into();
    let label = SharedString::from(tr(&label).to_string());
    if t.glass {
        return glass_ui::button(id, label, kind, t);
    }
    let t = *t;
    let hover = match kind {
        Kind::Primary => t.on_face.opacity(0.10),
        Kind::Secondary => t.bevel.opacity(if t.glass { 0.10 } else { 0.35 }),
        Kind::Ghost => t.raised,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(14.))
        .font_family(theme::PIXEL)
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, 0., 0.));
    let base = match kind {
        Kind::Primary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.face_bevel, t.face_lip, theme::LIP))
            .bg(t.face)
            .text_color(t.on_face),
        Kind::Secondary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.button_bevel, t.lip, theme::LIP))
            .bg(t.button)
            .text_color(t.ink),
        Kind::Ghost => base.text_color(t.muted).hover(move |s| s.text_color(t.ink)),
    };
    // The label sits on the block's face, above its lip; pressing pushes it down.
    let base = base.child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(if kind == Kind::Ghost { 0. } else { 2. }))
            .child(label),
    );
    keyboard(track_hover(base, id), &t).active(|s| s.opacity(theme::PRESSED).top(px(1.)))
}

/// Copies to the clipboard, or into `HORIZON_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`horizon_launcher::desktop_seams`).
pub(crate) fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    match item.text() {
        Some(text) => glass_toast::say(trf("Copied {text}", &[("text", &text)])),
        None => glass_toast::say(tr("Copied the picture")),
    }
    if let Some(file) = horizon_launcher::desktop_seams::clipboard_file() {
        let bytes = match item.entries().first() {
            Some(gpui::ClipboardEntry::Image(image)) => image.bytes.clone(),
            _ => item.text().unwrap_or_default().into_bytes(),
        };
        if let Err(e) = std::fs::write(&file, bytes) {
            eprintln!("{}: {e}", file.display());
        }
        return;
    }
    cx.write_to_clipboard(item.clone());
    #[cfg(target_os = "linux")]
    if let Some(text) = item.text() {
        sync_system_clipboard(text);
    }
}

/// GPUI's Wayland clipboard sends the last key press's serial, which a compositor ignores
/// after a mouse click: hand the text to the desktop's own tool too, the first one present
/// for this session (`wl-copy` on Wayland, `xclip` or `xsel` on X11).
#[cfg(target_os = "linux")]
pub(crate) fn sync_system_clipboard(text: String) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    const TOOLS: [(&str, &str, &[&str]); 3] = [
        ("WAYLAND_DISPLAY", "wl-copy", &[]),
        ("DISPLAY", "xclip", &["-selection", "clipboard"]),
        ("DISPLAY", "xsel", &["--clipboard", "--input"]),
    ];
    if text.is_empty() {
        return;
    }
    // Off the UI thread: a tool can take its time.
    std::thread::spawn(move || {
        for (session, tool, args) in TOOLS {
            if std::env::var_os(session).is_none() {
                continue;
            }
            let Ok(mut child) = Command::new(tool)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().is_ok_and(|status| status.success()) {
                return;
            }
        }
    });
}

/// An on/off switch: a square knob sliding in a sunk track that turns violet when on.
pub(crate) fn switch(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    if t.glass {
        let look = glass_ui::look();
        return glass_ui::toggle(id, on, look.accent, look.on_accent, t);
    }
    let t = *t;
    let id: SharedString = id.into();
    let knob = if on { t.on_face } else { t.muted };
    keyboard(div().id(id.clone()), &t)
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(24.))
        .border_2()
        .border_color(t.outline)
        .cursor_pointer()
        .bg(if on { t.face } else { t.well })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .bg(knob)
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(if on { t.face_lip } else { t.lip }),
                )
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 20. * p } else { 22. - 20. * p })),
                ),
        )
}

/// A section's title inside a page: a pixel icon in the accent and the name in the pixel face.
pub(crate) fn section_title(label: &'static str, icon: &'static str, t: &Theme) -> Div {
    if glass_ui::flat() {
        return glass_ui::card_heading(tr(label)).px(px(16.)).pt(px(12.));
    }
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(blocks::icon(icon, 20., t.accent))
        .child(
            blocks::pixel(18., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(label)),
        )
}

/// A segmented control: a sunk track, the chosen segment a raised block.
pub(crate) fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    if t.glass {
        return glass_ui::segments(id, options, current, t, on_pick);
    }
    let t = *t;
    div()
        .id(id)
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .border_2()
        .border_color(t.outline)
        .bg(t.well)
        .children(options.iter().map(|&(value, label)| {
            let active = current == value;
            let pick = on_pick.clone();
            keyboard(div().id(SharedString::from(format!("{id}-{label}"))), &t)
                .relative()
                .flex()
                .items_center()
                .h(px(theme::SEGMENT))
                .px(px(14.))
                .pb(px(2.))
                .font_family(theme::PIXEL)
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.button).text_color(t.ink).children(blocks::edges(
                        t.button_bevel,
                        t.lip,
                        3.,
                    ))
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.text_color(t.ink))
                })
                .child(div().relative().child(tr(label)))
                .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx))
        }))
}

/// What the Play button does for the selected version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayPress {
    Play,
    /// Not prepared, for an account that owns the game: prepare it (from the player's install,
    /// else downloaded from Mojang for that account), then play, as the official launcher does.
    PrepareThenPlay,
    /// Not prepared and no owner (signing in comes first), or busy preparing or playing.
    Nothing,
}

/// Account state uses the same semantic colors as the launcher's other notices. Keep a
/// distinct icon and the full label so color is never the only way to recognize a state.
pub(crate) fn account_status_line(
    status: Option<&account::session::Status>,
    progress: Option<&str>,
    t: &Theme,
) -> Div {
    use account::session::State;
    let (icon, color, label) = if let Some(progress) = progress {
        ("clock", t.muted, progress)
    } else {
        let (icon, color) = match status.map(|s| s.state) {
            Some(State::Ready) => ("check", t.success),
            Some(State::NeedsRefresh | State::Retry) => ("reload", t.warn),
            Some(State::SignInRequired) => ("user", t.danger),
            Some(State::OwnershipRequired) => ("info-box", t.danger),
            None => ("info-box", t.muted),
        };
        (
            icon,
            color,
            status.map_or("Sign-in not checked", |s| s.label()),
        )
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_color(color)
        .child(blocks::icon(icon, 14., color))
        .child(tr(label).to_owned())
}

/// Polling must handle each sign-in transition once. Replaying a completed sign-in
/// restarts every account's status worker and leaves the UI perpetually checking.
pub(crate) fn sign_in_change(
    shown: &mut Option<SignInState>,
    next: SignInState,
) -> Option<SignInState> {
    if shown.as_ref() == Some(&next) {
        return None;
    }
    *shown = Some(next.clone());
    Some(next)
}

/// `owner`: whether an account here owns the game, by the keychain (`Launcher::owner`).
pub(crate) fn play_press(prepared: bool, owner: Option<bool>, busy: bool) -> PlayPress {
    match (prepared, owner) {
        _ if busy => PlayPress::Nothing,
        (true, _) => PlayPress::Play,
        (false, Some(true)) => PlayPress::PrepareThenPlay,
        (false, _) => PlayPress::Nothing,
    }
}

/// What Play's button says for `press`, so its label is what a click does: "Prepare & play"
/// when it prepares first, "Sign in to play" when no account here owns the game (it does
/// nothing then), "Play" otherwise (dimmed while the keychain is still being read).
pub(crate) fn play_label(press: PlayPress, owner: Option<bool>) -> &'static str {
    match (press, owner) {
        (PlayPress::PrepareThenPlay, _) => "Prepare & play",
        (PlayPress::Nothing, Some(false)) => "Sign in to play",
        _ => "Play",
    }
}

/// Whether a shortcut that starts the game at once (a server's Join, a world's Play, a friend's
/// code, a pack's Play) starts it now: only when Play would play. Never while a preparation runs:
/// the game would open on half-written files.
pub(crate) fn starts_now(prepared: bool, preparing: bool, running: bool) -> bool {
    play_press(prepared, None, preparing || running) == PlayPress::Play
}

/// Whether `tag` can start being deleted now: it is installed, no install is writing it, and it
/// is not already going. Another version deleting at the same time does not stop it.
pub(crate) fn removable(
    installed: &[String],
    installing: Option<&str>,
    removing: &[String],
    tag: &str,
) -> bool {
    installing != Some(tag)
        && installed.iter().any(|t| t == tag)
        && !removing.iter().any(|t| t == tag)
}

/// The selected version once one has been removed: the saved choice if it is still installed,
/// else the newest left.
pub(crate) fn selection_after_removal(
    installed: &[String],
    saved: Option<String>,
) -> Option<String> {
    saved
        .filter(|s| installed.contains(s))
        .or_else(|| installed.first().cloned())
}

/// The row menu a press on a sidebar row leaves open: that row's, or none when it was already
/// that row's (the ⋮ and a right click on the row toggle the same way).
pub(crate) fn row_menu_after_press(
    open: Option<(String, bool)>,
    id: &str,
) -> Option<(String, bool)> {
    if open.as_ref().is_some_and(|(current, _)| current == id) {
        return None;
    }
    Some((id.to_owned(), false))
}

pub(crate) fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default().tab_stop(false)
}

/// A page's title in the pixel face and one line on what it is for.
pub(crate) fn page_header(title: &str, description: &str, t: &Theme) -> Div {
    if t.glass {
        return glass_ui::page_head(tr(title), tr(description));
    }
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            blocks::pixel(36., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(title).to_string()),
        )
        .child(
            text(theme::UI)
                .max_w(px(520.))
                .text_color(t.muted)
                .child(tr(description).to_string()),
        )
}

/// One row of a list, as its own panel: a name in the pixel face with an optional detail under
/// it, then a value or a control on the right. Rows were numbered in the quiet look; `_n` keeps
/// the call sites as they are.
/// How an available update is named: a Nightly by its title ("Nightly 2026-10-28 · 0c02067"), a
/// release by its tag.
pub(crate) fn update_name(release: &Release) -> &str {
    match release.nightly {
        Some(_) => &release.name,
        None => &release.tag,
    }
}

pub(crate) fn index_row(
    _n: usize,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    if glass_ui::flat() {
        return glass_ui::setting_row(name, detail, right);
    }
    slot_row(None, name, detail, right, t)
}

/// A row with an icon slot (`blocks::slot`) on its left: a project in the Mods page's lists.
pub(crate) fn slot_row(
    slot: Option<Div>,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    if glass_ui::flat() {
        return div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(16.))
            .py(px(12.))
            .children(slot.map(|s| s.rounded(px(8.)).overflow_hidden()))
            .child(
                glass_ui::setting_row(name, detail, right)
                    .p_0()
                    .border_0()
                    .bg(gpui::transparent_black())
                    .flex_1(),
            );
    }
    blocks::panel(t)
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(theme::ROW))
        .mb(px(8.))
        .px(px(16.))
        .py(px(12.))
        .children(slot)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    blocks::pixel(16., FontWeight::MEDIUM)
                        .text_color(t.ink)
                        .child(name),
                )
                .children(detail.map(|d| {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(d)
                        .into_any_element()
                })),
        )
        .children(right)
}

/// Microsoft account actions go below the profile in the fixed-width glass card.
pub(crate) fn account_row(
    slot: Option<Div>,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    actions: Div,
    t: &Theme,
) -> Div {
    if glass_ui::flat() {
        return div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(slot_row(slot, name, detail, None, t))
            .child(actions.flex_wrap().px(px(16.)).pb(px(12.)));
    }
    slot_row(slot, name, detail, Some(actions.into_any_element()), t)
}

/// A plate that stands apart from the page: notices, the console, the agents note.
pub(crate) fn plate(t: &Theme) -> Div {
    blocks::panel(t)
}

pub(crate) fn tab_action(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    shell::action(id, shell::Action::Raised, t).child(shell::label(tr(&label).to_string()))
}

/// Floating text with no background
pub(crate) fn quiet_note(text_: impl Into<SharedString>, t: &Theme) -> Div {
    plate(t)
        .px(px(12.))
        .py(px(8.))
        .child(text(theme::CAPTION).text_color(t.muted).child(text_.into()))
}

/// Something went wrong: a dot in the danger colour, then what happened.
pub(crate) fn notice(text_: String, t: &Theme) -> Div {
    plate(t)
        .flex()
        .gap(px(12.))
        .p(px(16.))
        .text_size(px(theme::UI.px))
        .line_height(px(21.))
        .text_color(t.ink)
        .border_l(px(6.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .bottom(px(-2.))
                .left(px(-6.))
                .w(px(6.))
                .bg(t.danger),
        )
        .child(blocks::icon("info-box", 20., t.danger))
        .child(div().flex_1().min_w_0().child(text_))
}

/// An empty state: a short sentence, and the one thing to do about it.
pub(crate) fn centered(text_: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(16.))
        .py(px(48.))
        .child(text(theme::UI).text_color(t.muted).child(text_.to_string()))
}

/// `1234` as `1,234`.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A mono line in upper case: the status above a title, the footer's links.
pub(crate) fn caps(s: impl AsRef<str>) -> Div {
    // The glass look writes these as the mockup's small headings (`h3`): 13 px, medium.
    if glass_ui::on() {
        return glass_ui::text(13., FontWeight::MEDIUM, horizon_theme::glass::ink::BODY)
            .line_height(px(19.5))
            .child(s.as_ref().to_string());
    }
    text(theme::MICRO).child(s.as_ref().to_uppercase())
}

pub(crate) fn short(path: &std::path::Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

pub(crate) fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

/// A thin bar. A known fraction eases from its previous value; an unknown one shuttles slowly.
/// Progress: a plate-coloured track filling with the accent; work of unknown length shuttles.
pub(crate) fn progress_bar(fraction: Option<(f32, f32)>, t: &Theme) -> Div {
    // Night: a thin accent line on a plate's colour, square like the art. Glass: the mockup's
    // `Bar`, 4 px and round, in the accent's soft shade on white at 12 %.
    let n = t.night;
    let bar = div()
        .absolute()
        .top_0()
        .bottom_0()
        .bg(plate::alpha(n.accent, 1.))
        .when(t.glass, |d| {
            d.rounded_full()
                .bg(glass_ui::color(glass_ui::look().accent_soft))
        });
    let track = div()
        .relative()
        .flex_1()
        .h(px(6.))
        .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
        .overflow_hidden()
        .when(t.glass, |d| {
            d.h(px(4.))
                .rounded_full()
                .bg(glass_ui::white(horizon_theme::glass::fill::ACTIVE))
        });
    match fraction {
        Some((from, to)) => {
            let (from, to) = (from.clamp(0.02, 1.), to.clamp(0.02, 1.));
            track.child(bar.left_0().with_animation(
                ElementId::named_usize("bar", (to * 1000.) as usize),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                move |d, p| d.w(relative(from + (to - from) * p)),
            ))
        }
        // Reduced motion holds the shuttle still where it can be seen (GPUI freezes a repeating
        // animation at its start, off the track).
        None if motion::still() => track.child(bar.left(relative(0.35)).w(relative(0.3))),
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "shuttle",
                Animation::new(Duration::from_millis(1600))
                    .repeat()
                    .with_easing(gpui::ease_in_out),
                |d, p| d.left(relative(-0.3 + 1.3 * p)),
            ),
        ),
    }
}
