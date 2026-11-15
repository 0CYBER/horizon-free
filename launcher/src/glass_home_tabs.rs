//! Home's tabbed panel in the glass look (the mockup's `TabbedPanel` and `Overview`,
//! `launcher-views.tsx`): one glass plate whose top-left corner is the tab strip, and under it
//! the open tab. Overview shows the instance's last screenshots beside the newest patch log,
//! then what Play still needs (the launcher's rows). Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use home_tabs::HomeTab;
use horizon_launcher::packs::Kind as PackKind;
use horizon_theme::glass::ink;

/// The tabbed panel's own body is at least this tall (`min-h-[340px]`), and so is the patch log.
const BODY: f32 = 340.;

impl Launcher {
    /// The tabbed panel: the strip, then the open tab.
    pub(super) fn glass_home_tabs(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let tabs: Vec<_> = self
            .home_tab_list(cx)
            .into_iter()
            .filter(|(tab, _)| *tab != HomeTab::Overview)
            .collect();
        let open = tabs
            .iter()
            .map(|(tab, _)| *tab)
            .find(|tab| *tab == self.home_tab)
            .unwrap_or(HomeTab::Mods);
        // Icons beside the names only where every tab still has the room for both.
        let icons = self.window_width >= crate::glass_home::SIDE_BY_SIDE + 160.;
        let active = tabs.iter().position(|(tab, _)| *tab == open).unwrap_or(0);
        let items: Vec<_> = tabs
            .into_iter()
            .map(|(tab, count)| {
                let (name, icon) = match tab {
                    HomeTab::Overview => (tr("Overview"), "dashboard-square-01"),
                    HomeTab::Mods => (tr("Mods"), "puzzle"),
                    HomeTab::Packs => (tr("Resource packs"), "image-01"),
                    HomeTab::Shaders => (tr("Shaders"), "flash"),
                    HomeTab::Worlds => (tr("Worlds"), "earth"),
                    HomeTab::Multiplayer => (tr("Multiplayer"), "user-multiple-02"),
                    HomeTab::Screenshots => (tr("Screenshots"), "image-01"),
                    HomeTab::Console => (tr("Log"), "computer-terminal-01"),
                };
                let on = tab == open;
                let ink_of = white(if on { ink::FULL } else { ink::BODY });
                ui::pill_tab(format!("home-tab-{tab:?}"), on, 38., &t)
                    // Each grows from its own width, so long names keep their room.
                    .flex_auto()
                    .min_w_0()
                    .px(px(8.))
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .children(icons.then(|| ui::icon(icon, 16., ink_of).relative()))
                    .child(
                        div()
                            .relative()
                            .min_w_0()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(name),
                    )
                    .children(count.filter(|n| *n > 0).map(|n| {
                        div()
                            .relative()
                            .flex_none()
                            .text_color(white(ink::DIM))
                            .child(n.to_string())
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.home_tab = tab;
                        cx.notify();
                    }))
            })
            .collect();
        let row = div()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .gap(px(2.))
            .px(px(8.))
            .pt(px(8.))
            .pb(px(4.))
            .child(crate::glass::measure(self.home_strip.clone()));
        let strip = ui::pill_strip("home-tabs", active, 38., 8., row, items).w_full();
        let body: Option<AnyElement> = match open {
            // The glass look has no Overview: its pieces live in their own tabs, Versions has the
            // patch notes, and what Play needs sits above the tabs.
            HomeTab::Overview => None,
            HomeTab::Mods => Some(self.glass_instance_mods(cx).into_any_element()),
            HomeTab::Worlds => self.glass_worlds(cx).map(IntoElement::into_any_element),
            HomeTab::Multiplayer => self
                .glass_multiplayer(cx)
                .map(IntoElement::into_any_element),
            HomeTab::Packs => self
                .glass_packs(PackKind::Resource, cx)
                .map(IntoElement::into_any_element),
            HomeTab::Shaders => self
                .glass_packs(PackKind::Shader, cx)
                .map(IntoElement::into_any_element),
            HomeTab::Screenshots => Some(
                self.glass_screenshots(cx)
                    .map(IntoElement::into_any_element)
                    .unwrap_or_else(|| empty_tab(tr("No screenshots yet. Press F2 in the game."))),
            ),
            HomeTab::Console => self.glass_console(cx).map(IntoElement::into_any_element),
        };
        div()
            .relative()
            // `mt-12` under the head, of which Home's own gap gives 28.
            .mt(px(20.))
            .child(crate::glass::tabbed(self.home_strip.clone()))
            .child(
                div()
                    .relative()
                    .flex()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .child(strip),
            )
            .child(
                div().relative().p(px(8.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .min_h(px(BODY))
                        .px(px(4.))
                        .pt(px(8.))
                        .pb(px(4.))
                        .children(body),
                ),
            )
    }
}

/// A release's changes: each section's name and its lines.
pub(super) type Sections = Vec<(String, Vec<String>)>;

/// An empty tab's one line.
fn empty_tab(line: &str) -> AnyElement {
    ui::empty().child(line.to_string()).into_any_element()
}

/// A release's changes by section ("### New", "### Fixed"), each line without its pull request
/// numbers; the contributors' thanks and what follows the notes are left out.
pub(super) fn patch_sections(body: &str) -> Sections {
    let mut out: Sections = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("### ") {
            out.push((name.trim().to_string(), Vec::new()));
        } else if line == "---" || line.starts_with("Your launcher updates itself") {
            break;
        } else if let (Some(item), Some((section, items))) =
            (line.strip_prefix("- "), out.last_mut())
        {
            if section.as_str() != "Contributors" {
                items.push(strip_refs(item));
            }
        }
    }
    out.retain(|(name, items)| name != "Contributors" && !items.is_empty());
    out
}

/// "Fix the picker (#6389, fixes #6342)" → "Fix the picker".
fn strip_refs(item: &str) -> String {
    let mut s = item.trim();
    while let Some(open) = s.rfind(" (") {
        let tail = &s[open..];
        if tail.ends_with(')') && tail.contains('#') {
            s = s[..open].trim_end();
        } else {
            break;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_body_becomes_its_sections_without_refs_or_thanks() {
        let body = "### New\n- Native mods: export a headless engine (#6462)\n\n### Fixed\n- Play with friends: any port (#6363, fixes #6342)\n- Keep (this) aside\n\n### Contributors\nThanks to @a.\n\nYour launcher updates itself. New here?\n\n---\nNot an official Minecraft product.";
        assert_eq!(
            patch_sections(body),
            vec![
                (
                    "New".to_string(),
                    vec!["Native mods: export a headless engine".to_string()]
                ),
                (
                    "Fixed".to_string(),
                    vec![
                        "Play with friends: any port".to_string(),
                        "Keep (this) aside".to_string()
                    ]
                ),
            ]
        );
    }
}
