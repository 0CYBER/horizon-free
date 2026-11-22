use super::*;
impl Title {
    pub(super) fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        // The game rules draw and press their own.
        if self.rules.borrow().is_some() {
            return Vec::new();
        }
        let (w, h) = (w.ceil(), h.ceil());
        let b = |x: f32, y: f32, w: f32, label: &str, enabled: bool, id: u8| Button {
            x,
            y,
            w,
            label: label.into(),
            enabled,
            id,
            agent_id: None,
            icon: None,
            trailing: None,
            action: None,
        };
        match &self.mode {
            Mode::Main => {
                let cx = (w / 2.0).floor();
                let y = (h / 4.0).floor() + 48.0;
                let mut buttons = vec![
                    b(
                        cx - 100.0,
                        y,
                        200.0,
                        &lang::get_or("menu.singleplayer", "Singleplayer"),
                        true,
                        1,
                    )
                    .identified("singleplayer"),
                    b(
                        cx - 100.0,
                        y + 24.0,
                        200.0,
                        &lang::get_or("menu.multiplayer", "Multiplayer"),
                        self.allows_multiplayer,
                        4,
                    )
                    .identified("multiplayer"),
                    b(
                        cx - 100.0,
                        y + 48.0,
                        200.0,
                        &lang::get_or("menu.online", "Minecraft Realms"),
                        // TitleScreen.createNormalMenuOptions: Realms is off when
                        // multiplayer is.
                        self.allows_multiplayer,
                        8,
                    )
                    .identified("minecraft-realms"),
                    b(
                        cx - 100.0,
                        y + 96.0,
                        98.0,
                        &lang::get_or("menu.options", "Options..."),
                        true,
                        3,
                    )
                    .identified("options")
                    .does(crate::widgets::Action::Options),
                    b(
                        cx + 2.0,
                        y + 96.0,
                        98.0,
                        &lang::get_or("menu.quit", "Quit Game"),
                        true,
                        2,
                    )
                    .identified("quit-game")
                    .does(crate::widgets::Action::Exit),
                ];
                // CommonButtons' icon buttons; Horizon's Join a Friend (an
                // invite code) sits behind vanilla's Friends button.
                for (i, (key, plain, agent_id, id, icon)) in [
                    ("horizon.mods.screen.title", "Mods", "mods", 9, "icon/mods"),
                    (
                        "gui.friends.open",
                        "Friends",
                        "friends",
                        5,
                        "friends/friends",
                    ),
                    (
                        "options.language",
                        "Language...",
                        "language",
                        6,
                        "icon/language",
                    ),
                    (
                        "options.accessibility",
                        "Accessibility Settings...",
                        "accessibility-settings",
                        7,
                        "icon/accessibility",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut button = b(
                        cx - 46.0 + i as f32 * 24.0,
                        y + 72.0,
                        20.0,
                        &lang::get_or(key, plain),
                        id != 5 || crate::net::joins_friends(),
                        id,
                    )
                    .identified(agent_id);
                    button.icon = Some((icon, 15.0));
                    buttons.push(button);
                }
                buttons
            }
            Mode::Select { worlds, selected } => {
                let world = selected.and_then(|i| worlds.get(i));
                // SelectWorldScreen.updateButtonStatus: primaryActionMessage, canEdit, canRecreate.
                let corrupted = world.is_some_and(|w| w.corrupted);
                let editable = world.is_some_and(WorldEntry::can_edit);
                let (play, play_id) = if corrupted {
                    (
                        lang::get_or("recover_world.button", "Attempt to Recover"),
                        "attempt-to-recover",
                    )
                } else if world.is_some_and(|w| w.file_fixing) {
                    (
                        lang::get_or("selectWorld.upgrade_and_play", "Upgrade and Play"),
                        "upgrade-and-play",
                    )
                } else {
                    (
                        lang::get_or("selectWorld.select", "Play Selected World"),
                        "play-selected-world",
                    )
                };
                vec![
                    b(
                        (w / 2.0).floor() - 154.0,
                        h - 52.0,
                        150.0,
                        &play,
                        world.is_some_and(WorldEntry::can_play),
                        10,
                    )
                    .identified(play_id),
                    b(
                        (w / 2.0).floor() + 4.0,
                        h - 52.0,
                        150.0,
                        &lang::get_or("selectWorld.create", "Create New World"),
                        true,
                        11,
                    )
                    .identified("create-new-world"),
                    b(
                        (w / 2.0).floor() - 154.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("selectWorld.edit", "Edit"),
                        editable,
                        13,
                    )
                    .identified("edit"),
                    b(
                        (w / 2.0).floor() - 75.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("selectWorld.delete", "Delete"),
                        selected.is_some(),
                        14,
                    )
                    .identified("delete"),
                    b(
                        (w / 2.0).floor() + 4.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("selectWorld.recreate", "Re-Create"),
                        editable,
                        15,
                    )
                    .identified("re-create"),
                    b(
                        (w / 2.0).floor() + 83.0,
                        h - 28.0,
                        71.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        12,
                    )
                    .identified("back")
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Create {
                creative, options, ..
            } => create_world::buttons(w, h, *creative, options, self.preset_data.is_some()),
            Mode::CreateFlat { flat, .. } => flat::buttons(w, h, flat),
            Mode::CreateBuffet { buffet, .. } => buffet::buttons(w, h, buffet),
            Mode::CreatePreset { .. } => vec![
                b(
                    w / 2.0 - 155.0,
                    h - 28.0,
                    150.0,
                    &lang::get("gui.done"),
                    true,
                    94,
                )
                .identified("done")
                .does(crate::widgets::Action::Done),
                b(
                    w / 2.0 + 5.0,
                    h - 28.0,
                    150.0,
                    &lang::get("gui.cancel"),
                    true,
                    95,
                )
                .identified("cancel")
                .does(crate::widgets::Action::Back),
            ],
            Mode::Edit {
                world,
                name,
                adopt_regions,
                ..
            } => {
                let x = (w / 2.0).floor() - 100.0;
                let top = edit_top(h);
                let mut buttons = vec![
                    b(
                        x,
                        top + 64.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.resetIcon", "Reset Icon"),
                        self.saves.join(&world.folder).join("icon.png").is_file(),
                        30,
                    ),
                    b(
                        x,
                        top + 89.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.openFolder", "Open World Folder"),
                        true,
                        31,
                    ),
                    b(
                        x,
                        top + 114.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.backup", "Make Backup"),
                        true,
                        32,
                    ),
                    b(
                        x,
                        top + 139.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.backupFolder", "Open Backups Folder"),
                        true,
                        33,
                    ),
                    b(
                        x,
                        top + 164.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.optimize", "Optimize World"),
                        true,
                        34,
                    ),
                    b(
                        x,
                        top + 214.0,
                        98.0,
                        &lang::get_or("selectWorld.edit.save", "Save"),
                        !name.trim().is_empty(),
                        35,
                    ),
                    b(
                        x + 102.0,
                        top + 214.0,
                        98.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        36,
                    )
                    .does(crate::widgets::Action::Cancel),
                ];
                // Horizon's opt-in (arismoko/horizon#2554), in the row vanilla leaves empty above Save.
                if *adopt_regions {
                    buttons.push(b(
                        x,
                        top + 189.0,
                        200.0,
                        &lang::get("horizon.biome_regions.adopt.button"),
                        true,
                        98,
                    ));
                }
                buttons
            }
            Mode::AdoptBiomeRegions { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get("horizon.biome_regions.adopt.confirm"),
                        true,
                        99,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        100,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Delete { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.deleteButton", "Delete"),
                        true,
                        37,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        38,
                    )
                    .does(crate::widgets::Action::Cancel),
                ]
            }
            Mode::WorldRequest { .. } => {
                let area = world_request_layout((w, h));
                vec![
                    b(
                        area.yes_x,
                        area.buttons_y,
                        area.button_w,
                        &lang::get_or("gui.yes", "Yes"),
                        true,
                        104,
                    ),
                    b(
                        area.no_x,
                        area.buttons_y,
                        area.button_w,
                        &lang::get_or("gui.no", "No"),
                        true,
                        105,
                    ),
                ]
            }
            Mode::LowDiskSpace { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("gui.continue", "Continue"),
                        true,
                        94,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        95,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::MissingMods { .. } => {
                let (_, _, narrow) = missing_mods::layout(w, h);
                let button_width = if narrow { (w - 20.0).min(300.0) } else { 150.0 };
                let center = (w - button_width) / 2.0;
                let y = h - if narrow { 76.0 } else { 52.0 };
                vec![
                    // Don't Open is the default: first, and what Escape does.
                    b(
                        center,
                        y + if narrow { 48.0 } else { 24.0 },
                        button_width,
                        &lang::get("horizon.missing_mods.dont_open"),
                        true,
                        101,
                    )
                    .does(crate::widgets::Action::Cancel),
                    b(
                        if narrow {
                            center
                        } else {
                            (w / 2.0).floor() - 152.0
                        },
                        y,
                        button_width,
                        &lang::get("horizon.missing_mods.backup_open"),
                        true,
                        102,
                    ),
                    b(
                        if narrow {
                            center
                        } else {
                            (w / 2.0).floor() + 2.0
                        },
                        y + if narrow { 24.0 } else { 0.0 },
                        button_width,
                        &lang::get("horizon.missing_mods.open"),
                        true,
                        103,
                    ),
                ]
            }
            Mode::MissingBiomeRegions { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get("horizon.biome_regions.missing.open_vanilla"),
                        true,
                        96,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get("horizon.biome_regions.missing.dont_open"),
                        true,
                        97,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Recover { issues, .. } => {
                let y = recover_buttons_top(h, self.message_lines.get());
                // The restore button is active only when level.dat is the broken one
                // and level.dat_old reads.
                vec![
                    b(
                        (w / 2.0).floor() - 60.0,
                        y,
                        120.0,
                        &lang::get_or("recover_world.restore", "Attempt to Restore"),
                        issues[0].is_some() && issues[1].is_none(),
                        90,
                    ),
                    b(
                        (w / 2.0).floor() - 60.0,
                        y + 28.0,
                        120.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        91,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Recovered { success: true, .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("gui.continue", "Continue"),
                        true,
                        92,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        93,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Recovered { success: false, .. } => {
                // AlertScreen.init: the button follows the text, kept inside the screen.
                let y = (90.0 + self.message_lines.get() as f32 * 9.0 + 12.0)
                    .max(h / 6.0 + 96.0)
                    .min(h - 24.0);
                vec![
                    b(
                        (w / 2.0).floor() - 75.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        93,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Error { alert, .. } => {
                let lines = self.message_lines.get();
                let y = if alert.is_some() {
                    // AlertScreen.init: the button follows the text, kept inside the screen.
                    (90.0 + lines as f32 * 9.0 + 12.0)
                        .max(h / 6.0 + 96.0)
                        .min(h - 24.0)
                } else {
                    confirm_top(h, lines) + 41.0 + lines as f32 * 9.0
                };
                vec![
                    b(
                        (w / 2.0).floor() - 75.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        39,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::BackupBeforeUpgrade { .. } => {
                // BackupConfirmScreen.init: the buttons sit under the text at fixed offsets.
                let y = 109.0 + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 155.0,
                        y,
                        150.0,
                        &lang::get_or(
                            "selectWorld.backupJoinConfirmButton",
                            "Create Backup and Load",
                        ),
                        true,
                        40,
                    ),
                    b(
                        (w / 2.0).floor() + 5.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.backupJoinSkipButton", "I Know What I'm Doing!"),
                        true,
                        41,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        y + 24.0,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        42,
                    )
                    .does(crate::widgets::Action::Cancel),
                ]
            }
            Mode::OptimizeConfirm { .. } => {
                // BackupConfirmScreen.init with the Erase Cached Data box: the buttons sit
                // under the text at fixed offsets.
                let y = 100.0 + (self.message_lines.get() as f32 + 1.0) * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 155.0,
                        y,
                        150.0,
                        &lang::get_or(
                            "optimizeWorld.confirm.proceed",
                            "Create Backup and Optimize",
                        ),
                        true,
                        43,
                    ),
                    b(
                        (w / 2.0).floor() + 5.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.backupJoinSkipButton", "I Know What I'm Doing!"),
                        true,
                        44,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        y + 24.0,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        45,
                    )
                    .does(crate::widgets::Action::Cancel),
                ]
            }
            // No `does`: Escape is inert here on purpose, so B is too. A on the button stops it.
            Mode::Optimize { .. } => vec![b(
                (w / 2.0).floor() - 100.0,
                (h / 4.0).floor() + 150.0,
                200.0,
                &lang::get_or("gui.cancel", "Cancel"),
                true,
                46,
            )],
            Mode::MultiplayerWarning { .. } => {
                let lines = self.message_lines.get();
                let y = warning_top(h, lines) + 82.0 + lines as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 154.0,
                        y,
                        150.0,
                        &lang::get_or("gui.proceed", "Proceed"),
                        true,
                        70,
                    ),
                    b(
                        (w / 2.0).floor() + 4.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        71,
                    )
                    .does(crate::widgets::Action::Back),
                ]
            }
            Mode::Onboarding(_) => self.onboarding_buttons(w, h),
            Mode::BanNotice(_) => {
                // ConfirmLinkScreen.addButtons: Open in Browser, Copy to Clipboard, then the
                // notice's Acknowledge, 100 wide each.
                let labels = [
                    lang::get_or("chat.link.open", "Open in Browser"),
                    lang::get_or("chat.copy", "Copy to Clipboard"),
                    lang::get_or("gui.acknowledge", "Acknowledge"),
                ];
                let positions = self.ban_buttons.get();
                (0..3)
                    .map(|i| {
                        b(
                            positions[i].0,
                            positions[i].1,
                            100.0,
                            &labels[i],
                            true,
                            120 + i as u8,
                        )
                    })
                    .collect()
            }
            Mode::Backup { .. } => Vec::new(),
            Mode::Multiplayer(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id)| {
                    let mut button = b(x, y, bw, &label, enabled, id);
                    button.action = crate::multiplayer::button_action(id);
                    button
                })
                .collect(),
            Mode::Realms(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id, icon)| {
                    let mut btn = b(x, y, bw, &label, enabled, id);
                    btn.icon = icon;
                    btn
                })
                .collect(),
        }
    }
}
