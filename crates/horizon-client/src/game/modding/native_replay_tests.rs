//! A client mod reads and drives replay playback, and the person's gameplay input stays off the
//! recording while it plays (#19247).
use super::native_replay::{
    FileIdentity, open_in_folder, open_recording, recording_path, recordings_folder, take_request,
};
use horizon_client::packet_source::PacketSource;
use horizon_client::studio::mcpr::TimestampedPacket;
use horizon_client::studio::{CameraKeyframe, CameraTrack, ReplayDriver};
use mod_api::{ClientHost, Hooks, ModEntry, ReplayError};
use mod_platform::testing::with_mods;
use std::any::Any;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A recording `/studio record` could have written: the Play login, then a packet at each of
/// `later` ms; its metadata says it lasts `duration` ms.
pub(super) fn write_recording(path: &Path, duration: u32, later: &[u32]) {
    write_recording_at(path, 0, duration, later);
}

/// As `write_recording`, with the Login stamped `login` ms in.
fn write_recording_at(path: &Path, login: u32, duration: u32, later: &[u32]) {
    use horizon_client::studio::{McprWriter, Metadata};
    let meta = Metadata {
        duration,
        ..Default::default()
    };
    let mut writer = McprWriter::new(path.to_path_buf(), meta).unwrap();
    write_packet(&mut writer, login, login_packet());
    for &at in later {
        writer.write_packet(at, 0x01, &[1]).unwrap();
    }
    writer.finish(duration).unwrap();
}

/// A recording of `packets`, each at its time, lasting `duration` ms.
fn write_packets(
    path: &Path,
    duration: u32,
    packets: Vec<(u32, horizon_protocol::packets::play::Clientbound)>,
) {
    use horizon_client::studio::{McprWriter, Metadata};
    let meta = Metadata {
        duration,
        ..Default::default()
    };
    let mut writer = McprWriter::new(path.to_path_buf(), meta).unwrap();
    for (at, packet) in packets {
        write_packet(&mut writer, at, packet);
    }
    writer.finish(duration).unwrap();
}

fn write_packet(
    writer: &mut horizon_client::studio::McprWriter,
    at: u32,
    packet: horizon_protocol::packets::play::Clientbound,
) {
    let encoded = packet.encode();
    let mut reader = horizon_protocol::buf::Reader::new(&encoded);
    let id = reader.varint().unwrap();
    writer.write_packet(at, id, reader.rest()).unwrap();
}

/// The Play login a recording starts with.
fn login_packet() -> horizon_protocol::packets::play::Clientbound {
    use horizon_protocol::packets::play;
    play::Clientbound::Login(play::Login {
        player_id: 1,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 8,
        simulation_distance: 8,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        online_mode: false,
        enforces_secure_chat: false,
    })
}

/// A system chat line saying `text`.
fn chat_packet(text: &str) -> horizon_protocol::packets::play::Clientbound {
    use horizon_protocol::packets::play;
    play::Clientbound::SystemChat(play::SystemChat {
        content: horizon_client::lan::data::component_nbt(&serde_json::json!({ "text": text })),
        overlay: false,
    })
}

/// A zip archive that is no recording: it passes the call's check and fails once read.
fn write_hollow_zip(path: &Path) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("readme.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, b"not a recording").unwrap();
    zip.finish().unwrap();
}

#[test]
fn a_recording_id_names_one_file_in_the_recordings_folder() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("recordings");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("my_trip_1.mcpr"), b"x").unwrap();
    std::fs::write(root.path().join("outside.mcpr"), b"x").unwrap();
    std::fs::create_dir(dir.join("folder.mcpr")).unwrap();

    assert_eq!(
        recording_path(&dir, "my_trip_1"),
        Ok(dir.join("my_trip_1.mcpr"))
    );
    let too_long = "a".repeat(251);
    // Refused as text, before any file is looked at.
    for id in [
        "",
        ".",
        "..",
        "../outside",
        "a/../my_trip_1",
        "sub/my_trip_1",
        "sub\\my_trip_1",
        "/etc/passwd",
        "\\\\server\\share\\x",
        "C:my_trip_1",
        "C:\\Windows\\x",
        "my_trip_1:stream",
        "my\0trip",
        "my\ntrip",
        "trip.",
        "trip ",
        "CON",
        "con",
        "Nul.txt",
        "aux.tar",
        "COM1",
        "lpt9.x",
        "CON .x",
        "COM\u{b9}",
        too_long.as_str(),
    ] {
        assert_eq!(
            recording_path(&dir, id),
            Err(ReplayError::InvalidId),
            "{id:?}"
        );
    }
    // Names that only look like devices, the longest id, a missing file, a folder, and the id
    // with its extension: no such recording.
    let longest = "a".repeat(250);
    for id in [
        "CONSOLE",
        "com10",
        "nul_trip",
        "a.CON",
        longest.as_str(),
        "nothing_here",
        "folder",
        "my_trip_1.mcpr",
    ] {
        assert_eq!(
            recording_path(&dir, id),
            Err(ReplayError::NotFound),
            "{id:?}"
        );
    }
}

#[test]
fn a_link_out_of_the_recordings_folder_is_no_recording() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("recordings");
    std::fs::create_dir(&dir).unwrap();
    let outside = root.path().join("secret.mcpr");
    std::fs::write(&outside, b"x").unwrap();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, dir.join("escape.mcpr"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, dir.join("escape.mcpr"));
    if let Err(error) = linked {
        // Windows makes links only with Developer Mode or the privilege.
        eprintln!("skipped: no symbolic link could be made: {error}");
        return;
    }
    assert_eq!(recording_path(&dir, "escape"), Err(ReplayError::NotFound));
}

/// A recordings folder that is itself a link is refused, though the recording it leads to is a
/// regular file directly in the folder it leads to.
#[test]
fn a_recordings_folder_that_is_a_link_holds_no_recording() {
    let root = tempfile::tempdir().unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("trip.mcpr"), b"PK\x03\x04").unwrap();
    let dir = root.path().join("recordings");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&elsewhere, &dir);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&elsewhere, &dir);
    if let Err(error) = linked {
        eprintln!("skipped: no symbolic link could be made: {error}");
        return;
    }
    assert_eq!(
        recording_path(&elsewhere, "trip"),
        Ok(elsewhere.join("trip.mcpr"))
    );
    assert_eq!(recording_path(&dir, "trip"), Err(ReplayError::NotFound));
    assert_eq!(
        open_recording(&dir, "trip").err(),
        Some(ReplayError::NotFound)
    );
}

/// The call opens the recording once; what is read later comes from that open file, even when
/// the name has since been given to another file.
#[test]
fn an_accepted_recording_is_read_from_the_file_opened_at_the_call() {
    let root = tempfile::tempdir().unwrap();
    write_recording(&root.path().join("trip.mcpr"), 1_000, &[]);
    std::fs::write(root.path().join("plain.mcpr"), b"not a zip").unwrap();
    assert_eq!(
        open_recording(root.path(), "plain").err(),
        Some(ReplayError::Unreadable)
    );

    let (path, file, identity) = open_recording(root.path(), "trip").unwrap();
    assert_eq!(FileIdentity::of(&file).unwrap(), identity);
    std::fs::rename(&path, root.path().join("moved.mcpr")).unwrap();
    std::fs::write(&path, b"swapped in after the check").unwrap();
    let driver = ReplayDriver::from_file(file, &path).unwrap();
    assert_eq!(driver.duration_ms(), 1_000);
    let mut swapped = String::new();
    std::fs::File::open(&path)
        .unwrap()
        .read_to_string(&mut swapped)
        .unwrap();
    assert_eq!(swapped, "swapped in after the check");
}

/// The recording is opened relative to the recordings folder's handle, not by its path. Where a
/// folder held open can be moved (not Windows), one moved away after it was opened, with a decoy
/// put at its old path, still yields the original; Windows refuses to move it at all while the
/// handle is held, so its path can't be redirected in the meantime.
#[test]
fn a_recording_is_opened_through_the_recordings_folders_handle() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("recordings");
    std::fs::create_dir(&dir).unwrap();
    write_recording(&dir.join("trip.mcpr"), 1_000, &[]);
    let original = FileIdentity::of(&std::fs::File::open(dir.join("trip.mcpr")).unwrap()).unwrap();

    let folder = recordings_folder(&dir).unwrap();
    let moved = root.path().join("moved");
    let renamed = std::fs::rename(&dir, &moved);
    #[cfg(windows)]
    assert!(
        renamed.is_err(),
        "Windows keeps a folder held open in place"
    );
    #[cfg(not(windows))]
    {
        renamed.expect("a folder held open can be moved");
        std::fs::create_dir(&dir).unwrap();
        write_hollow_zip(&dir.join("trip.mcpr"));
    }

    let (file, identity) = open_in_folder(&folder, "trip").unwrap();
    assert_eq!(
        identity, original,
        "the file in the folder opened, not the decoy"
    );
    assert_eq!(
        ReplayDriver::from_file(file, &dir.join("trip.mcpr"))
            .unwrap()
            .duration_ms(),
        1_000
    );
    assert_eq!(
        open_in_folder(&folder, "../x").err(),
        Some(ReplayError::InvalidId)
    );
    assert_eq!(
        open_in_folder(&folder, "nothing").err(),
        Some(ReplayError::NotFound)
    );
}

/// The clock ends at the recording's end, a packet past the recorded duration included; it never
/// reports past it; `ended` is true there however it got there and false anywhere before; and a
/// speed that is not a number changes nothing, whichever route sets it.
#[test]
fn a_replays_clock_stays_within_the_recording_and_says_when_it_ended() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("long.mcpr");
    write_recording(&path, 1_000, &[400, 1_500]);
    let mut driver = ReplayDriver::open(&path).unwrap();
    assert_eq!(
        driver.duration_ms(),
        1_500,
        "the last packet is past the metadata's"
    );

    let played = driver.advance(Duration::from_secs(5)).unwrap();
    assert_eq!(played.len(), 3, "every packet, the late one included");
    assert_eq!(driver.position_ms(), 1_500);
    assert!(driver.is_finished());
    assert!(driver.advance(Duration::from_secs(1)).unwrap().is_empty());
    assert_eq!(driver.position_ms(), 1_500);

    driver.seek_and_collect(400).unwrap();
    assert!(!driver.is_finished());
    driver.set_paused(true);
    driver.seek(1_500).unwrap();
    assert!(driver.is_finished(), "a paused seek to the end has ended");
    driver.seek(u32::MAX).unwrap();
    assert_eq!(driver.position_ms(), 1_500);
    assert!(driver.is_finished());
    driver.seek(0).unwrap();
    assert!(!driver.is_finished());

    driver.set_speed(2.0);
    driver.set_speed(f64::NAN);
    assert_eq!(
        driver.speed(),
        2.0,
        "NaN, as `/studio replay speed NaN` parses it"
    );
    for (speed, kept) in [
        (0.0, 0.01),
        (-3.0, 0.01),
        (1_000.0, 20.0),
        (f64::INFINITY, 20.0),
    ] {
        driver.set_speed(speed);
        assert_eq!(driver.speed(), kept, "the speed range holds for {speed}");
    }
}

/// A packet source standing in for a recording: its clock moves by the frame's time at its
/// speed unless paused, and it carries no packets.
struct Clock {
    position: f64,
    duration: u32,
    speed: f64,
    paused: bool,
}

impl Clock {
    fn new(duration: u32) -> Self {
        Self {
            position: 0.0,
            duration,
            speed: 1.0,
            paused: false,
        }
    }
}

impl PacketSource for Clock {
    fn position_ms(&self) -> u32 {
        self.position as u32
    }
    fn duration_ms(&self) -> u32 {
        self.duration
    }
    fn packet_count(&self) -> usize {
        0
    }
    fn progress(&self) -> f32 {
        (self.position / f64::from(self.duration)) as f32
    }
    fn speed(&self) -> f64 {
        self.speed
    }
    fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }
    fn is_paused(&self) -> bool {
        self.paused
    }
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }
    fn set_looping(&mut self, _: bool) {}
    fn is_finished(&self) -> bool {
        self.position_ms() >= self.duration
    }
    fn advance(&mut self, delta: Duration) -> io::Result<Vec<TimestampedPacket>> {
        if !self.paused {
            self.position = (self.position + delta.as_secs_f64() * 1000.0 * self.speed)
                .min(f64::from(self.duration));
        }
        Ok(Vec::new())
    }
    fn seek(&mut self, target_ms: u32) -> io::Result<()> {
        self.position = f64::from(target_ms.min(self.duration));
        Ok(())
    }
    fn seek_and_collect(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>> {
        self.seek(target_ms)?;
        Ok(Vec::new())
    }
    fn seek_forward(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>> {
        self.seek(target_ms)?;
        Ok(Vec::new())
    }
    fn restart(&mut self) -> io::Result<()> {
        self.seek(0)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// What a replay editor mod hears.
struct Heard(Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn replay_opened(&self, host: &mut dyn ClientHost, event: &mut mod_api::ReplayOpenedEvent<'_>) {
        let line = format!(
            "opened {} {} {} #{}",
            event.recording, event.duration_ms, event.standalone, event.request
        );
        self.0.lock().unwrap().push(line);
        // The recording's Login makes its player the local one; a replay heard open before
        // that is said so, and no test expects it.
        if host.local_player().is_none() {
            self.0
                .lock()
                .unwrap()
                .push("opened before its Login".into());
        }
    }
    fn replay_open_failed(
        &self,
        _: &mut dyn ClientHost,
        event: &mut mod_api::ReplayOpenFailedEvent<'_>,
    ) {
        let line = format!(
            "failed {} {:?} #{}",
            event.recording, event.error, event.request
        );
        self.0.lock().unwrap().push(line);
    }
    fn replay_advanced(&self, _: &mut dyn ClientHost, event: &mut mod_api::ReplayAdvancedEvent) {
        let line = format!("advanced {}", event.position_ms);
        self.0.lock().unwrap().push(line);
    }
    fn replay_closed(&self, _: &mut dyn ClientHost, event: &mut mod_api::ReplayClosedEvent<'_>) {
        let line = format!("closed {}", event.recording);
        self.0.lock().unwrap().push(line);
    }
    fn mouse_scrolling(
        &self,
        _: &mut dyn ClientHost,
        _: &mut mod_api::MouseScrollingEvent,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().push("wheel".into());
        mod_api::Flow::Continue
    }
}

fn with_heard(test: impl FnOnce(&mut crate::game::Game<'_>)) -> Vec<String> {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods =
        vec![ModEntry::new("editor", Box::new(Heard(Arc::clone(&heard))))].into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(test);
    });
    heard.lock().unwrap().clone()
}

/// A replay on a session it doesn't own, as a stand-in recording.
fn play(game: &mut crate::game::Game<'_>, duration: u32) {
    game.replay.driver = Some(Box::new(Clock::new(duration)));
    game.replay.recording = "demo".into();
    game.replay.generation += 1;
    game.replay.standalone = false;
    game.replay.reported_ms = Some(0);
    game.replay.connection = game.replay.connections;
}

/// A frame `ms` after the replay's last one.
fn frame(game: &mut crate::game::Game<'_>, ms: u64) {
    // One time sample: the frame lasts exactly `ms`.
    let now = std::time::Instant::now();
    game.replay.last_tick = Some(now - Duration::from_millis(ms));
    game.advance_studio_replay_at(now).unwrap();
}

/// Frame ends until the accepted open's recording is read and taken up.
fn finish_opening(game: &mut crate::game::Game<'_>) {
    let started = std::time::Instant::now();
    while game.replay.opening.is_some() || game.replay.queued.is_some() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "reading never ended"
        );
        std::thread::sleep(Duration::from_millis(5));
        game.native_replay_requests();
    }
}

#[test]
#[ignore = "daily"]
fn a_rebuilt_replay_player_model_ages_once_per_tick() {
    with_heard(|game| {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;

        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.clock.set_frozen(false);
        assert!(game.player_model.is_none());
        game.tick_replay_player_model();
        assert_eq!(game.player_model.as_ref().unwrap().age, 1);
        game.tick_replay_player_model();
        assert_eq!(game.player_model.as_ref().unwrap().age, 2);
    });
}

/// Pause, resume, speed and the state read act at once; seek and close wait for the frame's end.
/// The clock's moves are heard once a frame they happen, never on a paused frame; after a seek
/// the next one is exactly the seek's position, then the clock runs on.
#[test]
#[ignore = "daily"]
fn a_client_mod_drives_the_replay_and_hears_its_clock_once_a_move() {
    let heard = with_heard(|game| {
        let host: &mut dyn ClientHost = game;
        assert_eq!(host.replay_state(), None);
        assert_eq!(host.replay_pause(), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_resume(), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_set_speed(1.0), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_seek(0), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_close(), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_open("../x"), Err(ReplayError::InvalidId));

        play(game, 10_000);
        let host: &mut dyn ClientHost = game;
        let state = host.replay_state().unwrap();
        assert_eq!(
            (
                state.recording.as_str(),
                state.standalone,
                state.duration_ms
            ),
            ("demo", false, 10_000)
        );
        for speed in [f64::NAN, f64::INFINITY, 0.0, -1.0, 20.5] {
            assert_eq!(host.replay_set_speed(speed), Err(ReplayError::InvalidSpeed));
        }
        assert_eq!(host.replay_state().unwrap().speed, 1.0);
        assert_eq!(host.replay_set_speed(2.0), Ok(()));
        assert_eq!(host.replay_pause(), Ok(()));
        assert!(host.replay_state().unwrap().paused);

        frame(game, 100);
        frame(game, 100);
        assert_eq!(game.replay_resume(), Ok(()));
        // 100 ms at twice the speed.
        frame(game, 100);
        assert_eq!(game.replay_state().unwrap().position_ms, 200);

        assert_eq!(game.replay_seek(5_000), Ok(()));
        assert_eq!(game.replay_state().unwrap().position_ms, 200, "not yet");
        game.native_replay_requests();
        assert_eq!(game.replay_state().unwrap().position_ms, 5_000);
        assert_eq!(game.replay_seek(100), Err(ReplayError::BackwardSeek));
        // Playing on: the frame drawn after the seek holds its position, the next one runs.
        frame(game, 100);
        assert_eq!(game.replay_state().unwrap().position_ms, 5_000);
        frame(game, 100);
        assert_eq!(game.replay_state().unwrap().position_ms, 5_200);
        // A seek to where the clock is, paused, is told once too.
        game.replay_pause().unwrap();
        game.replay_seek(5_200).unwrap();
        game.native_replay_requests();
        frame(game, 100);
        frame(game, 100);

        // A close asked of a replay that is replaced before the frame ends reaches nothing.
        game.replay_close().unwrap();
        game.replay.generation += 1;
        game.native_replay_requests();
        assert!(game.replay.playing());

        // A recorded dimension change renews the world's epoch, not the replay: a seek and a
        // close asked before it still run.
        game.replay_seek(6_000).unwrap();
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        game.native_replay_requests();
        assert_eq!(game.replay_state().unwrap().position_ms, 6_000);
        game.replay_close().unwrap();
        assert!(game.replay.playing(), "open until the frame ends");
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        game.native_replay_requests();
        assert_eq!(game.replay_state(), None);
    });
    assert_eq!(
        heard,
        [
            "advanced 200",
            "advanced 5000",
            "advanced 5200",
            "advanced 5200",
            "closed demo"
        ]
    );
}

/// While a replay plays with no free camera (a mod's camera draws it), the person's movement
/// keys, the wheel's and a bumper's hotbar step and the focus pause stay off the recording; a mod
/// still hears the wheel and the bumper, and a screen or prompt open over the replay takes the
/// wheel first: the mod hears no hotbar scroll then.
#[test]
#[ignore = "daily"]
fn a_replay_keeps_gameplay_input_off_and_leaves_mods_the_wheel() {
    use horizon_client::held::Source;
    use winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
    use winit::keyboard::KeyCode;
    struct Loop;
    impl crate::game::event_loop::EventLoopCtl for Loop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
    }
    let wheel = || WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::LineDelta(0.0, -1.0),
        phase: TouchPhase::Moved,
    };
    let heard = with_heard(|game| {
        game.title = None;
        game.screen = crate::game::Screen::Playing;
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            )
            .unwrap(),
        );
        game.focus_lost = Some(std::time::Instant::now() - Duration::from_secs(1));
        game.pause_if_inactive();
        assert_eq!(game.screen, crate::game::Screen::Paused, "without a replay");
        game.screen = crate::game::Screen::Playing;
        game.mapped_key(KeyCode::KeyW, true, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        assert!(game.keys.raw(&KeyCode::KeyW), "walking without a replay");
        game.mapped_key(KeyCode::KeyW, false, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        game.selected = 0;
        game.handle_window_event_inner(&Loop, wheel());
        assert_eq!(
            game.selected, 1,
            "the wheel steps the hotbar without a replay"
        );

        play(game, 10_000);
        assert!(!game.studio.flying());
        game.mapped_key(KeyCode::KeyW, true, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        assert!(!game.keys.raw(&KeyCode::KeyW), "no walking in a replay");
        game.pause_if_inactive();
        assert_eq!(game.screen, crate::game::Screen::Playing, "no focus pause");
        game.handle_window_event_inner(&Loop, wheel());
        game.hotbar_step(true);
        assert_eq!(game.selected, 1, "neither the wheel nor a bumper steps it");
        game.screen = crate::game::Screen::Chat;
        game.handle_window_event_inner(&Loop, wheel());
        assert_eq!(game.selected, 1, "a screen over the replay takes the wheel");
        game.screen = crate::game::Screen::Playing;
        game.link_prompt = Some("https://example.invalid".into());
        game.handle_window_event_inner(&Loop, wheel());
        assert_eq!(game.selected, 1, "a prompt over the replay takes the wheel");
        game.link_prompt = None;
    });
    assert_eq!(heard, ["wheel", "wheel", "wheel"]);
}

/// A mod opens recordings by id from the title: the call accepts with a request id, the
/// recording is read off the game's thread and opened at a frame's end, and every accepted
/// request ends in exactly one `replay_opened` or `replay_open_failed` with its id. One thread
/// reads at a time: an open asked while another waits supersedes it, and only the later one
/// opens. A replay opened any other way is heard with request 0.
#[test]
#[ignore = "daily"]
fn a_client_mod_opens_recordings_by_id_at_the_frames_end() {
    let root = tempfile::tempdir().unwrap();
    write_packets(
        &root.path().join("trip.mcpr"),
        1_000,
        vec![(0, login_packet()), (0, chat_packet("said in trip"))],
    );
    let said_in_trip = |game: &crate::game::Game<'_>| {
        game.ui
            .chat
            .recent_lines(10)
            .iter()
            .any(|line| line == "said in trip")
    };
    write_recording(&root.path().join("trip2.mcpr"), 2_000, &[]);
    std::fs::write(root.path().join("damaged.mcpr"), b"not a zip").unwrap();
    write_hollow_zip(&root.path().join("hollow.mcpr"));
    let mut ids = Vec::new();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let host: &mut dyn ClientHost = game;
        assert_eq!(host.replay_open("damaged"), Err(ReplayError::Unreadable));
        assert_eq!(host.replay_open("missing"), Err(ReplayError::NotFound));
        let hollow = host.replay_open("hollow").unwrap();
        finish_opening(game);
        assert_eq!(game.replay_state(), None);

        let trip = game.replay_open("trip").unwrap();
        assert_eq!(game.replay_state(), None, "opens at a frame's end");
        finish_opening(game);
        let state = game.replay_state().expect("open");
        assert_eq!(
            (
                state.recording.as_str(),
                state.duration_ms,
                state.standalone
            ),
            ("trip", 1_000, true)
        );
        assert!(said_in_trip(game));

        // Two opens before the frame's end: one reading thread at most, the first superseded.
        let first = game.replay_open("trip2").unwrap();
        let second = game.replay_open("trip2").unwrap();
        assert!(game.replay.opening.is_some() && game.replay.queued.is_some());
        // A recorded dimension change renews the world's epoch: the open asked of this replay
        // still stands.
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        finish_opening(game);
        assert_eq!(game.replay_state().unwrap().recording, "trip2");
        assert!(
            !said_in_trip(game),
            "the replaced replay's chat went with it"
        );
        game.replay_close().unwrap();
        game.native_replay_requests();
        assert_eq!(game.replay_state(), None);

        let path = root.path().join("trip.mcpr");
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        game.replay_close().unwrap();
        game.native_replay_requests();
        ids = vec![hollow, trip, first, second];
    });
    let [hollow, trip, first, second] = ids[..] else {
        panic!("the test ended early");
    };
    assert!(hollow != 0 && trip != 0 && first != 0 && second != 0);
    assert!(
        hollow < trip && trip < first && first < second,
        "never given twice"
    );
    assert_eq!(
        heard,
        [
            format!("failed hollow Unreadable #{hollow}"),
            format!("opened trip 1000 true #{trip}"),
            format!("failed trip2 Superseded #{first}"),
            "closed trip".into(),
            format!("opened trip2 2000 true #{second}"),
            "closed trip2".into(),
            "opened trip 1000 true #0".into(),
            "closed trip".into(),
        ]
    );
}

/// The editor saves a mod's replay into the very archive it opened: two saves in a row both
/// land, and once another file is put at its path, the save is refused and that file untouched.
#[test]
#[ignore = "daily"]
fn the_editor_saves_a_mods_replay_only_into_the_archive_it_opened() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    write_recording(&path, 1_000, &[]);
    with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        game.replay_open("trip").unwrap();
        finish_opening(game);
        let markers = |game: &mut crate::game::Game<'_>| {
            game.replay
                .driver
                .as_mut()
                .and_then(|driver| driver.as_any_mut().downcast_mut::<ReplayDriver>())
                .map(|driver| driver.markers().len())
        };
        game.add_studio_marker();
        game.add_studio_marker();
        assert_eq!(markers(game), Some(2), "a save, then one after it");
        assert_eq!(
            ReplayDriver::open(&path).unwrap().markers().len(),
            2,
            "both in the archive"
        );

        std::fs::rename(&path, root.path().join("kept.mcpr")).unwrap();
        write_recording(&path, 3_000, &[]);
        game.add_studio_marker();
        assert_eq!(markers(game), Some(2), "refused");
        let other = ReplayDriver::open(&path).unwrap();
        assert_eq!((other.duration_ms(), other.markers().len()), (3_000, 0));
        game.replay_close().unwrap();
        game.native_replay_requests();
    });
}

/// A replay never displaces a session others share or one being joined: refused at the call
/// (a world entry waiting, a world open to LAN, splitscreen), and an accepted request whose
/// session changed before it was ready, or whose read replay lost its place as the world entry,
/// fails, opening nothing.
#[test]
#[ignore = "daily"]
fn a_replay_open_never_displaces_a_joining_or_multiplayer_session() {
    let root = tempfile::tempdir().unwrap();
    write_recording(&root.path().join("trip.mcpr"), 1_000, &[]);
    let mut ids = Vec::new();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        game.pending_entry = Some(crate::game::PendingWorldEntry::StudioReplay(
            root.path().join("trip.mcpr"),
        ));
        assert_eq!(game.replay_open("trip"), Err(ReplayError::WorldBusy));
        game.displace_pending_replay(ReplayError::Unavailable);
        assert!(
            game.pending_entry.is_some(),
            "only a mod's read replay is displaced"
        );
        // A read replay waiting as the world entry and then cancelled (Escape) or replaced is
        // told as failed, never dropped unheard.
        let driver = ReplayDriver::open(&root.path().join("trip.mcpr")).unwrap();
        game.pending_entry = Some(crate::game::PendingWorldEntry::PreparedReplay {
            driver: Box::new(driver),
            origin: super::native_replay::ReplayOrigin {
                recording: "trip".into(),
                request: 7,
                archive: None,
                identity: None,
                replacing: None,
            },
        });
        game.displace_pending_replay(ReplayError::Unavailable);
        assert!(game.pending_entry.is_none());

        game.split_window = true;
        assert_eq!(
            game.replay_open("trip"),
            Err(ReplayError::WorldBusy),
            "splitscreen"
        );
        game.split_window = false;

        // A world open to LAN, with nothing else that would refuse the open.
        crate::game::native_item_places::tests::open_to_lan(game);
        game.options.dedicated = None;
        assert!(game.remote.is_none() && game.integrated.is_none() && game.pending_entry.is_none());
        assert!(game.is_published());
        assert_eq!(game.replay_open("trip"), Err(ReplayError::WorldBusy), "LAN");
        game.lan = None;

        // Accepted on the title, then a server is joined before the recording is ready.
        ids.push(game.replay_open("trip").unwrap());
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        finish_opening(game);
        assert!(game.remote.is_some(), "the joined session stays");
        assert_eq!(game.replay_state(), None);
        assert_eq!(game.replay_open("trip"), Err(ReplayError::WorldBusy));
    });
    let [joined] = ids[..] else {
        panic!("the test ended early");
    };
    assert_eq!(
        heard,
        [
            "failed trip Unavailable #7".to_owned(),
            format!("failed trip WorldBusy #{joined}"),
        ]
    );
}

/// The control socket's seek goes the way chat's, Studio's and mods' do: to the end, the world
/// is brought there and the clock reports exactly that, ended.
#[test]
#[ignore = "daily"]
fn a_control_socket_seek_to_the_end_brings_the_world_there() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    write_packets(
        &path,
        1_000,
        vec![(0, login_packet()), (400, chat_packet("said at 400"))],
    );
    let said = |game: &crate::game::Game<'_>| {
        game.ui
            .chat
            .recent_lines(10)
            .iter()
            .any(|line| line == "said at 400")
    };
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        assert!(!said(game), "not yet played");
        let answer = game
            .studio_control(&["studio", "replay", "seek", "1000"])
            .unwrap()
            .unwrap();
        assert_eq!(answer["position_ms"], 1_000);
        let state = game.replay_state().unwrap();
        assert_eq!((state.position_ms, state.ended), (1_000, true));
        assert!(
            said(game),
            "the world at the end holds what was said before it"
        );
        assert!(
            game.replay.seek_report,
            "reported as a seek, on the next frame"
        );
        frame(game, 100);
        game.replay_close().unwrap();
        game.native_replay_requests();
    });
    assert_eq!(
        heard,
        ["opened trip 1000 true #0", "advanced 1000", "closed trip"]
    );
}

/// A recording 0 ms long still hands out its snapshot after a restart: its clock is at the end
/// at once, and the packets stamped there come out on the next frame, once.
#[test]
fn a_zero_length_recording_plays_its_snapshot_after_a_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("still.mcpr");
    write_packets(
        &path,
        0,
        vec![(0, login_packet()), (0, chat_packet("said at 0"))],
    );
    let mut driver = ReplayDriver::open(&path).unwrap();
    assert_eq!(driver.duration_ms(), 0);
    assert_eq!(driver.advance(Duration::from_millis(16)).unwrap().len(), 2);
    assert!(driver.is_finished());
    driver.restart().unwrap();
    assert!(driver.is_finished(), "the clock is at the end");
    assert_eq!(driver.advance(Duration::from_millis(16)).unwrap().len(), 2);
    assert!(
        driver
            .advance(Duration::from_millis(16))
            .unwrap()
            .is_empty()
    );
    assert!(driver.is_finished());
}

/// A seek back in a standalone replay shows the recording's state there, outside its session
/// too: a world border a later packet set is gone again.
#[test]
#[ignore = "daily"]
fn a_backward_seek_drops_what_later_packets_set_outside_the_session() {
    use horizon_protocol::packets::play;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("border.mcpr");
    write_packets(
        &path,
        1_000,
        vec![
            (0, login_packet()),
            (
                500,
                play::Clientbound::SetBorderSize(play::SetBorderSize { size: 100.0 }),
            ),
        ],
    );
    with_heard(|game| {
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        let fresh = game.border.clone();
        game.seek_studio_replay(800).unwrap();
        assert_eq!(game.border.size(), 100.0, "the border packet was applied");
        game.seek_studio_replay(200).unwrap();
        assert_eq!(game.replay_state().unwrap().position_ms, 200);
        assert_eq!(
            game.border, fresh,
            "back before it, the border is as at the open"
        );
        assert_eq!(
            game.border,
            horizon_client::worldborder::WorldBorder::default()
        );
        game.replay_close().unwrap();
        game.native_replay_requests();
    });
}

/// A replay ends with the session it plays on: leaving it (as the control socket's
/// `disconnect` does) closes the replay once, and a replay whose session was replaced by
/// another is no replay playing, so an open meets the ordinary guards.
#[test]
#[ignore = "daily"]
fn a_replay_ends_with_the_session_it_plays_on() {
    let root = tempfile::tempdir().unwrap();
    write_recording(&root.path().join("trip.mcpr"), 1_000, &[]);
    let mut ids = Vec::new();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let first = game.replay_open("trip").unwrap();
        finish_opening(game);
        assert!(game.replay_state().is_some());
        game.leave_server(None).unwrap();
        assert_eq!(game.replay_state(), None);
        assert!(game.remote.is_none(), "the replay's session went with it");

        // From the title, an open is admitted as ever.
        let second = game.replay_open("trip").unwrap();
        finish_opening(game);
        assert!(game.replay_state().is_some());
        // Left with a reason (a mod's `disconnect`): the replay closes once, and the
        // disconnect screen says the reason, as without a replay.
        let host: &mut dyn ClientHost = game;
        host.disconnect(Some(mod_api::Text::literal("kicked")))
            .unwrap();
        game.native_leave_if_asked().unwrap();
        assert_eq!(game.replay_state(), None);
        assert!(game.remote.is_none());
        let mode = &game.title.as_ref().expect("the title screen").mode;
        assert!(
            matches!(
                mode,
                horizon_client::title::Mode::Multiplayer(
                    horizon_client::multiplayer::Screen::Disconnected { reason, .. }
                ) if reason.plain() == "kicked"
            ),
            "the disconnect screen with the reason"
        );

        let third = game.replay_open("trip").unwrap();
        finish_opening(game);
        assert!(game.replay_state().is_some());
        // Another connection takes the session's place under the replay: the replay is
        // dropped, and the session there now is one a replay never displaces.
        game.replay.connections += 1;
        assert_eq!(game.replay_open("trip"), Err(ReplayError::WorldBusy));
        assert_eq!(game.replay_state(), None);
        assert!(game.remote.is_some(), "the session there now is left alone");
        ids = vec![first, second, third];
    });
    let [first, second, third] = ids[..] else {
        panic!("the test ended early");
    };
    assert_eq!(
        heard,
        [
            format!("opened trip 1000 true #{first}"),
            "closed trip".into(),
            format!("opened trip 1000 true #{second}"),
            "closed trip".into(),
            format!("opened trip 1000 true #{third}"),
            "closed trip".into(),
        ]
    );
}

/// A recorded packet that makes the client leave its session (a chat deletion naming no
/// message it holds) ends the replay where it plays: closed once, the rest of the batch not
/// applied, and nothing heard after.
#[test]
#[ignore = "daily"]
fn a_recorded_packet_that_leaves_the_session_ends_the_replay_once() {
    use horizon_protocol::packets::play;
    let root = tempfile::tempdir().unwrap();
    write_packets(
        &root.path().join("kick.mcpr"),
        1_000,
        vec![
            (0, login_packet()),
            (
                500,
                play::Clientbound::DeleteChat(play::DeleteChat {
                    signature: (5, None),
                }),
            ),
            (500, chat_packet("said after the kick")),
        ],
    );
    let mut request = 0;
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        request = game.replay_open("kick").unwrap();
        finish_opening(game);
        frame(game, 250);
        frame(game, 250);
        assert_eq!(game.replay_state(), None);
        frame(game, 250);
        assert!(
            !game
                .ui
                .chat
                .recent_lines(10)
                .iter()
                .any(|line| line == "said after the kick"),
            "the rest of the batch went nowhere"
        );
    });
    assert_eq!(
        heard,
        [
            format!("opened kick 1000 true #{request}"),
            // The open's own first frame ran 16 ms.
            "advanced 266".into(),
            "closed kick".into(),
        ]
    );
}

/// Request ids run out rather than wrap: the last `u32` is given once, then none, never the
/// reserved 0 nor an id given before.
#[test]
fn request_ids_run_out_instead_of_wrapping() {
    let next = std::sync::atomic::AtomicU32::new(u32::MAX - 1);
    assert_eq!(take_request(&next), Some(u32::MAX - 1));
    assert_eq!(take_request(&next), Some(u32::MAX));
    assert_eq!(take_request(&next), None);
    assert_eq!(take_request(&next), None);
}

/// A recording whose Login is stamped after 0 ms still sets up its first frame from it: the
/// driver hands out the opening snapshot without moving the clock, and a world rebuilt from the
/// start gets it too.
#[test]
fn a_late_login_is_the_opening_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("late.mcpr");
    write_recording_at(&path, 100, 1_000, &[100, 500]);
    let mut driver = ReplayDriver::open(&path).unwrap();
    let opening = driver.take_opening().unwrap();
    assert_eq!(opening.len(), 2, "the Login and the packet stamped with it");
    assert_eq!(driver.position_ms(), 0, "the clock stays");
    assert!(driver.take_opening().unwrap().is_empty(), "handed out once");
    assert_eq!(driver.seek_and_collect(0).unwrap().len(), 2);
    assert_eq!(driver.position_ms(), 0);
}

/// A replay whose Login is stamped 100 ms in is set up from it before `replay_opened` is heard,
/// while the clock is still before 100 ms.
#[test]
#[ignore = "daily"]
fn a_replay_with_a_late_login_is_set_up_before_it_is_heard_open() {
    let root = tempfile::tempdir().unwrap();
    write_recording_at(&root.path().join("late.mcpr"), 100, 1_000, &[]);
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let request = game.replay_open("late").unwrap();
        finish_opening(game);
        let remote = game.remote.as_ref().expect("the replay's session");
        assert!(remote.logged_in, "the Login was applied");
        assert_eq!(remote.entity_id, 1);
        assert!(game.replay_state().unwrap().position_ms < 100);
        assert!(request != 0);
        game.replay_close().unwrap();
        game.native_replay_requests();
    });
    assert_eq!(heard.len(), 2, "{heard:?}");
    assert!(heard[0].starts_with("opened late 1000 true #"));
    assert_eq!(
        heard[1], "closed late",
        "the hook already saw the recording's player"
    );
}

/// A replacement never costs the camera path edited in the replay playing: when it can't be
/// saved, or the replacement was read from the archive it was just saved into, the replay
/// playing stays and the request fails with `Unavailable`.
#[test]
#[ignore = "daily"]
fn a_replacement_keeps_the_replay_whose_camera_edits_it_would_lose() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    write_recording(&path, 1_000, &[]);
    write_recording(&root.path().join("trip2.mcpr"), 2_000, &[]);
    let mut ids = Vec::new();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let trip = game.replay_open("trip").unwrap();
        finish_opening(game);

        // Edited, then the same archive asked for again: saved, and the copy read before the
        // save is refused.
        let first = CameraKeyframe::new(250, [1.0, 2.0, 3.0], 10.0, 5.0, 70.0);
        game.studio.freecam.track.insert(first.clone());
        game.studio.camera_track_dirty = true;
        let again = game.replay_open("trip").unwrap();
        finish_opening(game);
        assert!(!game.studio.camera_track_dirty, "the edits were saved");
        assert_eq!(game.replay_state().unwrap().recording, "trip");
        let saved = ReplayDriver::open(&path).unwrap();
        let saved: CameraTrack =
            serde_json::from_slice(saved.camera_path().expect("a camera path")).unwrap();
        assert_eq!(
            saved.keyframes(),
            std::slice::from_ref(&first),
            "the archive holds the edit"
        );

        // Edited again, with the archive replaced so the save is refused: another recording
        // doesn't take the replay's place.
        let second = CameraKeyframe::new(750, [4.0, 5.0, 6.0], 20.0, 0.0, 70.0);
        game.studio.freecam.track.insert(second.clone());
        game.studio.camera_track_dirty = true;
        std::fs::rename(&path, root.path().join("kept.mcpr")).unwrap();
        write_recording(&path, 3_000, &[]);
        let other = game.replay_open("trip2").unwrap();
        finish_opening(game);
        assert!(
            game.studio.camera_track_dirty,
            "the edits are still there to save"
        );
        assert_eq!(game.studio.freecam.track.keyframes(), [first, second]);
        assert_eq!(game.replay_state().unwrap().recording, "trip");

        game.studio.camera_track_dirty = false;
        game.replay_close().unwrap();
        game.native_replay_requests();
        ids = vec![trip, again, other];
    });
    let [trip, again, other] = ids[..] else {
        panic!("the test ended early");
    };
    assert_eq!(
        heard,
        [
            format!("opened trip 1000 true #{trip}"),
            format!("failed trip Unavailable #{again}"),
            format!("failed trip2 Unavailable #{other}"),
            "closed trip".into(),
        ]
    );
}

const SAVE_AND_QUIT: &str =
    "game::native_replay_tests::save_and_quit_during_a_read_opens_no_replay";

/// A mod asks for a replay in a single-player world, and the person chooses Save and Quit while
/// it is read: the world is left, and the request fails with `WorldBusy`, opening nothing.
#[test]
#[ignore = "daily"]
fn save_and_quit_during_a_read_opens_no_replay() {
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        SAVE_AND_QUIT,
        "HORIZON_REPLAY_SAVE_AND_QUIT_CHILD",
        &["single_player"],
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_recording(&root.path().join("trip.mcpr"), 1_000, &[]);
    let mut ids = Vec::new();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        game.settings.render_distance = 2;
        game.settings.simulation_distance = 2;
        let options = horizon_client::title::WorldOptions {
            world_type: horizon_client::save::WorldPreset::Flat,
            ..Default::default()
        };
        game.open_integrated(
            "replay_quit",
            Some("Replay quit"),
            Some(1),
            true,
            Some(&options),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !game
            .remote
            .as_ref()
            .is_some_and(|r| r.logged_in && r.joined)
        {
            game.poll_remote().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }

        ids.push(game.replay_open("trip").unwrap());
        // Save and Quit, carried through before the frame's end takes up the read recording.
        game.close_world().unwrap();
        while game.pending_save.is_some() || game.integrated.is_some() || game.remote.is_some() {
            game.poll_closing_save().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "the world never closed"
            );
            std::thread::yield_now();
        }
        finish_opening(game);
        assert_eq!(game.replay_state(), None);
        assert!(game.remote.is_none(), "no replay over the title");
    });
    let [request] = ids[..] else {
        panic!("the test ended early");
    };
    assert_eq!(heard, [format!("failed trip WorldBusy #{request}")]);
}

/// A screen the opening snapshot opens (a workstation's, sent at the Login's time) stays open:
/// the replay's screen and input resets come before the snapshot, not after it.
#[test]
#[ignore = "daily"]
fn a_screen_the_opening_snapshot_opens_stays_open() {
    use horizon_core::nbt::Tag;
    use horizon_protocol::packets::play;
    // `minecraft:crafting` in the menu registry; checked against the session's below.
    const CRAFTING: i32 = 12;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("bench.mcpr");
    write_packets(
        &path,
        1_000,
        vec![
            (0, login_packet()),
            (
                0,
                play::Clientbound::OpenScreen(play::OpenScreen {
                    container: 1,
                    menu: CRAFTING,
                    title: Tag::String("Crafting".into()),
                }),
            ),
        ],
    );
    with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        let menu = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:crafting");
        assert_eq!(menu, Some(CRAFTING as _));
        assert_eq!(
            game.screen,
            crate::game::Screen::Inventory,
            "the crafting menu the snapshot opened is shown"
        );
        game.screen = crate::game::Screen::Playing;
        game.replay_close().unwrap();
        game.native_replay_requests();
    });
}

/// A mod asks for the archive playing again. Untouched, it reopens. Once the editor has saved
/// into it (a marker) after the request was accepted, the copy read is stale: the request fails
/// with `Unavailable`, the replay playing keeps the marker, and its later saves still land.
#[test]
#[ignore = "daily"]
fn a_reopen_of_the_playing_archive_after_an_editor_save_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    write_recording(&path, 1_000, &[]);
    let markers = |game: &mut crate::game::Game<'_>| {
        game.replay
            .driver
            .as_mut()
            .and_then(|driver| driver.as_any_mut().downcast_mut::<ReplayDriver>())
            .map(|driver| driver.markers().len())
    };
    let mut ids = Vec::new();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let first = game.replay_open("trip").unwrap();
        finish_opening(game);
        let clean = game.replay_open("trip").unwrap();
        finish_opening(game);
        assert_eq!(game.replay_state().unwrap().recording, "trip");

        let stale = game.replay_open("trip").unwrap();
        game.add_studio_marker();
        finish_opening(game);
        assert_eq!(markers(game), Some(1), "the marker stays");
        game.add_studio_marker();
        assert_eq!(markers(game), Some(2), "later saves still land");
        assert_eq!(ReplayDriver::open(&path).unwrap().markers().len(), 2);

        game.replay_close().unwrap();
        game.native_replay_requests();
        ids = vec![first, clean, stale];
    });
    let [first, clean, stale] = ids[..] else {
        panic!("the test ended early");
    };
    assert_eq!(
        heard,
        [
            format!("opened trip 1000 true #{first}"),
            "closed trip".into(),
            format!("opened trip 1000 true #{clean}"),
            format!("failed trip Unavailable #{stale}"),
            "closed trip".into(),
        ]
    );
}

/// What a replay shows that its packets set: the chat, where entity 50 is, the block at
/// (1, 64, 1).
fn shown(game: &crate::game::Game<'_>) -> (Vec<String>, Option<[f64; 3]>, Option<String>) {
    let host: &dyn ClientHost = game;
    let position = game
        .entity(50)
        .and_then(|entity| game.entity_position(entity));
    let block = host
        .level("minecraft:overworld")
        .and_then(|level| level.block(host, [1, 64, 1]))
        .and_then(mod_api::__private::block_key_id)
        .map(str::to_owned);
    (game.ui.chat.recent_lines(10), position, block)
}

/// An air column at chunk (0, 0), for a recorded block change to land in.
fn air_column(game: &crate::game::Game<'_>) -> horizon_protocol::packets::play::Clientbound {
    use horizon_protocol::chunk::{ChunkData, LightData, Paletted, Section, Strategy};
    use horizon_protocol::packets::play;
    let air = game.res.facts().air.0 as i32;
    let sections: Vec<_> = (0..24)
        .map(|_| Section {
            non_empty: 0,
            fluids: 0,
            states: Paletted::from_values(&[air; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
        })
        .collect();
    play::Clientbound::LevelChunkWithLight(play::LevelChunkWithLight {
        x: 0,
        z: 0,
        chunk: ChunkData {
            heightmaps: vec![],
            buffer: ChunkData::from_sections(&sections),
            block_entities: vec![],
        },
        light: LightData {
            sky_mask: vec![],
            block_mask: vec![],
            empty_sky_mask: vec![],
            empty_block_mask: vec![],
            sky: vec![],
            block: vec![],
        },
    })
}

/// A mod's seek, carried out at the frame's end, and the frame drawn after it.
fn seek_frame(game: &mut crate::game::Game<'_>, ms: u32) {
    game.replay_seek(ms).unwrap();
    game.native_replay_requests();
    frame(game, 16);
}

/// A paused replay sought forward in steps shows what one rebuilt at the same position shows:
/// the chat said by then, the entity where it is drawn and where the game's ticks then carry
/// it, the block changed. Each step is heard exactly where it went.
#[test]
#[ignore = "daily"]
fn a_replay_sought_forward_in_steps_shows_what_a_rebuild_there_shows() {
    use horizon_protocol::packets::play;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    let mut steps = Vec::new();
    let heard = with_heard(|game| {
        // The session's ids for a pig and stone, read off a replay's session first.
        let ids = root.path().join("ids.mcpr");
        write_recording(&ids, 1_000, &[]);
        let answer = game.open_studio_replay(&ids.display().to_string()).unwrap();
        assert_eq!(answer["ok"], true);
        let pig = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:pig")
            .unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0 as i32;
        assert!(game.close_studio_replay());

        write_packets(
            &path,
            1_000,
            vec![
                (0, login_packet()),
                (0, air_column(game)),
                (200, chat_packet("said at 200")),
                (
                    300,
                    play::Clientbound::AddEntity(play::AddEntity {
                        id: 50,
                        uuid: 50,
                        kind: pig,
                        position: [0.5, 64.0, 0.5],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }),
                ),
                (500, chat_packet("said at 500")),
                (
                    600,
                    play::Clientbound::EntityPositionSync(play::EntityPositionSync {
                        id: 50,
                        path: play::PositionPath::Linear([3.5, 64.0, 2.5]),
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground: true,
                    }),
                ),
                (
                    700,
                    play::Clientbound::BlockUpdate(play::BlockUpdate {
                        pos: [1, 64, 1],
                        state: stone,
                    }),
                ),
                (900, chat_packet("said at 900")),
            ],
        );
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        game.replay_pause().unwrap();
        let opened = shown(game);
        assert_eq!(opened.1, None, "no entity yet");
        assert_eq!(opened.2.as_deref(), Some("minecraft:air"));

        // Forward in steps, each past some of the packets.
        for ms in [250, 400, 650, 800] {
            seek_frame(game, ms);
            assert_eq!(game.replay_state().unwrap().position_ms, ms);
            steps.push(shown(game));
        }
        let forward = steps.last().unwrap().clone();
        assert_eq!(
            forward
                .0
                .iter()
                .filter(|line| line.starts_with("said at"))
                .count(),
            2,
            "{:?}",
            forward.0
        );
        // Drawn where it was added on the seek's frame, whichever way the seek went: the game's
        // own ticks (they run while the replay is paused) then carry it to where the sync at
        // 600 sends it.
        assert_eq!(forward.1, Some([0.5, 64.0, 0.5]));
        assert_eq!(forward.2.as_deref(), Some("minecraft:stone"));
        let ticked = |game: &mut crate::game::Game<'_>| {
            for _ in 0..3 {
                game.tick_remote_level().unwrap();
            }
            shown(game).1
        };
        assert_eq!(
            ticked(game),
            Some([3.5, 64.0, 2.5]),
            "where the sync sent it"
        );

        // To the end, then back: the world rebuilt from the start at the same position.
        seek_frame(game, 1_000);
        assert!(game.replay_state().unwrap().ended);
        seek_frame(game, 800);
        assert_eq!(shown(game), forward, "a rebuild at 800 shows the same");
        assert_eq!(ticked(game), Some([3.5, 64.0, 2.5]), "and moves the same");

        game.replay_close().unwrap();
        game.native_replay_requests();
    });
    assert_eq!(steps[0].1, None, "the pig comes at 300");
    assert_eq!(steps[1].1, Some([0.5, 64.0, 0.5]), "added there");
    assert_eq!(steps[1].2.as_deref(), Some("minecraft:air"));
    let heard: Vec<_> = heard
        .into_iter()
        .filter(|line| line.starts_with("advanced"))
        .collect();
    assert_eq!(
        heard,
        [
            "advanced 250",
            "advanced 400",
            "advanced 650",
            "advanced 800",
            "advanced 1000",
            "advanced 800",
        ]
    );
}

/// A seek forward plays on in the same session; only a seek back makes a new one.
#[test]
#[ignore = "daily"]
fn a_forward_seek_plays_on_and_only_a_backward_one_rebuilds() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    write_recording(&path, 1_000, &[200, 600]);
    with_heard(|game| {
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        let session = game.replay.connections;
        game.seek_studio_replay(300).unwrap();
        game.seek_studio_replay(900).unwrap();
        assert_eq!(game.replay.connections, session, "played on");
        assert_eq!(game.replay.connection, session);
        game.seek_studio_replay(100).unwrap();
        assert_eq!(game.replay.connections, session + 1, "rebuilt");
        assert_eq!(game.replay.connection, game.replay.connections);
        game.replay_close().unwrap();
        game.native_replay_requests();
    });
}

/// A seek to where the clock is, between two whole milliseconds or paused on one, reports that
/// position; a seek forward past a packet that leaves the session ends the replay once, the
/// rest of the batch not applied and nothing heard after.
#[test]
#[ignore = "daily"]
fn a_seek_to_the_clock_reports_it_and_a_seek_past_a_leave_ends_the_replay_once() {
    use horizon_protocol::packets::play;
    let root = tempfile::tempdir().unwrap();
    write_packets(
        &root.path().join("kick.mcpr"),
        1_000,
        vec![
            (0, login_packet()),
            (
                500,
                play::Clientbound::DeleteChat(play::DeleteChat {
                    signature: (5, None),
                }),
            ),
            (500, chat_packet("said after the kick")),
        ],
    );
    let mut request = 0;
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        request = game.replay_open("kick").unwrap();
        finish_opening(game);
        let session = game.replay.connections;
        // The open's first frame ran 16 ms; 33 ms at half speed puts the clock at 32.5.
        game.replay_set_speed(0.5).unwrap();
        frame(game, 33);
        assert_eq!(game.replay_state().unwrap().position_ms, 32);
        seek_frame(game, 32);
        game.replay_pause().unwrap();
        seek_frame(game, 32);
        assert_eq!(game.replay.connections, session, "never rebuilt");

        game.replay_seek(600).unwrap();
        game.native_replay_requests();
        assert_eq!(game.replay_state(), None);
        frame(game, 16);
        game.native_replay_requests();
        assert!(
            !game
                .ui
                .chat
                .recent_lines(10)
                .iter()
                .any(|line| line == "said after the kick"),
            "the rest of the batch went nowhere"
        );
    });
    assert_eq!(
        heard,
        [
            format!("opened kick 1000 true #{request}"),
            "advanced 32".into(),
            "advanced 32".into(),
            "advanced 32".into(),
            "closed kick".into(),
        ]
    );
}

/// A seek forward across a recorded relative teleport of the player is rebuilt: the player
/// lands where a fresh open sought there puts it, not on wherever the game's ticks had moved it.
#[test]
#[ignore = "daily"]
fn a_forward_seek_across_a_relative_teleport_lands_where_a_rebuild_does() {
    use horizon_protocol::packets::play;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hop.mcpr");
    let at = |relatives, position| {
        play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 0,
            position,
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives,
        })
    };
    let height = |game: &crate::game::Game<'_>| game.player.as_ref().unwrap().position;
    with_heard(|game| {
        // The session's air, read off a replay's session first.
        let ids = root.path().join("ids.mcpr");
        write_recording(&ids, 1_000, &[]);
        let answer = game.open_studio_replay(&ids.display().to_string()).unwrap();
        assert_eq!(answer["ok"], true);
        let column = air_column(game);
        assert!(game.close_studio_replay());
        write_packets(
            &path,
            1_000,
            vec![
                (0, login_packet()),
                (0, column),
                (0, at(0, [8.5, 80.0, 8.5])),
                (500, at(play::relative::Y, [0.0, 2.0, 0.0])),
            ],
        );

        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        for _ in 0..10 {
            game.tick_remote_level().unwrap();
        }
        assert!(height(game).y < 80.0, "the game's ticks moved the player");
        let session = game.replay.connections;
        game.seek_studio_replay(600).unwrap();
        assert_eq!(game.replay.connections, session + 1, "rebuilt");
        let sought = height(game);
        assert!(game.close_studio_replay());

        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        game.seek_studio_replay(600).unwrap();
        assert_eq!(sought, height(game), "as a fresh open sought there");
        assert_eq!(sought.y, 82.0);
        assert!(game.close_studio_replay());
    });
}

/// A recording whose reads all fail from here on, as a damaged archive's would: playing and
/// every seek that reads fail.
struct FailingReads(Box<dyn PacketSource>);

impl FailingReads {
    fn error() -> io::Error {
        io::Error::other("injected read failure")
    }
}

impl PacketSource for FailingReads {
    fn position_ms(&self) -> u32 {
        self.0.position_ms()
    }
    fn duration_ms(&self) -> u32 {
        self.0.duration_ms()
    }
    fn packet_count(&self) -> usize {
        self.0.packet_count()
    }
    fn progress(&self) -> f32 {
        self.0.progress()
    }
    fn speed(&self) -> f64 {
        self.0.speed()
    }
    fn set_speed(&mut self, speed: f64) {
        self.0.set_speed(speed);
    }
    fn is_paused(&self) -> bool {
        self.0.is_paused()
    }
    fn set_paused(&mut self, paused: bool) {
        self.0.set_paused(paused);
    }
    fn toggle_pause(&mut self) {
        self.0.toggle_pause();
    }
    fn set_looping(&mut self, looping: bool) {
        self.0.set_looping(looping);
    }
    fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
    fn advance(&mut self, _: Duration) -> io::Result<Vec<TimestampedPacket>> {
        Err(Self::error())
    }
    fn seek(&mut self, _: u32) -> io::Result<()> {
        Err(Self::error())
    }
    fn seek_and_collect(&mut self, _: u32) -> io::Result<Vec<TimestampedPacket>> {
        Err(Self::error())
    }
    fn seek_forward(&mut self, _: u32) -> io::Result<Vec<TimestampedPacket>> {
        Err(Self::error())
    }
    fn restart(&mut self) -> io::Result<()> {
        Err(Self::error())
    }
    // The editor's close reaches the real driver (its camera path).
    fn as_any(&self) -> &dyn Any {
        self.0.as_any()
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.0.as_any_mut()
    }
}

/// A read that fails while a replay seeks or plays ends it as a close does: mods hear it closed
/// once, a standalone replay's session goes, and the seek's caller gets the error. A refused
/// backward seek of an in-world replay ends nothing.
#[test]
#[ignore = "daily"]
fn a_failed_read_ends_the_replay_as_a_close_does() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("worn.mcpr");
    write_recording(&path, 1_000, &[200, 600]);
    let open = |game: &mut crate::game::Game<'_>| {
        let answer = game
            .open_studio_replay(&path.display().to_string())
            .unwrap();
        assert_eq!(answer["ok"], true);
        let driver = game.replay.driver.take().unwrap();
        game.replay.driver = Some(Box::new(FailingReads(driver)));
    };
    let ended = |game: &mut crate::game::Game<'_>| {
        assert_eq!(game.replay_state(), None);
        assert!(game.remote.is_none(), "the replay's session went");
        frame(game, 16);
        game.native_replay_requests();
    };
    let heard = with_heard(|game| {
        // A seek back rebuilds; a seek forward whose read fails falls back to a rebuild.
        for target in [0, 900] {
            open(game);
            assert!(game.seek_studio_replay(target).is_err());
            ended(game);
        }
        open(game);
        game.replay_seek(900).unwrap();
        game.native_replay_requests();
        ended(game);
        open(game);
        let answer = game
            .studio_control(&["studio", "replay", "seek", "900"])
            .unwrap()
            .unwrap();
        assert_eq!(answer["error"], "injected read failure");
        ended(game);
        open(game);
        assert!(game.studio_command("studio replay seek 900"));
        ended(game);
        open(game);
        frame(game, 16);
        ended(game);

        play(game, 10_000);
        let mut clock = Clock::new(10_000);
        clock.seek(500).unwrap();
        game.replay.driver = Some(Box::new(FailingReads(Box::new(clock))));
        assert_eq!(
            game.seek_studio_replay(100).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert!(game.replay_state().is_some(), "refused, not ended");
        assert!(game.seek_studio_replay(800).is_err());
        assert_eq!(game.replay_state(), None);
    });
    let closed: Vec<_> = heard
        .iter()
        .filter(|line| line.starts_with("closed"))
        .collect();
    // A replay opened by path has no recording id.
    assert_eq!(
        closed,
        ["closed "; 6]
            .into_iter()
            .chain(["closed demo"])
            .collect::<Vec<_>>()
    );
}

/// A close whose save of the edited camera path fails keeps the path in a new draft beside the
/// recording and says where in chat, whether a failed read or the player closed it; a later
/// draft never replaces an earlier one.
#[test]
#[ignore = "daily"]
fn a_close_that_cannot_save_the_camera_path_keeps_it_in_a_draft() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("trip.mcpr");
    write_recording(&path, 1_000, &[200, 600]);
    let draft = |name: &str| {
        let json = std::fs::read(root.path().join(name)).unwrap();
        serde_json::from_slice::<CameraTrack>(&json).unwrap()
    };
    with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let edited = |game: &mut crate::game::Game<'_>, at| {
            game.replay_open("trip").unwrap();
            finish_opening(game);
            let keyframe = CameraKeyframe::new(at, [1.0, 2.0, 3.0], 10.0, 5.0, 70.0);
            game.studio.freecam.track.insert(keyframe.clone());
            game.studio.camera_track_dirty = true;
            // Another file put at the recording's path: the save into it is refused.
            std::fs::remove_file(&path).unwrap();
            write_recording(&path, 1_000, &[200, 600]);
            keyframe
        };
        let said = |game: &crate::game::Game<'_>, name: &str| {
            let draft = root.path().join(name).display().to_string();
            game.ui
                .chat
                .recent_lines(10)
                .iter()
                .any(|line| line.contains(&draft))
        };

        let first = edited(game, 250);
        let driver = game.replay.driver.take().unwrap();
        game.replay.driver = Some(Box::new(FailingReads(driver)));
        assert!(game.seek_studio_replay(900).is_err());
        assert_eq!(game.replay_state(), None);
        assert_eq!(draft("trip.camera-draft.json").keyframes(), [first]);
        assert!(said(game, "trip.camera-draft.json"));

        let second = edited(game, 750);
        assert!(game.close_studio_replay());
        assert_eq!(draft("trip.camera-draft-2.json").keyframes(), [second]);
        assert!(said(game, "trip.camera-draft-2.json"));
        assert!(
            ReplayDriver::open(&path).unwrap().camera_path().is_none(),
            "the file put in its place is untouched"
        );
    });
}
