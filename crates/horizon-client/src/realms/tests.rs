use super::*;

fn server(id: i64, owner_uuid: Option<&str>) -> RealmServer {
    RealmServer {
        id,
        name: "Friends".into(),
        state: "OPEN".into(),
        owner_uuid: owner_uuid.map(str::to_owned),
        active_slot: 1,
        ..Default::default()
    }
}

fn listed(servers: Vec<RealmServer>) -> Screen {
    Screen {
        state: FetchState::Loaded(servers),
        subscreen: SubScreen::Main,
        selected: Some(0),
        joining: None,
        leaving: None,
        last_click: None,
        saves: None,
        news_link: None,
        news_job: None,
        news_unread: false,
        pending_invites: 0,
        pending_invites_job: None,
        join_width: std::cell::Cell::new(90.0),
        pdp_row: None,
        list_clicked: false,
        link_buttons: std::cell::Cell::new([(0.0, 0.0); 3]),
        terms_link: std::cell::Cell::new([0.0; 4]),
        notifications: Vec::new(),
        notifications_job: None,
        handled_seen: Vec::new(),
        dismiss_job: None,
        notice_rects: std::cell::Cell::new([[0.0; 4]; 2]),
        notice_height: std::cell::Cell::new(0.0),
        notice_buttons: std::cell::Cell::new([[0.0; 4]; 2]),
        popup_buttons: std::cell::Cell::new([[0.0; 4]; 2]),
        main_scroll: crate::ui::Scroll::default(),
        subscription_buttons: std::cell::Cell::new((0.0, 0.0)),
        players_button_rects: std::cell::RefCell::new(Vec::new()),
        backup_row_buttons: std::cell::RefCell::new(Vec::new()),
        settings_scroll: crate::ui::Scroll::default(),
        settings_slider: None,
        other_slider: None,
        after_save: None,
        redirect_to_pdp: false,
        template_link_rect: std::cell::Cell::new([0.0; 4]),
        trial: false,
        trial_job: None,
        live_players: std::collections::HashMap::new(),
        live_job: None,
        last_list_refresh: Instant::now(),
        last_pending_refresh: Instant::now(),
        last_news_refresh: Instant::now(),
        region_screen_requested: false,
        template_images: std::cell::RefCell::new(std::collections::HashMap::new()),
        world_icons: std::cell::RefCell::new(std::collections::HashMap::new()),
        skin_workers: SkinWorkers {
            profiles: std::sync::Arc::new(std::sync::Mutex::new(crate::skins::Resolver::new())),
            downloads: std::sync::Arc::new(std::sync::Mutex::new(crate::skins::Downloader::new(
                std::env::temp_dir().join("horizon-test-skins"),
            ))),
        },
        skin_faces: std::cell::RefCell::new(std::collections::HashMap::new()),
    }
}

fn notice(uuid: &str) -> RealmsNotification {
    RealmsNotification {
        uuid: uuid.to_owned(),
        dismissable: true,
        seen: false,
        kind: "visitUrl".into(),
        url: Some("https://realms.example/notice".into()),
        button_text: None,
        message: None,
        title: None,
        image: None,
        url_button: None,
    }
}

/// `RealmsMainScreen`: a notification alone makes the list, and the first `visitUrl` is
/// its entry; it is answered seen once (`handledSeenNotifications`).
#[test]
fn the_lists_first_visit_url_is_answered_seen_once() {
    let mut screen = listed(Vec::new());
    screen.notifications = vec![notice("a"), notice("b")];
    assert_eq!(screen.layout_state(), LayoutState::List);
    screen.mark_notifications_seen();
    assert_eq!(screen.handled_seen, vec!["a".to_owned()]);
    screen.mark_notifications_seen();
    assert_eq!(screen.handled_seen, vec!["a".to_owned()], "once");
    // An infoPopup is not the list's entry.
    let mut info = notice("c");
    info.kind = "infoPopup".into();
    screen.notifications = vec![info];
    assert!(screen.notice().is_none());
}

/// `RealmsMainScreen.dismissNotification`: a dismissable entry leaves at once; without a
/// session no request goes out.
#[test]
fn dismissing_a_notification_takes_it_off_the_list() {
    let mut screen = listed(Vec::new());
    screen.notifications = vec![notice("a")];
    screen.dismiss_notification("a");
    assert!(screen.notifications.is_empty());
    assert!(screen.dismiss_job.is_none());
}

#[test]
fn join_addresses_need_a_host_and_a_numeric_port() {
    for good in [
        "1.2.3.4:25565",
        "eu-west.realms.example:30001",
        "[2001:db8::1]:25565",
        " a.b:1 ",
    ] {
        assert_eq!(
            connect_address(good).as_deref(),
            Some(good.trim()),
            "{good}"
        );
    }
    for bad in [
        "",
        ":25565",
        "1.2.3.4",
        "1.2.3.4:",
        "1.2.3.4:http",
        "1.2.3.4:65536",
        "1.2.3.4:0",
        "1.2.3.4:-1",
        "https://1.2.3.4:25565",
        "tcp://host:25565",
        "user@host:25565",
        "host/path:25565",
        "2001:db8::1:25565",
        "[]:25565",
        "host name:25565",
    ] {
        assert_eq!(connect_address(bad), None, "{bad:?}");
    }
}

#[test]
fn leave_asks_first_and_cancel_leaves_nothing() {
    const ME: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let theirs = server(7, Some("fedcba9876543210fedcba9876543210"));
    // Our own Realm: no question, nothing to leave.
    let mut screen = listed(vec![server(7, Some("0123456789abcdef0123456789abcdef"))]);
    screen.leave_clicked(Some(ME));
    assert!(matches!(screen.subscreen, SubScreen::Main));

    // Someone else's: the first click only asks.
    let mut screen = listed(vec![theirs.clone()]);
    screen.leave_clicked(Some(ME));
    assert!(matches!(&screen.subscreen, SubScreen::Popup(Popup::Leave(s)) if *s == theirs));
    assert!(screen.leaving.is_none());
    let ids: Vec<u8> = screen.buttons(800.0, 600.0).iter().map(|b| b.5).collect();
    assert_eq!(ids, [50, 51]);

    // Cancel goes back with nothing sent and the Realm still selected.
    let _ = screen.click(Some(51), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(screen.subscreen, SubScreen::Main));
    assert!(screen.leaving.is_none());
    assert_eq!(screen.selected, Some(0));

    // Escape closes the question too.
    screen.leave_clicked(Some(ME));
    assert!(!screen.escape());
    assert!(matches!(screen.subscreen, SubScreen::Main));
    assert!(screen.leaving.is_none());

    // Signed out: no question at all.
    screen.leave_clicked(None);
    assert!(matches!(screen.subscreen, SubScreen::Main));
}

#[test]
fn the_pack_question_connects_only_after_continue_with_the_pack_queued() {
    let pack = RealmsPack {
        address: "9.8.7.6:25565".into(),
        id: 5,
        url: "https://packs.test/p.zip".into(),
        hash: "0".repeat(40),
    };
    let mut screen = listed(vec![]);
    screen.subscreen = SubScreen::Popup(Popup::ResourcePack { pack: pack.clone() });
    let _ = screen.click(Some(51), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(screen.subscreen, SubScreen::Main));

    screen.subscreen = SubScreen::Popup(Popup::ResourcePack { pack: pack.clone() });
    let outcome = screen.click(Some(50), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(outcome, Outcome::Connect(ref a) if a == "9.8.7.6:25565"));
    assert_eq!(take_pending_pack("other:1"), None);
    // A taken pack is gone, whatever the address.
    assert_eq!(take_pending_pack("9.8.7.6:25565"), None);

    screen.subscreen = SubScreen::Popup(Popup::ResourcePack { pack: pack.clone() });
    let _ = screen.click(Some(50), 0.0, 0.0, (800.0, 600.0));
    assert_eq!(take_pending_pack("9.8.7.6:25565"), Some(pack));
}

#[test]
fn a_join_answer_becomes_a_checked_connect_or_the_pack_question() {
    let mut screen = listed(vec![server(7, None)]);
    let answer = |address: &str, pack: bool| RealmAddress {
        address: Some(address.into()),
        resource_pack_url: pack.then(|| "https://packs.test/p.zip".into()),
        resource_pack_hash: pack.then(|| "ab".repeat(20)),
    };
    let start = |screen: &mut Screen, result: Result<RealmAddress, RealmsError>| {
        let job = Arc::new(Mutex::new(Some(result)));
        screen.joining = Some((job, Arc::new(AtomicBool::new(false))));
        screen.subscreen = SubScreen::Connecting {
            server: server(7, None),
            error: None,
            last: Box::new(SubScreen::Main),
        };
        screen.poll()
    };
    assert!(
        matches!(start(&mut screen, Ok(answer("1.2.3.4:25565", false))), Outcome::Connect(a) if a == "1.2.3.4:25565")
    );
    assert!(matches!(screen.subscreen, SubScreen::Main));

    assert!(matches!(
        start(&mut screen, Ok(answer("http://evil:25565", false))),
        Outcome::None
    ));
    // `LongRunningTask.error` replaces the connecting screen with the generic error
    // screen; its OK returns to the list.
    assert!(matches!(
        screen.subscreen,
        SubScreen::Error { ref next, .. } if matches!(**next, SubScreen::Main)
    ));

    assert!(matches!(
        start(&mut screen, Ok(answer("1.2.3.4:25565", true))),
        Outcome::None
    ));
    match &screen.subscreen {
        SubScreen::Popup(Popup::ResourcePack { pack }) => {
            assert_eq!(pack.address, "1.2.3.4:25565");
            assert_eq!(pack.id, pack_id(&server(7, None)));
        }
        other => panic!("{other:?}"),
    }

    assert!(matches!(
        start(&mut screen, Err(RealmsError::Timeout)),
        Outcome::None
    ));
    // `LongRunningTask.error` replaces the connecting screen with the generic error
    // screen; its OK returns to the list.
    assert!(matches!(
        screen.subscreen,
        SubScreen::Error { ref next, .. } if matches!(**next, SubScreen::Main)
    ));
}

/// `GetServerDetailsTask`: a 6002 answer shows `RealmsTermsScreen`, and Disagree
/// returns to the screen the join started from.
#[test]
fn a_6002_join_shows_the_terms_and_disagree_goes_back() {
    let mut screen = listed(vec![server(7, None)]);
    let job = Arc::new(Mutex::new(Some(Err(RealmsError::Service {
        status: 400,
        code: Some(6002),
        reason: None,
        message: None,
    }))));
    screen.joining = Some((job, Arc::new(AtomicBool::new(false))));
    screen.subscreen = SubScreen::Connecting {
        server: server(7, None),
        error: None,
        last: Box::new(SubScreen::Main),
    };
    assert!(matches!(screen.poll(), Outcome::None));
    assert!(matches!(screen.subscreen, SubScreen::Terms { .. }));
    // `RealmsTermsScreen.init`: Agree and Disagree are 23 and 24; Disagree is the
    // screen the join started from.
    let buttons = screen.buttons(800.0, 600.0);
    assert!(buttons.iter().any(|b| b.5 == 23));
    assert!(buttons.iter().any(|b| b.5 == 24));
    assert!(matches!(
        screen.click(Some(24), 0.0, 0.0, (800.0, 600.0)),
        Outcome::None
    ));
    assert!(matches!(screen.subscreen, SubScreen::Main));
}

#[test]
fn cancelling_a_join_aborts_its_worker() {
    let mut screen = listed(vec![server(7, None)]);
    let aborted = Arc::new(AtomicBool::new(false));
    screen.joining = Some((Arc::new(Mutex::new(None)), Arc::clone(&aborted)));
    screen.subscreen = SubScreen::Connecting {
        server: server(7, None),
        error: None,
        last: Box::new(SubScreen::Main),
    };
    let _ = screen.click(Some(52), 0.0, 0.0, (800.0, 600.0));
    assert!(aborted.load(Ordering::Relaxed));
    assert!(screen.joining.is_none());
    assert!(matches!(screen.subscreen, SubScreen::Main));
}

#[test]
fn pack_ids_are_javas_name_uuids() {
    // UUID.nameUUIDFromBytes("realms:Friends:1"): MD5 with the version-3 bits,
    // computed outside Rust.
    assert_eq!(
        horizon_protocol::crypto::uuid_string(pack_id(&server(7, None))),
        "9b833ced-6630-3283-b4a6-778948c32b5f"
    );
}

#[test]
fn without_a_session_nothing_is_requested() {
    let screen = Screen::new();
    assert!(matches!(screen.state, FetchState::Unauthenticated));
    let buttons = screen.buttons(800.0, 600.0);
    let enabled = |id| buttons.iter().find(|b| b.5 == id).map(|b| b.4);
    assert_eq!(enabled(14), Some(false), "Join Realm");
    assert_eq!(enabled(15), Some(false), "invites");
}

/// `InviteCodeExpiration.closestTo`: an existing expiration reads back as the choice
/// nearest to the time left.
#[test]
fn an_expiration_reads_back_as_the_closest_choice() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert_eq!(closest_expiration(now + 30 * 60 * 1000), 0);
    assert_eq!(closest_expiration(now + 60 * 60 * 1000), 1);
    assert_eq!(closest_expiration(now + 6 * 24 * 60 * 60 * 1000), 5);
    // A week is the longest timed choice: anything further reads back as it. (The
    // "never" row is the fallback for a code with no expiration, not a nearest.)
    assert_eq!(closest_expiration(now + 30 * 24 * 60 * 60 * 1000), 5);
}

/// `RealmsSettingsTab.updateData`: the draft starts from the Realm's own name,
/// description and region preference; a Realm without one is automatic by owner
/// (`RegionSelectionPreferenceDto.DEFAULT`), and only a manual choice sends its region.
#[test]
fn the_settings_draft_starts_from_the_realm() {
    let server: RealmServer = serde_json::from_str(
            r#"{"id":7,"name":"Mine","motd":"Hello",
                "regionSelectionPreference":{"regionSelectionPreference":2,"preferredRegion":"EastUs2"}}"#,
        )
        .unwrap();
    let mut draft = SettingsDraft::of(&server);
    assert_eq!(
        (draft.name.as_str(), draft.description.as_str()),
        ("Mine", "Hello")
    );
    assert_eq!(
        (draft.preference, draft.region.as_deref()),
        (2, Some("EastUs2"))
    );
    draft.preference = client::AUTOMATIC_PLAYER;
    assert_eq!(draft.preference_dto().preferred_region, None);
    let bare = SettingsDraft::of(&RealmServer::default());
    assert_eq!((bare.preference, bare.region), (1, None));
}

/// `ScrollableLayout`'s scissor: the region and open/close buttons live inside the
/// scroll view, so one below it must not be drawn (nor clickable) outside the tab.
#[test]
fn the_settings_buttons_leave_the_scroll_view() {
    let mut screen = listed(vec![server(7, None)]);
    let server = server(7, None);
    screen.subscreen = SubScreen::Configure {
        server: server.clone(),
        tab: ConfigureTab::Settings,
        subscription: None,
        subscription_job: None,
        detail: None,
        detail_job: None,
        deleting: None,
        member_job: None,
        settings: Some(SettingsDraft::of(&server)),
        players_scroll: crate::ui::Scroll::default(),
        tier: None,
        tier_job: None,
        regions: None,
        regions_job: None,
        saving: None,
        error: None,
        job: None,
    };
    let ids = |h: f32| -> Vec<u8> { screen.buttons(800.0, h).into_iter().map(|b| b.5).collect() };
    // 230 tall: the toggle (row 5) is past the 24..h-33 view, the region button is in.
    let short = ids(230.0);
    assert!(short.contains(&89));
    assert!(!short.contains(&30), "{short:?}");
    // Tall enough for the whole column: both are offered.
    let tall = ids(400.0);
    assert!(tall.contains(&89) && tall.contains(&30), "{tall:?}");
}

/// `RealmsWorldOptions.getSlotName`: the player's name, else the empty marker, else
/// "World N" by the slot.
#[test]
fn a_slot_without_a_name_says_world_n_or_empty() {
    let options = RealmWorldOptions::default();
    assert_eq!(
        slot_name(&options, 3),
        crate::lang::tr("mco.configure.world.slot", &[&3])
    );
    let empty = RealmWorldOptions {
        empty: true,
        ..Default::default()
    };
    assert_eq!(
        slot_name(&empty, 2),
        crate::lang::get("mco.configure.world.slot.empty")
    );
    let named = RealmWorldOptions {
        slot_name: "Base".into(),
        ..Default::default()
    };
    assert_eq!(slot_name(&named, 1), "Base");
}

/// `RealmsWorldsTab`: the active slot is 80 square, the others 50, 16 apart, centred.
#[test]
fn the_slot_buttons_give_the_active_slot_eighty_pixels() {
    let boxes = slot_boxes(427.0, 2);
    // 80 + 50*3 + 16*3 = 278 wide, centred on 427 starts at 74.
    assert_eq!(boxes[0], (74.0, 50.0, 1));
    assert_eq!(boxes[1].0, 140.0);
    assert_eq!(boxes[1].1, 80.0);
    assert_eq!(boxes[1].2, 2);
    assert_eq!(boxes[3].1, 50.0);
}

/// `RealmsWorldSlotButton.State`'s texture order: the minigame's image (or
/// `isles.png`), the empty frame, the slot's template image, the panoramas.
#[test]
fn the_slot_picture_follows_vanillas_order() {
    assert_eq!(
        slot_picture(4, true, Some("img")),
        SlotPicture::Image("img")
    );
    assert_eq!(slot_picture(4, true, None), SlotPicture::Preset);
    assert_eq!(slot_picture(4, false, None), SlotPicture::Preset);
    assert_eq!(slot_picture(2, true, Some("img")), SlotPicture::Empty);
    assert_eq!(
        slot_picture(2, false, Some("img")),
        SlotPicture::Image("img")
    );
    assert_eq!(
        slot_picture(1, false, None),
        SlotPicture::Panorama("panorama_0")
    );
    assert_eq!(
        slot_picture(2, false, None),
        SlotPicture::Panorama("panorama_2")
    );
    assert_eq!(
        slot_picture(3, false, None),
        SlotPicture::Panorama("panorama_3")
    );
}

/// `RealmsSelectWorldTemplateScreen`'s empty message: vanilla passes `"%link"` as the
/// translation's `%s` argument, so only that line carries the link.
#[test]
fn the_empty_template_message_links_one_line() {
    let lines = template_none_lines("Oops.\nPlease check back,\n%s.", "submit something");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], ("Oops.".into(), None, String::new()));
    assert_eq!(lines[1], ("Please check back,".into(), None, String::new()));
    assert_eq!(
        lines[2],
        ("".into(), Some("submit something".into()), ".".into())
    );
    // The Chinese translation's third line is `%s。`.
    let zh = template_none_lines("抱歉。\n请稍后再来，\n%s。", "提交内容");
    assert_eq!(zh[2], ("".into(), Some("提交内容".into()), "。".into()));
}

/// `RealmsInviteScreen.onInvite`: a blank name and one already invited are refused
/// before any request.
#[test]
fn inviting_an_empty_or_duplicate_name_asks_nothing() {
    let mut screen = Screen {
        state: FetchState::Loaded(vec![server(7, None)]),
        subscreen: SubScreen::Invite {
            server: server(7, None),
            players: vec![PlayerInfo {
                name: "Alex".into(),
                uuid: Some("01234567-89ab-cdef-0123-456789abcdef".into()),
                ..Default::default()
            }],
            name: "  ".into(),
            job: None,
            error: None,
        },
        selected: Some(0),
        joining: None,
        leaving: None,
        last_click: None,
        saves: None,
        news_link: None,
        news_job: None,
        news_unread: false,
        pending_invites: 0,
        pending_invites_job: None,
        join_width: std::cell::Cell::new(90.0),
        pdp_row: None,
        list_clicked: false,
        link_buttons: std::cell::Cell::new([(0.0, 0.0); 3]),
        terms_link: std::cell::Cell::new([0.0; 4]),
        notifications: Vec::new(),
        notifications_job: None,
        handled_seen: Vec::new(),
        dismiss_job: None,
        notice_rects: std::cell::Cell::new([[0.0; 4]; 2]),
        notice_height: std::cell::Cell::new(0.0),
        notice_buttons: std::cell::Cell::new([[0.0; 4]; 2]),
        popup_buttons: std::cell::Cell::new([[0.0; 4]; 2]),
        main_scroll: crate::ui::Scroll::default(),
        subscription_buttons: std::cell::Cell::new((0.0, 0.0)),
        players_button_rects: std::cell::RefCell::new(Vec::new()),
        backup_row_buttons: std::cell::RefCell::new(Vec::new()),
        settings_scroll: crate::ui::Scroll::default(),
        settings_slider: None,
        other_slider: None,
        after_save: None,
        redirect_to_pdp: false,
        template_link_rect: std::cell::Cell::new([0.0; 4]),
        trial: false,
        trial_job: None,
        live_players: std::collections::HashMap::new(),
        live_job: None,
        last_list_refresh: Instant::now(),
        last_pending_refresh: Instant::now(),
        last_news_refresh: Instant::now(),
        region_screen_requested: false,
        template_images: std::cell::RefCell::new(std::collections::HashMap::new()),
        world_icons: std::cell::RefCell::new(std::collections::HashMap::new()),
        skin_workers: SkinWorkers {
            profiles: std::sync::Arc::new(std::sync::Mutex::new(crate::skins::Resolver::new())),
            downloads: std::sync::Arc::new(std::sync::Mutex::new(crate::skins::Downloader::new(
                std::env::temp_dir().join("horizon-test-skins"),
            ))),
        },
        skin_faces: std::cell::RefCell::new(std::collections::HashMap::new()),
    };
    screen.invite_clicked();
    let SubScreen::Invite { error, job, .. } = &screen.subscreen else {
        panic!("still the invite screen");
    };
    assert!(job.is_none(), "a blank name asks the service nothing");
    assert_eq!(
        error.as_deref(),
        Some(crate::lang::get("mco.configure.world.players.error").as_str())
    );

    if let SubScreen::Invite { name, error, .. } = &mut screen.subscreen {
        *name = "ALEX".into();
        *error = None;
    }
    screen.invite_clicked();
    let SubScreen::Invite { error, job, .. } = &screen.subscreen else {
        panic!("still the invite screen");
    };
    assert!(job.is_none(), "a duplicate name asks the service nothing");
    assert_eq!(
        error.as_deref(),
        Some(crate::lang::get("mco.configure.world.players.invite.duplicate").as_str())
    );
}

#[test]
fn join_realm_button_navigates_to_join_code_screen() {
    let mut screen = listed(vec![]);
    let _ = screen.click(Some(14), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(screen.subscreen, SubScreen::JoinCode { .. }));
    let btns = screen.buttons(800.0, 600.0);
    let join_btn = btns.iter().find(|b| b.5 == 20).expect("Join button exists");
    assert!(!join_btn.4, "Join is off while the code is empty");
    assert!(!screen.escape());
    assert!(matches!(screen.subscreen, SubScreen::Main));
}

/// `RealmsConfigureWorldScreen`'s `MenuTabBar`: Worlds, Players, Subscription,
/// Settings, and an expired Realm's settings tab is off.
#[test]
fn configure_tabs_follow_the_vanilla_order_and_expiry() {
    assert_eq!(
        ConfigureTab::ALL,
        [
            ConfigureTab::Worlds,
            ConfigureTab::Players,
            ConfigureTab::Subscription,
            ConfigureTab::Settings,
        ]
    );
    for tab in ConfigureTab::ALL {
        assert!(tab.enabled(false), "{}", tab.title());
    }
    assert!(!ConfigureTab::Settings.enabled(true));
    for tab in [
        ConfigureTab::Worlds,
        ConfigureTab::Players,
        ConfigureTab::Subscription,
    ] {
        assert!(tab.enabled(true), "{}", tab.title());
    }
}

/// `MenuTabBar.builder(this, width)`: at most 400 wide, centred, four even tabs.
#[test]
fn the_tab_bar_is_centred_and_four_tabs_wide() {
    // 854x480 at GUI scale 2 is 427x240: 372 inner width, 28 from the left, 94 a tab.
    assert_eq!(tab_geometry(427.0), (28.0, 94.0));
    // A window narrower than 400 keeps its own width less the bar's margins:
    // 272 inner width over four tabs, 14 from the left.
    assert_eq!(tab_geometry(300.0), (14.0, 68.0));
}

/// `RealmsWorldsTab`'s four slot buttons: the active slot is 80 square, the others 50,
/// spaced 16 and centred (`GridLayout.spacing(16)`).
fn slot_boxes(w: f32, active_slot: i32) -> [(f32, f32, i32); 4] {
    let sizes: [f32; 4] = std::array::from_fn(|i| {
        if i as i32 + 1 == active_slot {
            80.0
        } else {
            50.0
        }
    });
    let total: f32 = sizes.iter().sum::<f32>() + 16.0 * 3.0;
    let mut x = ((w - total) / 2.0).floor();
    std::array::from_fn(|i| {
        let at = (x, sizes[i], i as i32 + 1);
        x += sizes[i] + 16.0;
        at
    })
}

/// `RealmsWorldOptions.getSlotName`: the player's own name, else the empty marker, else
/// "World N" by the slot's number.
fn slot_name(options: &RealmWorldOptions, slot: i32) -> String {
    if !options.slot_name.trim().is_empty() {
        return options.slot_name.clone();
    }
    if options.empty {
        return crate::lang::get("mco.configure.world.slot.empty");
    }
    crate::lang::tr("mco.configure.world.slot", &[&slot])
}

/// `InviteCodeExpiration.closestTo`: the choice nearest the time left on the code.
fn closest_expiration(expiration_millis: i64) -> usize {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let left = (expiration_millis - now).max(0) / 1000;
    let mut best = EXPIRATIONS.len() - 1;
    let mut best_delta = i64::MAX;
    for (i, (_, seconds)) in EXPIRATIONS.iter().enumerate() {
        let Some(seconds) = seconds else { continue };
        let delta = (left - seconds).abs();
        if delta < best_delta {
            best_delta = delta;
            best = i;
        }
    }
    best
}

/// `RealmsSubscriptionTab.daysLeftPresentation`, compared with the same translations
/// so the check reads in any language.
#[test]
fn days_left_says_what_vanilla_says() {
    let t = crate::lang::get;
    assert_eq!(
        days_left_text(-1, true),
        t("mco.configure.world.subscription.expired")
    );
    let less = t("mco.configure.world.subscription.less_than_a_day");
    assert_eq!(days_left_text(0, false), less);
    assert_eq!(days_left_text(1, false), less);
    assert_eq!(
        days_left_text(45, false),
        crate::lang::tr(
            "mco.configure.world.subscription.remaining.months.days",
            &[&1, &15]
        )
    );
    assert_eq!(
        days_left_text(60, false),
        crate::lang::tr("mco.configure.world.subscription.remaining.months", &[&2])
    );
    assert_eq!(
        days_left_text(5, false),
        crate::lang::tr("mco.configure.world.subscription.remaining.days", &[&5])
    );
    // An expired Realm still shows its remaining time while it has some.
    assert_eq!(
        days_left_text(60, true),
        crate::lang::tr("mco.configure.world.subscription.remaining.months", &[&2])
    );
}

/// An unreadable start date says nothing rather than a wrong date.
#[test]
fn an_unreadable_start_date_is_empty() {
    assert_eq!(start_date_text(i64::MAX), "");
}
/// `RealmsSettingsTab.save`: only a distance the player changed, and only when the tier
/// answer names the range, is sent.
#[test]
fn a_distance_is_saved_only_when_it_differs_from_the_service() {
    let range = RealmTierRange {
        min: 2,
        max: 16,
        default_value: 8,
        current: Some(10),
    };
    assert_eq!(changed_distance(Some(10), Some(&range)), None);
    assert_eq!(changed_distance(Some(12), Some(&range)), Some(12));
    // No tier answer: the slider was never usable, so nothing goes out.
    assert_eq!(changed_distance(Some(12), None), None);
    assert_eq!(changed_distance(None, Some(&range)), None);
    // A range without `current` compares against its default (`currentOrDefault`).
    let default = RealmTierRange {
        current: None,
        ..range
    };
    assert_eq!(changed_distance(Some(8), Some(&default)), None);
    assert_eq!(changed_distance(Some(9), Some(&default)), Some(9));
}

/// `ConfirmLinkScreen`: Open in Browser, Copy to Clipboard and Cancel, 100 wide each,
/// and every answer — Escape's no included — returns to the screen that asked.
#[test]
fn a_link_asks_three_ways_and_returns_to_its_parent() {
    let mut screen = listed(Vec::new());
    // The product page asks (its news button, `RealmsHeader.openNews`).
    screen.click(Some(13), 0.0, 0.0, (854.0, 480.0));
    let url = "https://realms.example/news".to_owned();
    screen.news_link = Some(url.clone());
    screen.click(Some(16), 0.0, 0.0, (854.0, 480.0));
    assert!(matches!(
        &screen.subscreen,
        SubScreen::Popup(Popup::OpenLink { .. })
    ));
    let answers: Vec<(u8, f32)> = screen
        .buttons(854.0, 480.0)
        .iter()
        .map(|b| (b.5, b.2))
        .collect();
    assert_eq!(answers, vec![(53, 100.0), (54, 100.0), (55, 100.0)]);
    // Cancel returns to the product page.
    screen.click(Some(55), 0.0, 0.0, (854.0, 480.0));
    assert!(screen.pdp_list());
    // Copy answers the link and returns there too.
    screen.click(Some(16), 0.0, 0.0, (854.0, 480.0));
    match screen.click(Some(54), 0.0, 0.0, (854.0, 480.0)) {
        Outcome::Copy(copied) => assert_eq!(copied, url),
        _ => panic!("the copy answer"),
    }
    assert!(screen.pdp_list());
    // So does Open.
    screen.click(Some(16), 0.0, 0.0, (854.0, 480.0));
    match screen.click(Some(53), 0.0, 0.0, (854.0, 480.0)) {
        Outcome::OpenUri(opened) => assert_eq!(opened, url),
        _ => panic!("the open answer"),
    }
    assert!(screen.pdp_list());
    // Escape is the no button.
    screen.click(Some(16), 0.0, 0.0, (854.0, 480.0));
    assert!(!screen.escape());
    assert!(screen.pdp_list());
}

/// `ObjectSelectionList.nextFocusPath`: the arrows move through the product page's
/// rows, the first one taking the end it heads to (`nextEntry`'s null start).
#[test]
fn the_pdp_rows_take_the_arrows_at_their_ends() {
    let mut screen = listed(Vec::new());
    screen.click(Some(13), 0.0, 0.0, (854.0, 480.0));
    assert!(screen.pdp_arrow(false));
    assert_eq!(screen.pdp_row, Some(0));
    assert!(screen.pdp_arrow(false));
    assert_eq!(screen.pdp_row, Some(1));
    assert!(screen.pdp_arrow(true));
    assert_eq!(screen.pdp_row, Some(0));
    // The ends hold.
    assert!(screen.pdp_arrow(true));
    assert_eq!(screen.pdp_row, Some(0));
    screen.pdp_row = None;
    assert!(screen.pdp_arrow(true));
    assert_eq!(screen.pdp_row, Some(pdp_entries().len() - 1));
    // Another screen has no product page.
    screen.subscreen = SubScreen::Main;
    assert!(!screen.pdp_arrow(false));
}

/// `RealmsPdpScreen.AbstractPdpEntry.getNarration`: the title and its descriptions,
/// the trial's only when the service says a trial is available (`PdpList`'s header).
#[test]
fn the_pdp_rows_narrate_their_lines() {
    let description = crate::lang::get("mco.pdp.friends.description");
    assert!(pdp_row_lines(1, false).contains(&description));
    let trial = crate::lang::get("mco.pdp.trial.description");
    assert!(!pdp_row_lines(0, false).contains(&trial));
    assert!(pdp_row_lines(0, true).contains(&trial));
}

/// `RealmsResetWorldScreen.init`'s grid: 60-wide tiles, 16 padding a side, the spacer row.
#[test]
fn the_reset_tiles_are_three_columns_with_vanillas_spacer() {
    // The 154-tall grid sits at min(63, h - 187): 56 belongs to a 243-tall screen.
    assert_eq!(reset_content_y(243.0), 56.0);
    assert_eq!(reset_content_y(480.0), 63.0);
    assert_eq!(reset_tile(200.0, 56.0, 0), (78.0, 56.0));
    assert_eq!(reset_tile(200.0, 56.0, 2), (262.0, 56.0));
    assert_eq!(reset_tile(200.0, 56.0, 3), (78.0, 141.0));
    assert_eq!(reset_tile(200.0, 56.0, 5), (262.0, 141.0));
}

#[test]
fn the_template_groups_are_vanillas_four_in_its_order() {
    let types: Vec<&str> = TemplateGroup::ALL.iter().map(|g| g.world_type()).collect();
    assert_eq!(
        types,
        ["NORMAL", "ADVENTUREMAP", "EXPERIENCE", "INSPIRATION"]
    );
    let titles: Vec<&str> = TemplateGroup::ALL.iter().map(|g| g.title()).collect();
    assert_eq!(
        titles,
        [
            "mco.reset.world.template",
            "mco.reset.world.adventure",
            "mco.reset.world.experience",
            "mco.reset.world.inspiration"
        ]
    );
}

/// `RealmsResetWorldScreen`'s Generate New World tile: `RealmsCreateWorldFlow` gets
/// the Realm, slot and reset kind to upload into.
#[test]
fn the_generate_tile_asks_for_the_create_world_screen() {
    for (size, (tx, ty)) in [
        ((854.0, 480.0), (305.0, 63.0)),
        ((400.0, 243.0), (78.0, 56.0)),
    ] {
        for kind in [ResetKind::Reset, ResetKind::Empty, ResetKind::NewRealm] {
            let creation = (kind == ResetKind::NewRealm)
                .then(|| ("Realm name".to_owned(), "Description".to_owned()));
            let mut screen = listed(vec![server(7, None)]);
            screen.subscreen = SubScreen::ResetWorld {
                server: server(7, None),
                slot: 2,
                kind,
                creation: creation.clone(),
                templates: None,
                job: None,
                reset: None,
                error: None,
            };
            for (hit, (x, y)) in [
                (Some(70), (0.0, 0.0)),
                (None, (tx, ty)),
                (None, (tx + 30.0, ty + 34.0)),
                (None, (tx + 59.5, ty + 68.5)),
            ] {
                match screen.click(hit, x, y, size) {
                    Outcome::CreateWorld(request) => {
                        assert_eq!((request.server.id, request.slot), (7, 2));
                        assert_eq!(request.kind, kind);
                        assert_eq!(request.creation, creation);
                    }
                    _ => panic!("the Generate New World tile asks for Create World"),
                }
            }
            for (dx, dy) in [(-1.0, 0.0), (60.0, 0.0), (0.0, -1.0), (0.0, 69.0)] {
                assert!(matches!(
                    screen.click(None, tx + dx, ty + dy, size),
                    Outcome::None
                ));
            }
            let abort = Arc::new(AtomicBool::new(false));
            if let SubScreen::ResetWorld { reset, .. } = &mut screen.subscreen {
                *reset = Some(RunningTask {
                    job: Arc::new(Mutex::new(None)),
                    title: Arc::new(Mutex::new(String::new())),
                    abort: Arc::clone(&abort),
                });
            }
            for hit in [
                None,
                Some(70),
                Some(71),
                Some(72),
                Some(73),
                Some(74),
                Some(75),
            ] {
                assert!(matches!(screen.click(hit, tx, ty, size), Outcome::None));
                assert!(matches!(
                    screen.subscreen,
                    SubScreen::ResetWorld { reset: Some(_), .. }
                ));
                assert!(!abort.load(Ordering::Relaxed));
            }
        }
    }
}

#[test]
fn reset_frames_register_focus_and_dispatch_through_the_title() {
    // Isolate the widget registry, narration and frame counter from other tests.
    std::thread::spawn(|| {
        use crate::gamepad::{Button, focus_step_rect};
        use crate::title::{Action, Mode, Title};
        use crate::widgets::{self, Kind};

        let resources = std::path::Path::new("/nonexistent/realms-reset-frame-assets");
        let mut atlas = HudAtlas::build(
            &horizon_assets::resource_source::ResourceSource::directory(resources.to_path_buf()),
        )
        .unwrap();
        atlas.sprites.insert(
            "gui/sprites/widget/slot_frame".into(),
            crate::hud::Rect {
                u0: 0.125,
                v0: 0.25,
                u1: 0.375,
                v1: 0.5,
                w: 60,
                h: 60,
            },
        );
        widgets::set_narrating(true);
        let mut frame = 0;
        let mut draw = |title: &Title, size: [u32; 2]| {
            // Seed at scale 1, then set logical GUI bounds without changing global GUI_SCALE.
            let mut batch = HudBatch::new(
                [1, 1],
                crate::hud::GuiScale {
                    requested: 1,
                    unicode: false,
                },
            );
            let logical = (size[0] as f32, size[1] as f32);
            (batch.width, batch.height) = logical;
            assert_eq!(batch.scale, 1.0);
            assert_eq!((batch.width, batch.height), logical);
            // Title.draw begins the narration screen and records the real controls.
            title.draw(&atlas, &mut batch, (-1.0, -1.0), true, &mut |_, _| {});
            frame += 1;
            widgets::end_frame(frame);
            batch
        };
        for (size, left, top) in [([854, 480], 305.0, 63.0), ([400, 243], 78.0, 56.0)] {
            let mut screen = listed(vec![server(7, None)]);
            screen.subscreen = SubScreen::ResetWorld {
                server: server(7, None),
                slot: 2,
                kind: ResetKind::Reset,
                creation: None,
                templates: None,
                job: None,
                reset: None,
                error: None,
            };
            let mut title = Title::new(PathBuf::new(), resources, "");
            title.mode = Mode::Realms(screen.clone());
            widgets::set_narration_focus(None);
            let plain = draw(&title, size);
            let (w, h) = (size[0] as f32, size[1] as f32);
            let expected = [
                [left, top, 60.0, 69.0],
                [left + 92.0, top, 60.0, 69.0],
                [left + 184.0, top, 60.0, 69.0],
                [left, top + 85.0, 60.0, 69.0],
                [left + 92.0, top + 85.0, 60.0, 69.0],
                [left + 184.0, top + 85.0, 60.0, 69.0],
                [w / 2.0 - 75.0, h - 27.0, 150.0, 20.0],
            ];
            let registered = widgets::last_labelled();
            let rects: Vec<_> = registered.iter().map(|b| [b.x, b.y, b.w, b.h]).collect();
            assert_eq!(
                rects.as_slice(),
                expected.as_slice(),
                "no 20px tile proxies"
            );
            for (b, key) in registered.iter().zip([
                "mco.reset.world.generate",
                "mco.upload.select.world.title",
                "mco.reset.world.template",
                "mco.reset.world.adventure",
                "mco.reset.world.experience",
                "mco.reset.world.inspiration",
                "gui.back",
            ]) {
                assert_eq!(b.label, crate::lang::get(key));
                assert!(b.enabled && b.kind == Kind::Button);
            }
            let targets = widgets::last_targets();
            assert_eq!(
                targets.iter().map(|t| t.rect).collect::<Vec<_>>(),
                expected[..6]
            );
            assert!(targets.iter().all(|t| t.kind == Kind::Button));
            assert_eq!(
                focus_step_rect([0.0, -1.0, w, 1.0], Button::DPadDown, &rects),
                Some(0)
            );
            assert_eq!(
                focus_step_rect([0.0, h, w, 1.0], Button::DPadUp, &rects),
                Some(6)
            );
            for (from, direction, to) in [
                (0, Button::DPadRight, 1),
                (1, Button::DPadRight, 2),
                (0, Button::DPadDown, 3),
                (1, Button::DPadDown, 4),
                (2, Button::DPadDown, 5),
                (3, Button::DPadRight, 4),
                (5, Button::DPadUp, 2),
                (4, Button::DPadDown, 6),
            ] {
                assert_eq!(focus_step_rect(rects[from], direction, &rects), Some(to));
            }
            let frame_color = |batch: &HudBatch| {
                batch
                    .vertices
                    .iter()
                    .find(|v| v.pos == [left, top] && v.uv == [0.125, 0.25])
                    .expect("the Generate frame was actually drawn")
                    .color
            };
            assert_eq!(frame_color(&plain), u32::MAX);
            widgets::set_narration_focus(Some(rects[0]));
            assert_eq!(frame_color(&draw(&title, size)), 0xFF8E_8E8E);
            assert!(widgets::take_last_narration().items[0].focused);
            let [x, y, bw, bh] = targets[0].rect;
            let center = (x + bw / 2.0, y + bh / 2.0);
            title.focused_button = Some(76);
            assert_eq!(
                title.click(center.0, center.1, (w, h)),
                (Action::None, true)
            );
            assert!(matches!(title.mode, Mode::Create { .. }));
            assert_eq!(title.focused_button, None);
            assert!(title.has_caret_field());
            let before = title.focused_field().unwrap().1.clone();
            title.type_text(" "); // The title's EditBox.charTyped/insertText path.
            assert_eq!(*title.focused_field().unwrap().1, format!("{before} "));

            let abort = Arc::new(AtomicBool::new(false));
            if let SubScreen::ResetWorld { reset, .. } = &mut screen.subscreen {
                *reset = Some(RunningTask {
                    job: Arc::new(Mutex::new(None)),
                    title: Arc::new(Mutex::new(String::new())),
                    abort: Arc::clone(&abort),
                });
            }
            title.mode = Mode::Realms(screen);
            draw(&title, size);
            assert!(widgets::last_targets().is_empty());
            let busy = widgets::last_labelled();
            assert_eq!(busy.len(), 1);
            assert_eq!(busy[0].label, crate::lang::get("gui.cancel"));
            assert_eq!(
                title.click(center.0, center.1, (w, h)),
                (Action::None, false)
            );
            assert!(matches!(
                title.mode,
                Mode::Realms(Screen {
                    subscreen: SubScreen::ResetWorld { reset: Some(_), .. },
                    ..
                })
            ));
            assert!(!abort.load(Ordering::Relaxed));
        }
    })
    .join()
    .unwrap();
}

/// `RealmsCreateWorldFlow.createTemporaryWorldFolder`: level.dat without
/// `singleplayer_uuid`, the world generation and game rules, and no player file or
/// weather/clocks (the Realm's server makes those on its own).
#[test]
fn a_generated_upload_world_carries_only_what_the_realm_needs() {
    let mut level = crate::save::Level {
        name: "New World".into(),
        seed: 4242,
        day_time: 0,
        health: 20.0,
        food: 20,
        saturation: 5.0,
        ..Default::default()
    };
    level.difficulty = "normal".into();
    level.enabled_features = vec!["minecraft:vanilla".into()].into();
    level.data_packs = Some((vec!["vanilla".into()], Vec::new()));
    let world = CreatedWorld {
        level,
        world_gen: crate::save::WorldGen::default(),
        flat_settings: None,
        dimensions: None,
        single_biome: None,
        data_packs: (vec!["vanilla".into()], Vec::new()),
        pack_dir: None,
    };
    let dir = create_temporary_world(&world).unwrap();
    let data = crate::save::read_dat(&dir.join("level.dat")).unwrap();
    let level = data.get("Data").expect("the level's data");
    assert_eq!(
        crate::save::text(level.get("LevelName")).as_deref(),
        Some("New World")
    );
    assert!(level.get("singleplayer_uuid").is_none());
    assert!(!dir.join("players").exists());
    assert!(dir.join("data/minecraft/world_gen_settings.dat").is_file());
    assert!(dir.join("data/minecraft/game_rules.dat").is_file());
    assert!(!dir.join("data/minecraft/weather.dat").exists());
    assert!(!dir.join("data/minecraft/world_clocks.dat").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `WorldTemplatePaginatedList.isLastPage`: page*size >= total, all three set.
#[test]
fn a_template_page_knows_when_it_is_the_last() {
    assert!(!last_template_page(1, 10, 11));
    assert!(last_template_page(2, 10, 11));
    assert!(last_template_page(1, 10, 10));
    assert!(!last_template_page(1, 10, 0));
    assert!(!last_template_page(0, 10, 10));
}

/// `RealmsWorldsTab.switchToMinigame`'s list returns to the Worlds tab when cancelled
/// (Back and Escape); a reset screen's template list still cancels back to the reset
/// screen.
#[test]
fn leaving_the_minigame_template_list_returns_to_the_worlds_tab() {
    let mut screen = listed(vec![server(7, None)]);
    let template_list = |group: Option<TemplateGroup>| SubScreen::TemplateList {
        server: server(7, None),
        slot: 1,
        kind: ResetKind::Reset,
        creation: None,
        group,
        templates: Vec::new(),
        selected: None,
        scroll: crate::ui::Scroll::default(),
        job: None,
        select: None,
        icons: std::cell::RefCell::new(Vec::new()),
        error: None,
    };
    screen.subscreen = template_list(None);
    let _ = screen.click(Some(78), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(
        &screen.subscreen,
        SubScreen::Configure {
            tab: ConfigureTab::Worlds,
            ..
        }
    ));
    screen.subscreen = template_list(None);
    assert!(!screen.escape());
    assert!(matches!(
        &screen.subscreen,
        SubScreen::Configure {
            tab: ConfigureTab::Worlds,
            ..
        }
    ));
    screen.subscreen = template_list(Some(TemplateGroup::ALL[0]));
    let _ = screen.click(Some(78), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(&screen.subscreen, SubScreen::ResetWorld { .. }));
}

fn backups_screen(server: RealmServer, list: Vec<Backup>) -> Screen {
    let mut screen = listed(vec![server.clone()]);
    screen.subscreen = SubScreen::Backups {
        server,
        slot_id: 1,
        backups: Some(Ok(list)),
        list_job: None,
        restore_job: None,
        download_task: None,
        selected: None,
        scroll: crate::ui::Scroll::default(),
    };
    screen
}

fn download_screen(size_job: Option<Job<u64>>) -> Screen {
    let mut screen = listed(vec![server(7, None)]);
    screen.subscreen = SubScreen::DownloadLatest {
        server: server(7, None),
        slot_id: 1,
        download: WorldDownload::default(),
        world_name: "Friends".into(),
        size_job,
        progress: Arc::new(download::DownloadProgress::default()),
        job: None,
        error: None,
        done: false,
        started: Instant::now(),
        speed: std::cell::Cell::new((0, Instant::now(), 0)),
    };
    screen
}

/// `RealmsUtil.convertToAgePresentationFromInstant`: the backup's age is the wall
/// clock's distance, not the animation clock's (`just now` for every row was wrong).
#[test]
fn a_backups_age_is_wall_clock_time() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert_eq!(
        age_presentation(now - 2 * 3_600_000),
        crate::lang::tr("mco.time.hoursAgo", &[&2i64 as &dyn std::fmt::Display])
    );
    assert_eq!(
        age_presentation(now - 3 * 86_400_000),
        crate::lang::tr("mco.time.daysAgo", &[&3i64 as &dyn std::fmt::Display])
    );
}

/// `RealmsBackupScreen.init`: Download Latest starts disabled and turns on with a
/// non-empty list; its click asks first (`downloadClicked`'s `infoPopupScreen`).
#[test]
fn the_download_button_needs_a_backup_and_asks_first() {
    let empty = backups_screen(server(7, None), Vec::new());
    let button = |screen: &Screen| {
        screen
            .buttons(800.0, 600.0)
            .into_iter()
            .find(|b| b.5 == 192)
            .expect("the download button")
    };
    assert!(!button(&empty).4);
    let mut full = backups_screen(server(7, None), vec![Backup::default()]);
    assert!(button(&full).4);
    // A click opens the info question over the list.
    full.click(Some(192), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(
        &full.subscreen,
        SubScreen::Popup(Popup::DownloadLatest { .. })
    ));
    // Cancel returns to a freshly read list.
    full.click(Some(51), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(
        &full.subscreen,
        SubScreen::Backups { backups: None, .. }
    ));
}

/// `checkDownloadSize`: a link of 5 GiB or more asks with
/// `warningAcknowledgePopupScreen` before any download starts.
#[test]
fn a_five_gigabyte_world_asks_before_downloading() {
    let answered: Job<u64> = Arc::new(Mutex::new(Some(download::SIZE_LIMIT)));
    let mut screen = download_screen(Some(answered));
    let _ = screen.poll();
    assert!(matches!(
        &screen.subscreen,
        SubScreen::Popup(Popup::DownloadOversized { .. })
    ));
    // OK starts the download; without a saves folder that fails at once.
    let _ = screen.click(Some(194), 0.0, 0.0, (800.0, 600.0));
    let SubScreen::DownloadLatest { error, done, .. } = &screen.subscreen else {
        panic!("the download screen");
    };
    assert!(*done);
    assert!(error.is_some());
}

/// `RealmsDownloadLatestWorldScreen`: the worker's answer turns Cancel into Done, and
/// Done returns to the backup list.
#[test]
fn a_finished_download_offers_done() {
    let mut screen = download_screen(None);
    if let SubScreen::DownloadLatest { job, .. } = &mut screen.subscreen {
        let answered: Job<Result<(), download::DownloadError>> = Arc::new(Mutex::new(Some(Ok(()))));
        *job = Some(answered);
    }
    let _ = screen.poll();
    let buttons = screen.buttons(800.0, 600.0);
    let done = buttons
        .iter()
        .find(|b| b.5 == 195)
        .expect("the footer button");
    assert_eq!(done.3, crate::lang::get("gui.done"));
    let _ = screen.click(Some(195), 0.0, 0.0, (800.0, 600.0));
    assert!(matches!(
        &screen.subscreen,
        SubScreen::Backups { backups: None, .. }
    ));
}

/// `RestoreTask.abortTask`: the task screen's Cancel stops the worker and returns to
/// the Worlds tab.
#[test]
fn cancelling_a_restore_aborts_its_worker() {
    let mut screen = backups_screen(server(7, None), Vec::new());
    let abort = Arc::new(AtomicBool::new(false));
    let job: Job<Result<(), RealmsError>> = Arc::new(Mutex::new(None));
    if let SubScreen::Backups { restore_job, .. } = &mut screen.subscreen {
        *restore_job = Some(RestoreTask {
            job,
            abort: Arc::clone(&abort),
        });
    }
    let _ = screen.click(Some(193), 0.0, 0.0, (800.0, 600.0));
    assert!(abort.load(Ordering::Relaxed));
    assert!(matches!(
        &screen.subscreen,
        SubScreen::Configure {
            tab: ConfigureTab::Worlds,
            ..
        }
    ));
}

/// Transfer percentages and byte sizes round decimal midpoints half-up like Java's
/// `String.format(Locale.ROOT, ...)`.
#[test]
fn transfer_captions_round_decimal_midpoints_half_up() {
    assert_eq!(transfer_percent(0.0125), "1.3");
    assert_eq!(human_readable(512), "512 B");
    assert_eq!(human_readable(2560), "3 KB");
    assert_eq!(human_readable(1_342_177_280), "1.3 GB");
}
