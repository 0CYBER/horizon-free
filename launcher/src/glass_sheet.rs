//! Instance settings as a dialog centred over the page, drawn as Settings is (the glass look):
//! its icon, "Instance settings", the instance's name and version at the top; Settings' list of
//! sections on the left (General, Installation, Launch, Hooks, Storage) and the open one on the
//! right, under its name and what it is for, its rows in groups; Save and Cancel and the
//! instance's folder along the foot. The page behind is frosted and dimmed, as under Settings.
use super::*;
use crate::glass_instance::section;
use crate::glass_ui::{self as ui, white};
use horizon_launcher::instances::{Channel, Icon, Instance, Kind as InstanceKind, Plays};
use horizon_theme::glass::{ink, radius};
use new_instance::NewKind;
use servers_page::Field;

/// The sheet's sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SheetTab {
    #[default]
    General,
    Installation,
    Launch,
    Hooks,
    Storage,
}

impl SheetTab {
    const ALL: [SheetTab; 5] = [
        SheetTab::General,
        SheetTab::Installation,
        SheetTab::Launch,
        SheetTab::Hooks,
        SheetTab::Storage,
    ];

    /// The sections `instance` has something to set in: the Launch section is for a Java instance
    /// (Horizon's own edition runs without Java).
    fn shown(instance: &Instance) -> impl Iterator<Item = SheetTab> + '_ {
        SheetTab::ALL
            .into_iter()
            .filter(|t| *t != SheetTab::Launch || instance.java.is_some())
    }

    /// Its name, icon and the line under its name.
    fn about(self) -> (&'static str, &'static str, &'static str) {
        match self {
            SheetTab::General => (
                "General",
                "pencil-edit-02",
                "Name, icon and where it shows in the sidebar.",
            ),
            SheetTab::Installation => (
                "Installation",
                "download-04",
                "The game version this instance runs.",
            ),
            SheetTab::Launch => (
                "Launch",
                "play",
                "How the game starts and how much it may use.",
            ),
            SheetTab::Hooks => (
                "Launch commands",
                "computer-terminal-01",
                "Commands that run before and after the game. Most people never need these.",
            ),
            SheetTab::Storage => (
                "Storage",
                "package-01",
                "Space this instance takes, and ways to take it elsewhere.",
            ),
        }
    }

    /// `HORIZON_LAUNCHER_SHEET` (screenshots): the section the sheet opens on.
    pub(super) fn from_env() -> Option<SheetTab> {
        let name = std::env::var("HORIZON_LAUNCHER_SHEET").ok()?;
        SheetTab::ALL
            .into_iter()
            .find(|t| t.about().0.eq_ignore_ascii_case(&name))
    }
}

/// The dialog's largest size: Settings' own.
const WIDTH: f32 = 960.;
const HEIGHT: f32 = 660.;
/// The section list's width.
const CATS: f32 = 200.;

/// A group of rows (the mockup's `grp`): its name over a faint rounded box.
fn group(title: Option<SharedString>, rows: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .children(title.map(|title| {
            ui::text(12.5, FontWeight::SEMIBOLD, ink::QUIET)
                .px(px(4.))
                .pb(px(7.))
                .child(title)
        }))
        .child(
            rows.flex()
                .flex_col()
                .p(px(2.))
                .rounded(px(14.))
                .bg(white(0.03))
                .border_1()
                .border_color(white(0.1)),
        )
}

/// What the dialog would keep, as typed or picked.
struct Draft {
    name: String,
    group: String,
    icon: Option<Icon>,
    /// A picture chosen, or the saved one removed.
    picture: bool,
    /// Horizon's version and builds, for an instance that plays Horizon.
    horizon: Option<(Option<String>, Channel)>,
    plays: Plays,
    /// A Java instance's arguments and window size.
    java: (String, (String, String)),
    /// The hook fields (before, wrapper, after, variables), once the Hooks section was opened.
    hooks: Option<[String; 4]>,
    /// The Linux switches, once one was flipped.
    switches: Option<[bool; 3]>,
    /// A Java instance's maximum memory, once changed; Err while the box holds no size.
    memory: Result<Option<u32>, ()>,
}

impl Draft {
    /// The sections whose fields Save would change in `saved`.
    fn changed(&self, saved: &Instance) -> Vec<SheetTab> {
        let hooks = [
            saved.launch.pre_launch.clone(),
            saved.launch.wrapper.clone(),
            saved.launch.post_exit.clone(),
            horizon_launcher::launch::env_line(&saved.launch.env),
        ];
        let general = self.name != saved.name
            || self.group != saved.group.clone().unwrap_or_default()
            || self.icon != saved.icon
            || self.picture;
        let installation = self.plays != Plays::of(saved)
            || self
                .horizon
                .as_ref()
                .is_some_and(|(pin, channel)| *pin != saved.version || *channel != saved.channel);
        let launch = saved.java.as_ref().is_some_and(|j| {
            self.java
                != (
                    j.jvm_args.trim().to_string(),
                    j.window
                        .map(|(w, h)| (w.to_string(), h.to_string()))
                        .unwrap_or_default(),
                )
                || match self.memory {
                    Ok(memory) => memory.is_some_and(|mb| {
                        mb != j.memory_mb.unwrap_or(instances::DEFAULT_MEMORY_MB)
                    }),
                    // Text that isn't a size is still an edit, kept until it is fixed.
                    Err(()) => true,
                }
        });
        let l = &saved.launch;
        let hooks = self.hooks.as_ref().is_some_and(|typed| *typed != hooks)
            || self
                .switches
                .is_some_and(|s| s != [l.gamemode, l.mangohud, l.dedicated_gpu]);
        [
            (general, SheetTab::General),
            (installation, SheetTab::Installation),
            (launch, SheetTab::Launch),
            (hooks, SheetTab::Hooks),
        ]
        .into_iter()
        .filter_map(|(changed, tab)| changed.then_some(tab))
        .collect()
    }
}

impl Launcher {
    /// Whether the sheet holds edits its Save has not kept.
    pub(super) fn sheet_changed(&self) -> bool {
        !self.sheet_changes().is_empty()
    }

    /// The sections holding edits Save has not kept.
    pub(super) fn sheet_changes(&self) -> Vec<SheetTab> {
        let Some(d) = self.instances.dialog.as_ref() else {
            return Vec::new();
        };
        let Some(saved) = d
            .editing
            .as_ref()
            .and_then(|id| self.instances.list.iter().find(|i| &i.id == id))
        else {
            return Vec::new();
        };
        let draft = Draft {
            name: self.servers.value(Field::InstanceName).trim().to_string(),
            group: self.servers.value(Field::InstanceGroup).to_string(),
            icon: d.icon.clone(),
            picture: match d.picture {
                Some(Some(_)) => true,
                Some(None) => saved.home.join(instances::PICTURE).is_file(),
                None => false,
            },
            horizon: (d.java.kind == NewKind::Horizon).then(|| (d.pin.clone(), d.channel)),
            plays: self.draft_plays(saved).unwrap_or_else(|| Plays::of(saved)),
            java: (
                self.servers.value(Field::JavaArgs).trim().to_string(),
                (
                    self.servers.value(Field::WindowWidth).trim().to_string(),
                    self.servers.value(Field::WindowHeight).trim().to_string(),
                ),
            ),
            hooks: self.hooks_typed(&saved.id),
            switches: self.switches_typed(&saved.id),
            memory: self.sheet_memory(),
        };
        draft.changed(saved)
    }

    /// What `instance` plays, as its rows and the sheet's head name it: "Nightly · 514ecf9", or
    /// "26.3 · Fabric" for a Java one.
    pub(super) fn sheet_version(&self, instance: &Instance) -> String {
        match &instance.java {
            Some(java) => match &java.loader {
                Some(loader) => format!(
                    "{} · {} {}",
                    java.minecraft,
                    loader.kind.name(),
                    loader.version
                ),
                None => java.minecraft.clone(),
            },
            // An instance that keeps no version of its own plays the latest, unless Use held
            // the launcher's version on an older one: then it says which.
            None if instance.version.is_none() && self.version_of(instance).is_some() => match self
                .held_version()
                .filter(|_| instance.channel == Channel::Nightly)
            {
                Some(tag) => self.version_name(tag),
                None => tr("Latest version").to_string(),
            },
            None => self.version_of(instance).map_or_else(
                || tr("No version installed").to_string(),
                |tag| self.version_label(tag),
            ),
        }
    }

    /// The sheet, while the instance dialog edits an instance.
    pub(super) fn instance_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let t = self.theme;
        let d = self.instances.dialog.as_ref()?;
        let (busy, error, kind, discarding) = (d.busy, d.error.clone(), d.java.kind, d.discarding);
        let id = d.editing.clone()?;
        let editing = self.instances.list.iter().find(|i| i.id == id).cloned()?;
        // Launch, Hooks and Storage act on the selected instance (`launch_page`, `export_page`).
        let selected = self.selected_instance().is_some_and(|s| s.id == editing.id);
        // A section this instance doesn't have (a saved choice, or the screenshot variable) opens
        // General.
        let tab = SheetTab::shown(&editing)
            .find(|t| *t == self.sheet_tab)
            .unwrap_or_default();
        let version = self.sheet_version(&editing);

        let close = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.close_instance_dialog();
            cx.notify();
        });
        let what = match kind {
            NewKind::Horizon => tr("Horizon"),
            NewKind::Vanilla => tr("Vanilla"),
            NewKind::Modded => tr("Modded"),
        };
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .pl(px(20.))
            .pr(px(14.))
            .pt(px(14.))
            .pb(px(12.))
            .border_b_1()
            .border_color(white(0.1))
            .child(self.glass_instance_icon(&editing, 36.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(horizon_theme::glass::DISPLAY_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(20.))
                            .line_height(px(26.))
                            .text_color(white(ink::FULL))
                            .child(tr("Instance settings")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .min_w_0()
                            .child(
                                ui::text(13., FontWeight::MEDIUM, ink::BODY)
                                    .truncate()
                                    .child(editing.name.clone()),
                            )
                            .child(
                                ui::text(12., FontWeight::MEDIUM, ink::BODY)
                                    .flex_none()
                                    .px(px(8.))
                                    .py(px(1.))
                                    .rounded_full()
                                    .bg(white(0.07))
                                    .border_1()
                                    .border_color(white(0.12))
                                    .child(format!("{what} · {version}")),
                            ),
                    ),
            )
            .child(
                ui::icon_button(
                    "sheet-close",
                    "cancel-01",
                    36.,
                    18.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .tooltip(ui::tip(tr("Close")))
                .when(!busy, |b| {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.close_instance_dialog();
                        cx.notify();
                    }))
                }),
            );

        // The settings list's entries, on a plate of their own as Settings draws them.
        let changes = self.sheet_changes();
        let cats = crate::glass::plate(radius::PANEL)
            .flex()
            .flex_col()
            .flex_none()
            .w(px(CATS))
            .gap(px(4.))
            .p(px(6.))
            .children(SheetTab::shown(&editing).map(|each| {
                let (label, icon, _) = each.about();
                // A section with edits Save has not kept carries a dot in the accent.
                let unsaved = changes.contains(&each);
                crate::glass_settings::nav_item(
                    format!("sheet-cat-{label}"),
                    icon,
                    tr(label).to_string(),
                    None,
                    each == tab,
                    true,
                    &t,
                )
                .when(unsaved, |d| {
                    d.child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .size(px(7.))
                            .rounded_full()
                            .bg(ui::color(ui::look().accent)),
                    )
                    .tooltip(ui::tip(tr("Not saved yet")))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.sheet_tab = each;
                    cx.notify();
                }))
            }));

        let (label, _, about) = tab.about();
        let mut pane = div()
            .id("sheet-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(16.))
            .pt(px(18.))
            .px(px(20.))
            .pb(px(20.))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .font_family(horizon_theme::glass::DISPLAY_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(19.))
                            .line_height(px(23.))
                            .text_color(white(ink::FULL))
                            .child(tr(label).to_string()),
                    )
                    .child(ui::text(13., FontWeight::NORMAL, ink::NOTE).child(tr(about))),
            );
        let not_selected = || {
            ui::empty().child(tr(
                "Pick this instance on Home to change how it launches and to export it.",
            ))
        };
        match tab {
            SheetTab::General => {
                let name = self
                    .field(Field::InstanceName, tr("My instance"), 300., cx)
                    .w_full();
                let group_field = self
                    .field(Field::InstanceGroup, tr("For example Survival"), 300., cx)
                    .w(px(230.));
                pane = pane
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(14.))
                            .child(self.glass_instance_icon(&editing, 56.))
                            .child(section(tr("Name").into(), None, name).flex_1().min_w_0())
                            .child(section(tr("Group (optional)").into(), None, group_field)),
                    )
                    .child(self.icon_choice(Some(&editing), kind, cx))
                    .child(self.sheet_actions(&editing, cx))
                    .children(self.sheet_danger(&editing, cx));
            }
            SheetTab::Installation => {
                pane = pane.children(self.switch_notice(&editing));
                if kind == NewKind::Horizon {
                    pane = pane.child(self.horizon_versions(cx));
                } else {
                    self.load_versions(cx);
                    pane = pane.child(self.minecraft_versions(cx));
                    if kind == NewKind::Modded {
                        pane = pane.child(self.loader_choice(cx));
                    }
                }
            }
            SheetTab::Launch if selected => {
                pane = pane.children(
                    self.glass_memory_row(cx)
                        .map(|row| group(Some(tr("Memory").into()), div().child(row))),
                );
                if editing.java.is_some() {
                    pane = pane.child(self.sheet_java(cx));
                }
            }
            SheetTab::Hooks if selected => {
                pane = pane.children(self.glass_hooks(cx));
            }
            SheetTab::Storage if selected => {
                pane = pane.child(self.sheet_space(&editing, cx));
                if self.export.is_none() {
                    self.toggle_export();
                }
                // The export panel is its own plate, set beside a list elsewhere: flush here.
                pane = pane.children(self.export_panel(cx).map(|panel| panel.ml(px(0.))));
            }
            SheetTab::Storage => {
                pane = pane
                    .child(self.sheet_space(&editing, cx))
                    .child(not_selected());
            }
            SheetTab::Launch | SheetTab::Hooks => {
                pane = pane.child(not_selected());
            }
        }

        // One foot for every section: the folder on the left, Cancel and Save on the right. Save
        // keeps everything the dialog holds, whichever section shows, and waits for a change.
        let changed = !changes.is_empty();
        let home = editing.home.clone();
        let cancel = ui::quiet("instance-cancel", tr("Cancel"), 44., &t);
        let save = button("instance-save", "Save", Kind::Primary, &t)
            .h(px(44.))
            .px(px(24.));
        let buttons = if busy {
            div()
                .flex()
                .gap(px(8.))
                .opacity(0.5)
                .child(cancel)
                .child(save)
        } else {
            div()
                .flex()
                .gap(px(8.))
                .child(cancel.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.instances.dialog = None;
                    cx.notify();
                })))
                .child(if changed {
                    save.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.save_instance(cx);
                        cx.notify();
                    }))
                } else {
                    save.opacity(0.35).cursor_default()
                })
        };
        let foot = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .px(px(20.))
            .py(px(14.))
            // Why Save failed takes the folder's place.
            .child(match error {
                Some(e) => ui::text(13., FontWeight::NORMAL, ink::FULL)
                    .flex_1()
                    .min_w_0()
                    .text_color(ui::danger())
                    .child(e),
                None => ui::text(12.5, FontWeight::NORMAL, ink::QUIET)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(short(&editing.home)),
            })
            .child(
                ui::quiet_icon(
                    "sheet-foot-folder",
                    Some("folder-01"),
                    tr("Open folder"),
                    44.,
                    &t,
                )
                .on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&home);
                    crate::glass_toast::open(cx, &home);
                }),
            )
            .child(buttons.flex_none());

        // Centred over the page, as Settings is: the same plate, size and frosted dimming.
        let sheet = ui::dialog_plate(radius::MODAL)
            .id("instance-sheet")
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(WIDTH))
            .h_full()
            .max_h(px(HEIGHT))
            .child(head)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(12.))
                    .p(px(12.))
                    .child(cats)
                    .child(pane),
            )
            .child(foot.border_t_1().border_color(white(0.1)))
            .with_animation(
                ElementId::named_usize("sheet-in", 0),
                motion::eased(motion::POP),
                |d, p| d.opacity(p),
            );
        Some(
            div()
                .id("instance-sheet-layer")
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
                        .bg(ui::scrim().opacity(if ui::light() { 0.2 } else { 0.4 }))
                        .on_mouse_down(gpui::MouseButton::Left, close),
                )
                .child(sheet)
                .children(discarding.then(|| self.discard_prompt(cx))),
        )
    }

    /// Over the sheet closed with edits not saved: keep editing, or drop them.
    fn discard_prompt(&mut self, cx: &mut Context<Self>) -> Div {
        // Where the edits are, so the player knows what Discard drops.
        let unsaved = self
            .sheet_changes()
            .into_iter()
            .map(|tab| tr(tab.about().0).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let t = self.theme;
        let answer = |this: &mut Self, discard: bool, cx: &mut Context<Self>| {
            if discard {
                this.instances.dialog = None;
            } else if let Some(d) = &mut this.instances.dialog {
                d.discarding = false;
            }
            cx.notify();
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(ui::scrim().opacity(0.4))
            .occlude()
            .child(
                ui::dialog_plate(radius::MODAL)
                    .id("discard-prompt")
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .w(px(380.))
                    .p(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .font_family(horizon_theme::glass::DISPLAY_FONT)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(18.))
                                    .text_color(white(ink::FULL))
                                    .child(tr("Discard your changes?")),
                            )
                            .child(ui::text(13., FontWeight::NORMAL, ink::NOTE).child(trf(
                                "Not saved yet: {sections}.",
                                &[("sections", &unsaved)],
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(ui::quiet("discard-yes", tr("Discard"), 44., &t).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    answer(this, true, cx)
                                }),
                            ))
                            .child(
                                button("discard-keep", "Keep editing", Kind::Primary, &t)
                                    .h(px(44.))
                                    .px(px(24.))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        answer(this, false, cx)
                                    })),
                            ),
                    ),
            )
    }

    /// General's rows: a copy, a desktop shortcut and the folder.
    fn sheet_actions(&mut self, editing: &Instance, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let home = editing.home.clone();
        let (id, name) = (editing.id.clone(), editing.name.clone());
        let own = matches!(editing.kind, InstanceKind::Default | InstanceKind::Own);
        let mut rows = div();
        if own && editing.java.is_none() {
            let (from, copy_name) = (id.clone(), trf("{name} copy", &[("name", &name)]));
            // The copy is made from the saved instance: edits not saved yet would be left out of
            // it, so they are settled first.
            let unsaved = self.sheet_changed();
            let busy = self.instances.dialog.as_ref().is_some_and(|d| d.busy);
            let copying = self.instances.dialog.as_ref().is_some_and(|d| d.copying);
            let duplicate = button(
                "sheet-duplicate",
                if copying { "Copying…" } else { "Duplicate" },
                Kind::Secondary,
                &t,
            );
            rows = rows.child(ui::setting_row(
                tr("Duplicate"),
                Some(
                    div()
                        .child(match (copying, unsaved) {
                            (true, _) => tr("Making the copy. When it's ready, this window closes and the copy is selected."),
                            (false, true) => tr(
                                "Save your edits first: the copy is made from the saved instance.",
                            ),
                            (false, false) => tr("A copy with the same worlds, options and packs."),
                        })
                        .into_any_element(),
                ),
                Some(
                    match unsaved || busy {
                        true => disabled(duplicate),
                        false => {
                            duplicate.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.duplicate_instance(from.clone(), copy_name.to_string(), cx);
                            }))
                        }
                    }
                    .into_any_element(),
                ),
            ));
        }
        if own {
            let has = self.has_shortcut(&id);
            rows = rows.child(ui::setting_row(
                tr("Desktop shortcut"),
                Some(
                    div()
                        .child(tr("Starts this instance without opening the launcher."))
                        .into_any_element(),
                ),
                Some(
                    button(
                        "sheet-shortcut",
                        if has {
                            "Remove shortcut"
                        } else {
                            "Create shortcut"
                        },
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_shortcut(id.clone(), name.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
                ),
            ));
        }
        rows = rows.child(ui::setting_row(
            tr("Folder"),
            Some(
                div()
                    .font_family(theme::MONO)
                    .text_size(px(12.))
                    .child(short(&editing.home))
                    .into_any_element(),
            ),
            Some(
                button("sheet-folder", "Open folder", Kind::Secondary, &t)
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&home);
                        crate::glass_toast::open(cx, &home);
                    })
                    .into_any_element(),
            ),
        ));
        group(None, rows)
    }

    /// Launch's rows for a Java instance: its own JVM arguments and the game window's size, kept
    /// by the sheet's Save.
    fn sheet_java(&mut self, cx: &mut Context<Self>) -> Div {
        let args = self.field(Field::JavaArgs, tr("For example -XX:+UseZGC"), 560., cx);
        let width = self.field(Field::WindowWidth, "854", 84., cx).w(px(84.));
        let height = self.field(Field::WindowHeight, "480", 84., cx).w(px(84.));
        group(
            Some(tr("Java and the window").into()),
            div()
                .child(ui::setting_row_stacked(
                    tr("Java arguments"),
                    Some(
                        div()
                            .child(tr("Added to this instance only, split on spaces."))
                            .into_any_element(),
                    ),
                    args.into_any_element(),
                ))
                .child(ui::setting_row(
                    tr("Window size"),
                    Some(
                        div()
                            .child(tr("Width and height the game opens at. Empty: 854 by 480."))
                            .into_any_element(),
                    ),
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(width)
                            .child(ui::text(13., FontWeight::NORMAL, ink::DIM).child("×"))
                            .child(height)
                            .into_any_element(),
                    ),
                )),
        )
    }

    /// Storage's first row: the space the instance's folder takes (measured on a thread, once
    /// per opening) and its folder.
    fn sheet_space(&mut self, editing: &Instance, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        if self.sheet_size.as_ref().map(|s| &s.0) != Some(&editing.id) {
            self.sheet_size = Some((editing.id.clone(), None));
            let (id, home) = (editing.id.clone(), editing.home.clone());
            let size = cx.background_spawn(async move {
                horizon_launcher::storage::allocated_bytes(&home).unwrap_or(0)
            });
            cx.spawn(async move |this, cx| {
                let bytes = size.await;
                let _ = this.update(cx, |this, cx| {
                    if this.sheet_size.as_ref().map(|s| &s.0) == Some(&id) {
                        this.sheet_size = Some((id, Some(bytes)));
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        let size = match self.sheet_size.as_ref().and_then(|s| s.1) {
            Some(bytes) => storage::format_bytes(bytes),
            None => tr("Calculating…").to_string(),
        };
        let home = editing.home.clone();
        group(
            None,
            div().child(ui::setting_row(
                tr("Space used"),
                Some(
                    div()
                        .child(trf(
                            "{size}: its worlds, mods, packs and settings",
                            &[("size", &size)],
                        ))
                        .into_any_element(),
                ),
                Some(
                    button("sheet-space-folder", "Open folder", Kind::Secondary, &t)
                        .on_click(move |_, _, cx| {
                            let _ = std::fs::create_dir_all(&home);
                            crate::glass_toast::open(cx, &home);
                        })
                        .into_any_element(),
                ),
            )),
        )
    }

    /// General's danger zone: delete the instance (to the Trash), asked once more. Horizon's own
    /// instance, the game's home, has none.
    fn sheet_danger(&mut self, editing: &Instance, cx: &mut Context<Self>) -> Option<Div> {
        let delete = self.delete_button(editing, cx)?;
        let danger = ui::color(horizon_theme::glass::DANGER_HOVER);
        Some(
            div()
                .flex()
                .flex_col()
                .mt(px(4.))
                .p(px(14.))
                .rounded(px(12.))
                .border_1()
                .border_color(danger.opacity(0.3))
                .bg(danger.opacity(0.05))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    ui::text(14.5, FontWeight::SEMIBOLD, ink::FULL)
                                        .child(tr("Delete this instance")),
                                )
                                .child(
                                    ui::text(13., FontWeight::NORMAL, ink::NOTE).mt(px(2.)).child(
                                        tr("Its mods, settings and worlds go to the Trash. Back up worlds you want to keep first."),
                                    ),
                                ),
                        )
                        .child(delete),
                )
                .children(self.delete_confirmation(editing, cx)),
        )
    }

    /// Copies `from` into a new instance named `name` on a thread, then selects it.
    fn duplicate_instance(&mut self, from: String, name: String, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.instances.dialog {
            d.busy = true;
            d.copying = true;
        }
        let source = from.clone();
        let task = cx.background_spawn(async move {
            let made = instances::create(&name, Some(&from))?;
            instances::edit(&made.id, |m| m.name = Some(name))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(instance) => {
                        crate::glass_toast::say(trf(
                            "Created {name}",
                            &[("name", instance.name.as_str())],
                        ));
                        // The sheet closes onto the copy unless it holds edits made meanwhile
                        // (or is another instance's now): those stay open.
                        let left_alone = this
                            .instances
                            .dialog
                            .as_ref()
                            .is_some_and(|d| d.editing.as_deref() != Some(source.as_str()))
                            || this.sheet_changed();
                        this.reload_instances();
                        match (left_alone, &mut this.instances.dialog) {
                            (true, Some(d)) => {
                                if d.editing.as_deref() == Some(source.as_str()) {
                                    d.busy = false;
                                    d.copying = false;
                                }
                            }
                            _ => {
                                this.instances.dialog = None;
                                this.select_instance(&instance);
                            }
                        }
                    }
                    Err(e) => {
                        if let Some(d) = this
                            .instances
                            .dialog
                            .as_mut()
                            .filter(|d| d.editing.as_deref() == Some(source.as_str()))
                        {
                            d.busy = false;
                            d.copying = false;
                        }
                        crate::glass_toast::say(e)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Instance {
        Instance {
            id: "mine".into(),
            name: "Mine".into(),
            kind: InstanceKind::Own,
            work: Default::default(),
            home: Default::default(),
            icon: None,
            group: None,
            version: None,
            channel: Channel::Stable,
            native_profile: Default::default(),
            played: Default::default(),
            last_played: None,
            launch: Default::default(),
            java: None,
        }
    }

    fn untouched(saved: &Instance) -> Draft {
        Draft {
            name: saved.name.clone(),
            group: String::new(),
            icon: None,
            picture: false,
            horizon: Some((None, Channel::Stable)),
            plays: Plays::Horizon,
            java: (String::new(), (String::new(), String::new())),
            hooks: None,
            switches: None,
            memory: Ok(None),
        }
    }

    #[test]
    fn invalid_window_text_still_counts_as_an_unsaved_edit() {
        let mut saved = saved();
        saved.java = Some(instances::JavaGame::default());
        let mut draft = untouched(&saved);
        draft.plays = Plays::of(&saved);
        assert!(draft.changed(&saved).is_empty());
        for (width, height) in [
            ("bad", ""),
            ("0", "720"),
            ("1280", ""),
            ("4294967296", "720"),
        ] {
            draft.java.1 = (width.into(), height.into());
            assert_eq!(draft.changed(&saved), vec![SheetTab::Launch]);
        }
        saved.java.as_mut().unwrap().window = Some((1280, 720));
        draft.java.1 = ("1280".into(), "720".into());
        assert!(draft.changed(&saved).is_empty());
        draft.java.1 .0 = "oops".into();
        assert_eq!(draft.changed(&saved), vec![SheetTab::Launch]);
        // Memory set in the sheet waits for Save too; set back to what is saved, it is no edit.
        draft.java.1 = ("1280".into(), "720".into());
        draft.memory = Ok(Some(instances::DEFAULT_MEMORY_MB));
        assert!(draft.changed(&saved).is_empty());
        draft.memory = Ok(Some(instances::DEFAULT_MEMORY_MB * 2));
        assert_eq!(draft.changed(&saved), vec![SheetTab::Launch]);
        // Text in the box that isn't a size is an edit too, until it is fixed.
        draft.memory = Err(());
        assert_eq!(draft.changed(&saved), vec![SheetTab::Launch]);
    }

    #[test]
    fn save_waits_for_a_change_in_any_section() {
        let saved = saved();
        assert!(untouched(&saved).changed(&saved).is_empty());
        let general = SheetTab::General;
        let installation = SheetTab::Installation;
        type Edit = (fn(&mut Draft), SheetTab);
        let edits: [Edit; 8] = [
            (|d| d.name = "Renamed".into(), general),
            (|d| d.group = "Friends".into(), general),
            (|d| d.icon = Some(Icon::Colour("#6b4bff".into())), general),
            (|d| d.picture = true, general),
            (
                |d| d.horizon = Some((Some("nightly-2026-11-13-aaaaaaa".into()), Channel::Stable)),
                installation,
            ),
            (|d| d.horizon = Some((None, Channel::Nightly)), installation),
            // The Hooks section was opened and a command typed.
            (
                |d| {
                    d.hooks = Some([
                        "./backup.sh".into(),
                        String::new(),
                        String::new(),
                        String::new(),
                    ])
                },
                SheetTab::Hooks,
            ),
            // A Linux switch flipped waits for Save like a typed command.
            (|d| d.switches = Some([true, false, false]), SheetTab::Hooks),
        ];
        // Each edit marks its own section, and only that one.
        for (edit, section) in edits {
            let mut draft = untouched(&saved);
            edit(&mut draft);
            assert_eq!(draft.changed(&saved), [section]);
        }
        // Hooks opened and left as they were is no edit.
        let mut draft = untouched(&saved);
        draft.hooks = Some(Default::default());
        assert!(draft.changed(&saved).is_empty());
        // A switch flipped and flipped back is no edit either.
        draft.switches = Some([false, false, false]);
        assert!(draft.changed(&saved).is_empty());
    }
}
