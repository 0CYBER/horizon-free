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
                    HomeTab::Mods => (tr("Installed mods"), "puzzle"),
                    HomeTab::Packs => (tr("Resource packs"), "image-01"),
                    HomeTab::Shaders => (tr("Shaders"), "flash"),
                    HomeTab::Worlds => (tr("Worlds"), "earth"),
                    HomeTab::Multiplayer => (tr("Saved servers"), "user-multiple-02"),
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

// MERGE-CHECK: main drew this patch log on Home's Overview; the fork's Home has no patch log,
// so nothing draws it. Kept only because notes_cache.rs (main) builds on it; drop both together
// if the fork's Home stays without one.
#[allow(dead_code)]
/// What the patch log shows: the line under its title, then its rows.
pub(super) struct PatchLog {
    note: String,
    rows: Vec<PatchRow>,
}

/// A section's name (only when there are several; `true` for the first), or one of its lines.
#[allow(dead_code)]
enum PatchRow {
    Section(SharedString, bool),
    Item(SharedString),
}

#[allow(dead_code)]
impl PatchLog {
    /// From the release list, once it has loaded (see [`patch_source`]).
    fn new(releases: Option<&[Release]>, played: Option<&str>) -> PatchLog {
        let (note, sections) = releases
            .and_then(|list| patch_source(list, played))
            .unwrap_or_else(|| (tr("No notes yet").to_string(), Vec::new()));
        let many = sections.len() > 1;
        let mut rows = Vec::new();
        for (i, (section, items)) in sections.into_iter().enumerate() {
            if many {
                rows.push(PatchRow::Section(section.into(), i == 0));
            }
            rows.extend(items.into_iter().map(|item| PatchRow::Item(item.into())));
        }
        PatchLog { note, rows }
    }

    #[cfg(test)]
    pub(super) fn empty() -> PatchLog {
        PatchLog {
            note: String::new(),
            rows: Vec::new(),
        }
    }
}

/// What the patch log shows from `releases` and its line under the title: the release `played`
/// installs as when it lists changes, else the newest Nightly that does ("Nightly 2026-11-01 ·
/// 4aa6854 · Nov 1"); for a Java instance (`played` None) or without one, the newest launcher
/// release ("Launcher 0.2.37 · Nov 2").
#[allow(dead_code)]
fn patch_source(releases: &[Release], played: Option<&str>) -> Option<(String, Sections)> {
    let day = |r: &Release| glass_versions::published(&r.date, "");
    if let Some(played) = played {
        let nightly = releases
            .iter()
            .filter(|r| r.tag == played)
            .chain(releases.iter().filter(|r| r.nightly.is_some()))
            .map(|r| (r, patch_sections(&r.body)))
            .find(|(_, sections)| !sections.is_empty());
        if let Some((r, sections)) = nightly {
            return Some((format!("{} · {}", r.name, day(r)), sections));
        }
    }
    let r = releases
        .iter()
        .find(|r| r.nightly.is_none() && !r.prerelease)?;
    let version = r
        .tag
        .rsplit('@')
        .next()
        .unwrap_or(&r.tag)
        .trim_start_matches('v');
    let note = format!(
        "{} · {}",
        trf("Launcher {version}", &[("version", version)]),
        day(r)
    );
    Some((note, patch_sections(&r.body)))
}

/// An empty tab's one line.
fn empty_tab(line: &str) -> AnyElement {
    ui::empty().child(line.to_string()).into_any_element()
}

/// What a release tells a player. A release body that has a `### For players` section is
/// written for them: only that section's bullets show. Any other body is the maintainers' list
/// of changes: it shows as a one-line summary with the list behind "Technical details".
pub(super) enum PatchNotes {
    ForPlayers(Vec<String>),
    Technical(Sections),
}

pub(super) fn patch_notes(body: &str) -> Option<PatchNotes> {
    let mut sections = patch_sections(body);
    if let Some(i) = sections
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case("For players"))
    {
        return Some(PatchNotes::ForPlayers(sections.swap_remove(i).1));
    }
    (!sections.is_empty()).then_some(PatchNotes::Technical(sections))
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
    fn player_notes_win_over_the_technical_list() {
        let body = "Intro.\n\n### For players\n- Mods load faster (#1)\n- Fixed a crash on the title screen\n\n### Fixed\n- Rollup 3: #4, #5 stages (#6)\n\nThanks to @a.";
        match patch_notes(body) {
            Some(PatchNotes::ForPlayers(items)) => assert_eq!(
                items,
                vec!["Mods load faster", "Fixed a crash on the title screen"]
            ),
            _ => panic!("expected the player section"),
        }
    }

    #[test]
    fn a_body_without_player_notes_keeps_its_list_as_technical() {
        let body = "A long intro paragraph.\n\n### Fixed\n- Sync upstream to 7b42678bcd (#2922)\n- Make CI failures readable (#2993)\n\nThanks to @arismoko.\n\n---\nLegal.";
        match patch_notes(body) {
            Some(PatchNotes::Technical(sections)) => {
                assert_eq!(sections.len(), 1);
                assert_eq!(sections[0].1.len(), 2);
            }
            _ => panic!("expected a technical list"),
        }
        assert!(patch_notes("Only an intro.").is_none());
    }

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
