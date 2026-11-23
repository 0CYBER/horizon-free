//! The selected instance's server list on Home (`horizon_launcher::server_list`, the game's own
//! `servers.dat`): each server with the picture the game last saw, Join (the game starts straight
//! into it), Remove after a confirmation, and a line to add one. The official server is put on top
//! once, as the game does, at the address links.json gives (`server_list::add_official`).
use super::*;
use gpui::{Image, ImageFormat};
use horizon_launcher::server_list::{self, Server};
use servers_page::Field;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Multiplayer {
    /// The game folder the list is of, and the list as last read with each picture decoded.
    home: Option<PathBuf>,
    pub(super) list: Vec<(Server, Option<Arc<Image>>)>,
    fresh: bool,
    /// Whether a read of `home` has arrived: until then the list is empty, not read.
    arrived: bool,
    /// The list's read on a thread: decoding and shrinking each server's picture is too slow
    /// to do while drawing.
    reads: modpacks_page::PackReads,
    pub(super) adding: bool,
    /// The row whose Remove waits for its confirmation, by its place in `list`. Any change to
    /// the list drops it (`stale`).
    pub(super) confirm_remove: Option<usize>,
    pub(super) error: Option<String>,
    /// What each address answered to the status ping, asked once per list.
    pub(super) reach: HashMap<String, Reach>,
}

/// A saved server as its status ping found it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reach {
    Asking,
    Online { players: u64, max: u64 },
    Offline,
}

impl Multiplayer {
    /// Read again on the next frame: the game may have changed its list.
    pub(super) fn stale(&mut self) {
        self.fresh = false;
        self.confirm_remove = None;
        self.reads.changed();
    }

    /// Whether the rows show the file as it is: after a change, until the new read arrives,
    /// their places may name other servers, so they offer no Remove.
    pub(super) fn current(&self) -> bool {
        self.fresh
    }
}

impl Launcher {
    /// Asks each listed server for its players once, as the game's Multiplayer list does when it
    /// opens; the rows show the answer (the mockup's dot and "3 of 10" or "Offline").
    pub(super) fn ping_servers(&mut self, cx: &mut Context<Self>) {
        let asks: Vec<String> = self
            .multiplayer
            .list
            .iter()
            .map(|(s, _)| s.ip.clone())
            .filter(|ip| !self.multiplayer.reach.contains_key(ip))
            .collect();
        for ip in asks {
            self.multiplayer.reach.insert(ip.clone(), Reach::Asking);
            let address = ip.clone();
            let task = cx
                .background_executor()
                .spawn(async move { horizon_launcher::telemetry::server_players(&address) });
            cx.spawn(async move |this, cx| {
                let answer = task.await;
                let _ = this.update(cx, |this, cx| {
                    let reach = match answer {
                        Some(p) => Reach::Online {
                            players: p.online,
                            max: p.max,
                        },
                        None => Reach::Offline,
                    };
                    this.multiplayer.reach.insert(ip, reach);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Reads the server list of `home` in the background when it is missing, out of date or of
    /// another instance: each server's picture is decoded and made small
    /// (`pictures::list_picture`).
    pub(super) fn load_servers(&mut self, home: PathBuf, cx: &mut Context<Self>) {
        if self.multiplayer.home.as_ref() != Some(&home) {
            // The read numbers go on, so a read of the instance picked before never applies.
            let mut reads = std::mem::take(&mut self.multiplayer.reads);
            reads.changed();
            self.multiplayer = Multiplayer {
                home: Some(home.clone()),
                reads,
                ..Multiplayer::default()
            };
        }
        if self.multiplayer.fresh || self.multiplayer.reads.running() {
            return;
        }
        // On this thread, as every other edit of the list: it reads no picture. A list that
        // can't be read is the game's to report: the page shows it empty.
        let _ = server_list::add_official(
            &home,
            &self.links.official_server,
            &self.links.official_server_previous,
        );
        let read = self.multiplayer.reads.ask();
        let task = cx.background_executor().spawn(async move {
            server_list::List::load(&home)
                .servers()
                .into_iter()
                .map(|s| {
                    let icon = s
                        .icon
                        .clone()
                        .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                    (s, icon)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let list = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.multiplayer.reads.arrived(read) {
                    this.multiplayer.list = list;
                    this.multiplayer.fresh = true;
                    this.multiplayer.arrived = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Edits the list as the file holds it now, and saves it when `edit` says it changed. Reads
    /// no picture.
    pub(super) fn edit_servers(
        &mut self,
        edit: impl FnOnce(&mut server_list::List) -> Result<bool, String>,
    ) {
        let Some(home) = self.multiplayer.home.clone() else {
            return;
        };
        let mut list = server_list::List::load(&home);
        self.multiplayer.error = edit(&mut list)
            .and_then(|changed| match changed {
                true => list.save(&home),
                false => Ok(()),
            })
            .err();
        self.multiplayer.stale();
    }

    /// The confirmed Remove of the server a row showed at `index`: removed from the file as it
    /// is now only when it is still there (`List::remove_confirmed`).
    pub(super) fn remove_server(&mut self, index: usize, server: &Server) {
        self.edit_servers(|list| Ok(list.remove_confirmed(index, &server.name, &server.ip)));
    }

    /// Enter in the address field, or Add.
    pub(super) fn add_server(&mut self) {
        let name = self.servers.value(Field::ServerName).to_string();
        let address = self.servers.value(Field::ServerAddress).to_string();
        self.edit_servers(|list| list.add(&name, &address).map(|()| true));
        if self.multiplayer.error.is_none() {
            self.servers.text(Field::ServerName).set(String::new());
            self.servers.text(Field::ServerAddress).set(String::new());
            self.multiplayer.adding = false;
        }
    }

    /// How many servers the selected instance's list has; none until the first read of its list
    /// has arrived.
    pub(super) fn servers_count(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let home = self.selected_instance()?.home.clone();
        self.load_servers(home, cx);
        self.multiplayer
            .arrived
            .then_some(self.multiplayer.list.len())
    }

    /// Home's Multiplayer tab, for the selected instance: its button (beside the tabs) and its
    /// list.
    pub(super) fn multiplayer_section(&mut self, cx: &mut Context<Self>) -> Option<(Div, Div)> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_servers(home, cx);
        let actions = div().child(tab_action("server-add", "Add server", &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.multiplayer.adding = !this.multiplayer.adding;
                this.multiplayer.error = None;
                if this.multiplayer.adding {
                    this.servers.focus_soon = Some(Field::ServerAddress);
                }
                cx.notify();
            }),
        ));
        let mut section = div().flex().flex_col().gap(px(8.));
        if let Some(e) = self.multiplayer.error.clone() {
            section = section.child(notice(e, &t));
        }
        if self.multiplayer.adding {
            section = section.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(10.))
                    .child(self.field(Field::ServerName, tr("Minecraft Server"), 220., cx))
                    .child(self.field(Field::ServerAddress, "play.example.com", 300., cx))
                    .child(button("server-save", "Add", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.add_server();
                            cx.notify();
                        }),
                    )),
            );
        }
        let list = self.multiplayer.list.clone();
        if list.is_empty() && !self.multiplayer.adding {
            section = section.child(text(theme::CAPTION).text_color(t.muted).child(tr(
                "No servers yet: add one here or in the game's Multiplayer screen.",
            )));
        }
        for (i, (server, icon)) in list.into_iter().enumerate() {
            section = section.child(self.server_row(i, server, icon, cx));
        }
        Some((actions, section))
    }

    fn server_row(
        &mut self,
        i: usize,
        server: Server,
        icon: Option<Arc<Image>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let picture = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(48.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well);
        let picture = match icon {
            Some(image) => picture.child(gpui::img(image).size(px(44.))),
            None => picture.child(blocks::icon("server", 22., t.muted)),
        };
        let mut actions = div().flex().items_center().gap(px(6.));
        let current = self.multiplayer.current();
        if current && self.multiplayer.confirm_remove == Some(i) {
            let gone = server.clone();
            actions = actions
                .child(
                    text(theme::CAPTION)
                        .text_color(t.danger)
                        .child(tr("Remove it from the list?")),
                )
                .child(
                    button(
                        SharedString::from(format!("server-keep-{i}")),
                        "Keep",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.multiplayer.confirm_remove = None;
                        cx.notify();
                    })),
                )
                .child(
                    button(
                        SharedString::from(format!("server-remove-{i}")),
                        "Remove",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.remove_server(i, &gone);
                            cx.notify();
                        },
                    )),
                );
        } else {
            let ip = server.ip.clone();
            let busy = !self.can_join_selected();
            let join = button(
                SharedString::from(format!("server-join-{i}")),
                "Join",
                Kind::Secondary,
                &t,
            );
            let join = if busy { disabled(join) } else { join };
            let delete = button(
                SharedString::from(format!("server-delete-{i}")),
                "Remove",
                Kind::Ghost,
                &t,
            );
            let delete = match current {
                true => delete.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.multiplayer.confirm_remove = Some(i);
                    cx.notify();
                })),
                false => disabled(delete),
            };
            actions = actions
                .child(
                    join.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.quick_play("--server", ip.clone());
                        cx.notify();
                    })),
                )
                .child(delete);
        }
        plate(&t)
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(10.))
            .child(picture)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        blocks::pixel(15., FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .truncate()
                            .child(server.name.clone()),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .truncate()
                            .child(server.ip.clone()),
                    ),
            )
            .child(actions)
    }
}
