//! Launch… on Home's Instance row: what runs around the selected instance's game
//! (`horizon_launcher::launch`): a command before it, a wrapper, and environment variables.
use super::servers_page::Field;
use super::*;
use gpui::{
    canvas, relative, Bounds, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point,
};
use horizon_launcher::launch;

pub(super) struct LaunchPanel {
    /// The instance it is for: picking another closes it.
    instance: String,
    /// What Save did: Ok with the summary of what runs (None when nothing runs), or Err with reason.
    saved: Option<Result<Option<String>, String>>,
    /// What this computer has for the Linux switches, looked at when the panel opens.
    found: launch::Found,
    /// The Linux switches (GameMode, MangoHud, dedicated card) as flipped, kept with Save like
    /// the commands: None until one is flipped.
    switches: Option<[bool; 3]>,
}

/// What saving the memory slider did, kept to show under the slider in the active language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MemorySaved {
    /// Saved Java heap size in MB.
    Ok(u32),
    /// Saving failed on disk.
    Error(String),
}

/// The memory row on Home. What it shows comes from the selected instance, except during a drag.
#[derive(Default)]
pub(super) struct MemorySlider {
    /// The computer's memory in MiB, read once: the slider's top.
    total: Option<u32>,
    area: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// The Java heap size being dragged to.
    dragging: Option<u32>,
    /// What the last change did, or why not.
    saved: Option<MemorySaved>,
    /// The instance whose value is being typed: saved to it when the box loses the keyboard, even
    /// if that's because another instance was picked.
    typing: Option<String>,
    /// The instance `saved` is about.
    shown: Option<String>,
}

impl MemorySlider {
    /// A new sheet starts from the saved value: nothing left typed in the box carries over.
    pub(super) fn forget_typing(&mut self) {
        self.typing = None;
        self.shown = None;
    }

    fn total(&mut self) -> u32 {
        *self.total.get_or_insert_with(launch::total_memory_mb)
    }
}

/// A Linux switch: which one, its label, and what it does.
#[derive(Clone, Copy)]
enum Toggle {
    GameMode,
    MangoHud,
    Gpu,
}

impl Launcher {
    pub(super) fn toggle_launch(&mut self) {
        if self.launch.take().is_some() {
            return;
        }
        let Some(i) = self.selected_instance().cloned() else {
            return;
        };
        self.servers
            .text(Field::PreLaunch)
            .set(i.launch.pre_launch.clone());
        self.servers
            .text(Field::Wrapper)
            .set(i.launch.wrapper.clone());
        self.servers
            .text(Field::LaunchEnv)
            .set(launch::env_line(&i.launch.env));
        self.servers
            .text(Field::PostExit)
            .set(i.launch.post_exit.clone());
        self.launch = Some(LaunchPanel {
            instance: i.id,
            saved: None,
            found: launch::Found::here(),
            switches: None,
        });
    }

    /// What the hook fields hold now, when the panel is open for `id`: the commands before and
    /// after the game, the wrapper and the variables, as typed.
    pub(super) fn hooks_typed(&self, id: &str) -> Option<[String; 4]> {
        self.launch.as_ref().filter(|p| p.instance == id)?;
        let typed = |f: Field| self.servers.value(f).trim().to_string();
        Some([
            typed(Field::PreLaunch),
            typed(Field::Wrapper),
            typed(Field::PostExit),
            typed(Field::LaunchEnv),
        ])
    }

    /// The Linux switches as flipped in the open panel for `id`, when one was.
    pub(super) fn switches_typed(&self, id: &str) -> Option<[bool; 3]> {
        self.launch.as_ref().filter(|p| p.instance == id)?.switches
    }

    /// Parse the open command fields (and the flipped switches) without writing any instance
    /// settings.
    pub(super) fn parsed_launch(&self, id: &str) -> Option<Result<launch::Launch, String>> {
        let fields = self.hooks_typed(id)?;
        let saved = self.instances.list.iter().find(|i| i.id == id)?;
        let switches = self.switches_typed(id);
        Some(
            parse_commands(saved.launch.clone(), fields).map(|mut launch| {
                if let Some([gamemode, mangohud, gpu]) = switches {
                    launch.gamemode = gamemode;
                    launch.mangohud = mangohud;
                    launch.dedicated_gpu = gpu;
                }
                launch
            }),
        )
    }

    pub(super) fn save_launch(&mut self) {
        let Some(id) = self.launch.as_ref().map(|p| p.instance.clone()) else {
            return;
        };
        let Some(parsed) = self.parsed_launch(&id) else {
            return;
        };
        let saved = parsed.and_then(|launch| instances::edit(&id, |m| m.launch = launch));
        let saved = saved.map(|i| {
            self.reload_instances();
            i.launch.summary()
        });
        if let Some(p) = &mut self.launch {
            p.saved = Some(saved);
        }
    }

    /// Flips a Linux switch in the draft: Save keeps it with the rest, Cancel drops it.
    fn flip(&mut self, toggle: Toggle) {
        let Some(panel) = &self.launch else {
            return;
        };
        let Some(saved) = self.instances.list.iter().find(|i| i.id == panel.instance) else {
            return;
        };
        let l = &saved.launch;
        let mut switches = panel
            .switches
            .unwrap_or([l.gamemode, l.mangohud, l.dedicated_gpu]);
        let i = match toggle {
            Toggle::GameMode => 0,
            Toggle::MangoHud => 1,
            Toggle::Gpu => 2,
        };
        switches[i] = !switches[i];
        if let Some(panel) = &mut self.launch {
            panel.switches = Some(switches);
        }
    }

    /// The Java instance being edited, or the selected Java instance on Home.
    fn memory_instance(&self) -> Option<instances::Instance> {
        match self
            .instances
            .dialog
            .as_ref()
            .and_then(|d| d.editing.as_ref())
        {
            Some(id) => self.instances.list.iter().find(|i| &i.id == id).cloned(),
            None => self.selected_instance().cloned(),
        }
        .filter(|i| i.java.is_some())
    }

    /// The value under the pointer. Saving waits until the mouse comes up.
    fn pick_memory(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(_) = self.memory_instance() else {
            return;
        };
        let slider = &mut self.memory_slider;
        let Some(area) = slider.area.get() else {
            return;
        };
        let width: f32 = area.size.width.into();
        if width <= 0. {
            return;
        }
        let across: f32 = (at.x - area.origin.x).into();
        let total = slider.total();
        let picked = launch::memory_at((across / width).clamp(0., 1.), total);
        slider.saved = None;
        slider.dragging = Some(picked);
        cx.notify();
    }

    /// A release ends the drag and saves it to the instance, once.
    fn finish_memory(&mut self, cx: &mut Context<Self>) {
        let Some(memory) = self.memory_slider.dragging.take() else {
            return;
        };
        let Some(id) = self.memory_instance().map(|i| i.id) else {
            return;
        };
        self.save_memory(&id, memory);
        cx.notify();
    }

    fn save_memory(&mut self, id: &str, memory: u32) {
        // In instance settings it is part of the draft: Save keeps it, Cancel drops it.
        let saved_mb = self
            .instances
            .list
            .iter()
            .find(|i| i.id == id)
            .and_then(Self::saved_memory);
        if let Some(d) =
            (self.instances.dialog.as_mut()).filter(|d| d.editing.as_deref() == Some(id))
        {
            d.memory = (saved_mb != Some(memory)).then_some(memory);
            self.memory_slider.saved = None;
            return;
        }
        let saved = instances::edit(id, |m| {
            if let Some(java) = &mut m.java {
                java.memory_mb = Some(memory);
            }
        });
        self.memory_slider.saved = Some(match saved {
            Ok(_) => MemorySaved::Ok(memory),
            Err(e) => MemorySaved::Error(e),
        });
        self.reload_instances();
    }

    /// The memory instance settings would save for the instance it edits: the box's text while
    /// the row shows that instance (typed or not), else the slider's draft. Err: the box holds
    /// something that isn't a size. Ok(None): nothing set in the sheet.
    pub(super) fn sheet_memory(&self) -> Result<Option<u32>, ()> {
        let Some(d) = self.instances.dialog.as_ref() else {
            return Ok(None);
        };
        let Some(id) = d.editing.as_deref() else {
            return Ok(None);
        };
        if self.memory_slider.shown.as_deref() != Some(id) {
            return Ok(d.memory);
        }
        launch::parse_memory(self.servers.value(Field::MemoryLimit))
            .map(|typed| Some(typed.unwrap_or(instances::DEFAULT_MEMORY_MB)))
            .map_err(|_| ())
    }

    /// The value the row shows: the sheet's draft for the instance it edits, else the saved one.
    fn shown_memory(&self, instance: &instances::Instance) -> Option<u32> {
        (self.instances.dialog.as_ref())
            .filter(|d| d.editing.as_deref() == Some(instance.id.as_str()))
            .and_then(|d| d.memory)
            .or_else(|| Self::saved_memory(instance))
    }

    /// The instance's saved value.
    pub(super) fn saved_memory(instance: &instances::Instance) -> Option<u32> {
        instance
            .java
            .as_ref()
            .map(|java| java.memory_mb.unwrap_or(instances::DEFAULT_MEMORY_MB))
    }

    /// Saves what's in the box to `id`, if it's a value and a new one.
    fn save_typed_memory(&mut self, id: &str) {
        let typed = self.servers.value(Field::MemoryLimit).to_owned();
        let Ok(memory) = launch::parse_memory(&typed) else {
            self.memory_slider.saved = None;
            self.servers.set_field_invalid(Field::MemoryLimit, true);
            return;
        };
        let memory = memory.unwrap_or(instances::DEFAULT_MEMORY_MB);
        self.servers.set_field_invalid(Field::MemoryLimit, false);
        let same = instances::list()
            .iter()
            .find(|i| i.id == id)
            .is_some_and(|i| self.shown_memory(i) == Some(memory));
        if !same {
            self.save_memory(id, memory);
        }
    }

    /// Enter in the box.
    pub(super) fn memory_enter(&mut self) {
        if let Some(id) = self.memory_slider.typing.clone() {
            self.save_typed_memory(&id);
        }
    }

    /// The box follows the saved value and the slider except while it has the keyboard, and saves
    /// when it loses it. Read every frame.
    fn sync_memory_box(&mut self, instance: &str, memory: u32) {
        if self.memory_slider.shown.as_deref() != Some(instance) {
            self.memory_slider.saved = None;
            self.memory_slider.shown = Some(instance.to_owned());
        }
        if self.servers.has_focus(Field::MemoryLimit) {
            self.memory_slider.typing = Some(instance.to_owned());
            return;
        }
        if let Some(id) = self.memory_slider.typing.take() {
            self.save_typed_memory(&id);
            return;
        }
        let shown = memory.to_string();
        if self.servers.value(Field::MemoryLimit) != shown {
            self.servers.text(Field::MemoryLimit).set(shown);
        }
    }

    /// The maximum Java heap size for the selected Java instance.
    pub(super) fn memory_row(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let instance = self.selected_instance()?.clone();
        instance.java.as_ref()?;
        let total = self.memory_slider.total();
        let area = self.memory_slider.area.clone();
        let saved = Self::saved_memory(&instance)?;
        let memory = self.memory_slider.dragging.unwrap_or(saved);
        self.sync_memory_box(&instance.id, memory);
        let typed = launch::parse_memory(self.servers.value(Field::MemoryLimit));
        // While typing, the slider follows a value it can read.
        let memory = match (self.servers.has_focus(Field::MemoryLimit), typed) {
            (true, Ok(typed)) => typed.unwrap_or(instances::DEFAULT_MEMORY_MB),
            _ => memory,
        };
        let unit = text(theme::LABEL).text_color(t.ink_soft).child("MB");
        let input = self.field(Field::MemoryLimit, "4096", 120., cx);
        let about = "How much memory Java may use (-Xmx). Set it too low and the game can crash.";
        let track = div()
            .id("launch-memory")
            .relative()
            .w_full()
            .h(px(20.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well)
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, _| {
                        area.set(Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(relative(launch::memory_fraction(memory, total)))
                    .bg(t.face),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.pick_memory(ev.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                // Only a press that began on the track drags it: a button held down elsewhere, for a
                // scrollbar say, can pass over it without changing what's saved.
                if ev.pressed_button == Some(MouseButton::Left)
                    && this.memory_slider.dragging.is_some()
                {
                    this.pick_memory(ev.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_memory(cx)),
            )
            // `on_mouse_up` fires only while the track is hovered: a drag released off it saves too.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_memory(cx)),
            );
        let result = self.memory_slider.saved.as_ref().map(|s| match s {
            MemorySaved::Ok(mb) => text(theme::CAPTION).text_color(t.muted).child(trf(
                "Saved: the next Play uses at most {mb} MB.",
                &[("mb", &mb.to_string())],
            )),
            MemorySaved::Error(e) => text(theme::CAPTION).text_color(t.danger).child(e.clone()),
        });
        Some(
            blocks::panel(&t)
                .flex()
                .flex_col()
                .gap(px(8.))
                .mb(px(8.))
                .px(px(16.))
                .py(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            blocks::pixel(16., FontWeight::MEDIUM)
                                .text_color(t.ink)
                                .child(tr("Maximum memory")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(input)
                                .child(unit),
                        ),
                )
                .child(track)
                .child(text(theme::CAPTION).text_color(t.muted).child(tr(about)))
                .children(result),
        )
    }

    /// Maximum memory in the glass look: a `SettingRow` of the instance's Edit dialog, the name
    /// and what it does on the left, the mockup's `Slider` (as Settings' Glass and Blur) and the
    /// box in MB on the right. In instance settings it is part of the draft (Save keeps it);
    /// on Home it is saved as it is set.
    pub(super) fn glass_memory_row(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::glass_ui as ui;
        const TRACK: f32 = 208.;
        let instance = self.memory_instance()?;
        let total = self.memory_slider.total();
        let area = self.memory_slider.area.clone();
        let saved = self.shown_memory(&instance)?;
        let memory = self.memory_slider.dragging.unwrap_or(saved);
        self.sync_memory_box(&instance.id, memory);
        let typed = launch::parse_memory(self.servers.value(Field::MemoryLimit));
        let memory = match (self.servers.has_focus(Field::MemoryLimit), typed) {
            (true, Ok(typed)) => typed.unwrap_or(instances::DEFAULT_MEMORY_MB),
            _ => memory,
        };
        let f = launch::memory_fraction(memory, total).clamp(0., 1.);
        let about = "How much memory Java may use (-Xmx). Set it too low and the game can crash.";
        let track = div()
            .id("instance-memory")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .w(px(TRACK))
            .h(px(24.))
            .cursor_pointer()
            .child(
                canvas(move |bounds, _, _| area.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .h(px(4.))
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
                    .left(px(f * (TRACK - 16.)))
                    .size(px(16.))
                    .rounded_full()
                    .bg(crate::glass_ui::paper()),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| this.pick_memory(ev.position, cx)),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left)
                    && this.memory_slider.dragging.is_some()
                {
                    this.pick_memory(ev.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_memory(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_memory(cx)),
            );
        let unit = ui::text(13., FontWeight::NORMAL, 0.6).child("MB");
        let input = self.field(Field::MemoryLimit, "4096", 96., cx);
        let result = self.memory_slider.saved.as_ref().map(|s| match s {
            MemorySaved::Ok(mb) => div().child(trf(
                "Saved: the next Play uses at most {mb} MB.",
                &[("mb", &mb.to_string())],
            )),
            MemorySaved::Error(e) => div().text_color(ui::danger()).child(e.clone()),
        });
        let note = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(tr(about))
            .children(result);
        Some(
            ui::setting_row(
                tr("Maximum memory"),
                Some(note.into_any_element()),
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(track)
                        .child(input)
                        .child(unit)
                        .into_any_element(),
                ),
            )
            .px_0(),
        )
    }

    /// The Linux switches this computer can use: GameMode and MangoHud when installed, the
    /// dedicated GPU with two cards.
    fn linux_switches(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let found = self.launch.as_ref()?.found;
        let now = self.selected_instance()?.launch.clone();
        let gpu_about = match found.nvidia {
            true => "NVIDIA render offload: __NV_PRIME_RENDER_OFFLOAD and its Vulkan layer.",
            false => "Mesa's DRI_PRIME=1, so Vulkan picks the other card.",
        };
        let mut list = div().flex().flex_col().gap(px(10.));
        for (toggle, shown, on, label, about) in [
            (
                Toggle::GameMode,
                found.gamemode,
                now.gamemode,
                "GameMode",
                "Feral's gamemoderun: CPU and scheduler tuned while you play.",
            ),
            (
                Toggle::MangoHud,
                found.mangohud,
                now.mangohud,
                "MangoHud",
                "The frame time, CPU and GPU overlay.",
            ),
            (
                Toggle::Gpu,
                found.two_gpus,
                now.dedicated_gpu,
                "Dedicated graphics card",
                gpu_about,
            ),
        ] {
            if !shown {
                continue;
            }
            list = list.child(
                div()
                    .id(SharedString::from(format!("launch-switch-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .child(switch(
                        SharedString::from(format!("launch-switch-{label}-s")),
                        on,
                        &t,
                    ))
                    .child(text(theme::LABEL).text_color(t.ink_soft).child(tr(label)))
                    .child(text(theme::CAPTION).text_color(t.muted).child(tr(about)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.flip(toggle);
                        cx.notify();
                    })),
            );
        }
        found.any().then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(caps(tr("On this computer")).text_color(t.muted))
                .child(list)
        })
    }

    /// The panel under the Instance row, while open for the selected instance.
    pub(super) fn launch_panel(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let selected = self.selected_instance().map(|i| i.id.clone());
        if self.launch.as_ref().map(|p| &p.instance) != selected.as_ref() {
            self.launch = None;
        }
        let saved = self.launch.as_ref()?.saved.clone();
        let row = |this: &mut Self,
                   cx: &mut Context<Self>,
                   label: &'static str,
                   field: Field,
                   placeholder: &'static str,
                   about: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(caps(tr(label)).text_color(t.muted))
                .child(this.field(field, tr(placeholder), 520., cx))
                .child(text(theme::CAPTION).text_color(t.muted).child(tr(about)))
        };
        let pre = row(
            self,
            cx,
            "Before the game",
            Field::PreLaunch,
            "For example ./backup.sh",
            "A shell command, run in the instance's folder. The game starts only if it succeeds.",
        );
        let wrapper = row(
            self,
            cx,
            "Run the game through",
            Field::Wrapper,
            "For example nice -n 5",
            "A program and its arguments; the game's command line goes after them.",
        );
        let env = row(
            self,
            cx,
            "Environment variables",
            Field::LaunchEnv,
            "For example NAME=value OTHER=\"two words\"",
            "For the game and the commands; INST_NAME, INST_DIR and the other instance variables are set too.",
        );
        let post = row(
            self,
            cx,
            "After the game",
            Field::PostExit,
            "For example ./backup.sh",
            "A shell command, run in the instance's folder once the game has exited.",
        );
        let save = button("launch-save", "Save", Kind::Secondary, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.save_launch();
                cx.notify();
            },
        ));
        let result = saved.map(|s| match s {
            Ok(Some(what)) => text(theme::CAPTION)
                .text_color(t.muted)
                .child(trf("Saved: the next Play {what}.", &[("what", &what)])),
            Ok(None) => text(theme::CAPTION)
                .text_color(t.muted)
                .child(tr("Saved: nothing runs around the game.")),
            Err(e) => text(theme::CAPTION).text_color(t.danger).child(e),
        });
        Some(
            plate(&t)
                .flex()
                .flex_col()
                .gap(px(14.))
                .px(px(14.))
                .py(px(12.))
                .ml(px(40.))
                .child(caps(tr("When it starts")).text_color(t.muted))
                .child(pre)
                .child(wrapper)
                .child(env)
                .child(post)
                .children(self.linux_switches(cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(save)
                        .children(result),
                ),
        )
    }
}

impl Launcher {
    /// The instance sheet's Launch section in the glass look: the commands around the game, its
    /// variables, and the Linux switches this computer can use. Opens the panel's state for the
    /// selected instance when it isn't yet.
    pub(super) fn glass_hooks(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::glass_ui as ui;
        use horizon_theme::glass::ink;
        let t = self.theme;
        let selected = self.selected_instance()?.clone();
        if self.launch.as_ref().map(|p| &p.instance) != Some(&selected.id) {
            self.launch = None;
            self.toggle_launch();
        }
        let panel = self.launch.as_ref()?;
        let (saved, found) = (panel.saved.clone(), panel.found);
        let row = |this: &mut Self,
                   cx: &mut Context<Self>,
                   label: &'static str,
                   field: Field,
                   placeholder: &'static str,
                   about: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(ui::text(13.5, FontWeight::MEDIUM, ink::FULL).child(tr(label)))
                .child(this.field(field, tr(placeholder), 560., cx))
                .child(ui::text(12.5, FontWeight::NORMAL, ink::NOTE).child(tr(about)))
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(row(
                self,
                cx,
                "Before the game",
                Field::PreLaunch,
                "For example ./backup.sh",
                "A shell command, run in the instance's folder. The game starts only if it succeeds.",
            ))
            .child(row(
                self,
                cx,
                "Run the game through",
                Field::Wrapper,
                "For example nice -n 5",
                "A program and its arguments; the game's command line goes after them.",
            ))
            .child(row(
                self,
                cx,
                "After the game",
                Field::PostExit,
                "For example ./backup.sh",
                "A shell command, run in the instance's folder once the game has exited.",
            ))
            .child(row(
                self,
                cx,
                "Environment variables",
                Field::LaunchEnv,
                "For example NAME=value OTHER=\"two words\"",
                "For the game and the commands; INST_NAME, INST_DIR and the other instance variables are set too.",
            ));
        let result = saved.map(|s| match s {
            Ok(Some(what)) => ui::text(12.5, FontWeight::NORMAL, ink::NOTE)
                .child(trf("Saved: the next Play {what}.", &[("what", &what)])),
            Ok(None) => ui::text(12.5, FontWeight::NORMAL, ink::NOTE)
                .child(tr("Saved: nothing runs around the game.")),
            Err(e) => ui::text(12.5, FontWeight::NORMAL, ink::FULL)
                .text_color(ui::color(horizon_theme::glass::DANGER_HOVER))
                .child(e),
        });
        body = body.child(div().flex().items_center().gap(px(12.)).children(result));
        // Linux only, and only what this computer has (`launch::Found`).
        if found.any() {
            let mut now = selected.launch.clone();
            if let Some([gamemode, mangohud, gpu]) = self.switches_typed(&selected.id) {
                (now.gamemode, now.mangohud, now.dedicated_gpu) = (gamemode, mangohud, gpu);
            }
            let gpu_about = match found.nvidia {
                true => "NVIDIA render offload: __NV_PRIME_RENDER_OFFLOAD and its Vulkan layer.",
                false => "Mesa's DRI_PRIME=1, so Vulkan picks the other card.",
            };
            let mut list =
                div().flex().flex_col().gap(px(12.)).child(
                    ui::text(13.5, FontWeight::MEDIUM, ink::FULL).child(tr("On this computer")),
                );
            for (toggle, shown, on, label, about) in [
                (
                    Toggle::GameMode,
                    found.gamemode,
                    now.gamemode,
                    "GameMode",
                    "Feral's gamemoderun: CPU and scheduler tuned while you play.",
                ),
                (
                    Toggle::MangoHud,
                    found.mangohud,
                    now.mangohud,
                    "MangoHud",
                    "The frame time, CPU and GPU overlay.",
                ),
                (
                    Toggle::Gpu,
                    found.two_gpus,
                    now.dedicated_gpu,
                    "Dedicated graphics card",
                    gpu_about,
                ),
            ] {
                if !shown {
                    continue;
                }
                list = list.child(ui::setting_row(
                    tr(label),
                    Some(div().child(tr(about)).into_any_element()),
                    Some(
                        ui::toggle(
                            SharedString::from(format!("hook-{label}")),
                            on,
                            self.look.accent,
                            self.look.on_accent,
                            &t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.flip(toggle);
                            cx.notify();
                        }))
                        .into_any_element(),
                    ),
                ));
            }
            body = body.child(list);
        }
        Some(body)
    }
}

fn parse_commands(
    mut launch: launch::Launch,
    [pre, wrapper, post, env]: [String; 4],
) -> Result<launch::Launch, String> {
    launch.env = launch::parse_env(&env)?;
    launch.pre_launch = pre;
    launch.wrapper = wrapper;
    launch.post_exit = post;
    Ok(launch)
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn parsing_commands_is_an_unsaved_draft() {
        let saved = launch::Launch::default();
        let fields = [
            "backup".into(),
            "wrapper".into(),
            "after".into(),
            "MODE=play".into(),
        ];
        let draft = parse_commands(saved.clone(), fields).unwrap();
        assert!(saved.pre_launch.is_empty());
        assert!(saved.env.is_empty());
        assert_eq!(draft.pre_launch, "backup");
        assert_eq!(draft.wrapper, "wrapper");
        assert_eq!(draft.post_exit, "after");
        assert_eq!(draft.env.get("MODE").unwrap(), "play");
        assert!(parse_commands(
            saved,
            [
                String::new(),
                String::new(),
                String::new(),
                "not an assignment".into()
            ]
        )
        .is_err());
    }
}
