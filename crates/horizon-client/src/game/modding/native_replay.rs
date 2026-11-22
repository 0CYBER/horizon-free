//! A client mod reading and driving the platform's replay playback (#19247): the state read,
//! open, close, pause, resume, speed and seek. Opening reads the recording off the game's thread
//! and installs it at a frame's end; close and seek leave a world or apply the recording's
//! packets, so they too run at the end of the frame, outside every hook, as `disconnect` does.
//! Pause, resume and speed only set the clock and apply at once.
use super::{ClientQuit, Game, PendingWorldEntry};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use horizon_client::file_id::{reparse_point, valid_id};
use horizon_client::studio::ReplayDriver;
use mod_api::{REPLAY_MAX_SPEED, REPLAY_MIN_SPEED, ReplayError, ReplayState};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;

/// A fresh request id for `replay_open`: never zero, never given twice while the game runs; none
/// once every `u32` has been given (then `replay_open` is `Unavailable`).
fn next_request() -> Option<u32> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    take_request(&NEXT)
}

/// The id `next` holds, moving it on; none once it ran out (it holds 0 then, never handed out).
pub(super) fn take_request(next: &AtomicU32) -> Option<u32> {
    next.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        (id != 0).then(|| id.wrapping_add(1))
    })
    .ok()
}

/// A mod's close or seek, carried out at the end of the frame. One a frame: a later request
/// replaces an earlier one.
pub(super) enum ReplayRequest {
    /// Close the replay of this generation.
    Close { generation: u64 },
    /// Seek the replay of this generation.
    Seek { generation: u64, ms: u32 },
}

/// The session an open was accepted in. A request that meets another one when its recording is
/// ready fails (`WorldBusy`). Neither is a world's epoch, which a dimension change renews.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ReplayScope {
    /// A replay plays: this one (its generation).
    Replay(u64),
    /// No replay plays: the connections this window had made by then, so one made (and maybe
    /// left) since counts as a change, and whether one was open, so leaving it (Save and Quit,
    /// Disconnect) counts too.
    Session { connections: u64, connected: bool },
}

/// A file as the system knows it (its volume or device, and its index or inode), whatever path
/// names it now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::game) struct FileIdentity {
    device: u64,
    index: u64,
}

impl FileIdentity {
    /// `file`'s identity, read from its open handle.
    pub(in crate::game) fn of(file: &File) -> io::Result<Self> {
        use cap_fs_ext::MetadataExt;
        let metadata = cap_std::fs::File::from_std(file.try_clone()?).metadata()?;
        Ok(Self {
            device: metadata.dev(),
            index: metadata.ino(),
        })
    }
}

/// Who a replay was opened for: its id, the mod request that asked for it (0 for none: the
/// replays list, `/studio replay`, the control socket), and for a mod's, the archive it opened.
pub(in crate::game) struct ReplayOrigin {
    pub(in crate::game) recording: String,
    pub(in crate::game) request: u32,
    pub(in crate::game) archive: Option<FileIdentity>,
    /// The archive file read, taken from the handle it was read through (any route).
    pub(in crate::game) identity: Option<FileIdentity>,
    /// For a mod's open asked while a replay played, the archive file that replay read when the
    /// open was accepted: an editor save into it since makes the copy read stale.
    pub(in crate::game) replacing: Option<FileIdentity>,
}

/// A mod's accepted open waiting for the reading thread: the file opened at the call.
pub(super) struct QueuedOpen {
    origin: ReplayOrigin,
    scope: ReplayScope,
    file: File,
    path: PathBuf,
}

/// The one thread reading a mod's recording. `origin` is none once a later open superseded it:
/// its work is cancelled and whatever it answers is dropped.
pub(super) struct ReplayOpening {
    origin: Option<ReplayOrigin>,
    scope: ReplayScope,
    cancel: Arc<AtomicBool>,
    prepared: Receiver<io::Result<ReplayDriver>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ReplayOpening {
    /// A game closing (or a superseded open) stops the reading; its temporary stream goes with
    /// its handle.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The recording file a mod names by `id` in `dir`: `<dir>/<id>.mcpr`. The id is checked as text
/// first (`valid_id`), touching no file, so it can't be used to probe the disk. Then `dir` must be
/// a real folder, not a link to one, and the file a regular file, not a link, whose real path is
/// directly in `dir`'s. A missing file, a link, a reparse point, a redirected folder or an escape
/// all answer `NotFound`, the same as one another.
pub(super) fn recording_path(dir: &Path, id: &str) -> Result<PathBuf, ReplayError> {
    if !valid_id(id) {
        return Err(ReplayError::InvalidId);
    }
    let folder = std::fs::symlink_metadata(dir).map_err(|_| ReplayError::NotFound)?;
    if !folder.is_dir() || reparse_point(&folder) {
        return Err(ReplayError::NotFound);
    }
    let file = dir.join(format!("{id}.mcpr"));
    let metadata = std::fs::symlink_metadata(&file).map_err(|_| ReplayError::NotFound)?;
    if !metadata.is_file() || reparse_point(&metadata) {
        return Err(ReplayError::NotFound);
    }
    let real_dir = dir.canonicalize().map_err(|_| ReplayError::NotFound)?;
    let real_file = file.canonicalize().map_err(|_| ReplayError::NotFound)?;
    if real_file.parent() != Some(real_dir.as_path()) {
        return Err(ReplayError::NotFound);
    }
    Ok(file)
}

/// The recordings folder `dir`, opened as a folder handle: `dir`'s parent is opened as the game
/// names it, and `dir` in it without following a link. Every recording is opened through this
/// handle, so a folder renamed or replaced by a link afterwards can't redirect the open.
pub(super) fn recordings_folder(dir: &Path) -> Result<Dir, ReplayError> {
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
        return Err(ReplayError::NotFound);
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let parent = Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| ReplayError::NotFound)?;
    let metadata = parent
        .symlink_metadata(name)
        .map_err(|_| ReplayError::NotFound)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ReplayError::NotFound);
    }
    parent
        .open_dir_nofollow(name)
        .map_err(|_| ReplayError::NotFound)
}

/// The recording `id` names in the recordings folder `folder`, opened once relative to that
/// handle without following a link, and checked again once open: a regular file, not a reparse
/// point, starting as a zip archive does (else `Unreadable`). It comes back rewound with its
/// identity. Nothing later opens it by name.
pub(super) fn open_in_folder(folder: &Dir, id: &str) -> Result<(File, FileIdentity), ReplayError> {
    if !valid_id(id) {
        return Err(ReplayError::InvalidId);
    }
    let name = format!("{id}.mcpr");
    let metadata = folder
        .symlink_metadata(&name)
        .map_err(|_| ReplayError::NotFound)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ReplayError::NotFound);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let opened = folder
        .open_with(&name, &options)
        .map_err(|_| ReplayError::NotFound)?;
    let mut file = opened.into_std();
    let metadata = file.metadata().map_err(|_| ReplayError::NotFound)?;
    if !metadata.is_file() || reparse_point(&metadata) {
        return Err(ReplayError::NotFound);
    }
    let identity = FileIdentity::of(&file).map_err(|_| ReplayError::NotFound)?;
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err()
        || magic != *b"PK\x03\x04"
        || file.seek(SeekFrom::Start(0)).is_err()
    {
        return Err(ReplayError::Unreadable);
    }
    Ok((file, identity))
}

/// The recording `id` names in `dir`, opened once through the folder's handle
/// (`recordings_folder`, `open_in_folder`); its path only names it to the editor and the log.
pub(super) fn open_recording(
    dir: &Path,
    id: &str,
) -> Result<(PathBuf, File, FileIdentity), ReplayError> {
    if !valid_id(id) {
        return Err(ReplayError::InvalidId);
    }
    let folder = std::fs::symlink_metadata(dir).map_err(|_| ReplayError::NotFound)?;
    if !folder.is_dir() || reparse_point(&folder) {
        return Err(ReplayError::NotFound);
    }
    let (file, identity) = open_in_folder(&recordings_folder(dir)?, id)?;
    Ok((dir.join(format!("{id}.mcpr")), file, identity))
}

impl Game<'_> {
    /// The recordings folder replays are opened from, the one `/studio record` writes to.
    fn recordings_dir(&self) -> PathBuf {
        self.replay
            .recordings_dir
            .clone()
            .unwrap_or_else(horizon_client::studio::recorder::default_recordings_dir)
    }

    /// The id of the recording at `path`, under the one policy `replay_open` resolves by: its
    /// file name without `.mcpr` when that id names this very file in the recordings folder,
    /// else empty (a recording opened from elsewhere, or whose name no id can hold).
    pub(super) fn recording_id(&self, path: &str) -> String {
        let path = Path::new(path);
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".mcpr"))
        else {
            return String::new();
        };
        let same = recording_path(&self.recordings_dir(), id)
            .ok()
            .and_then(|by_id| by_id.canonicalize().ok())
            .is_some_and(|by_id| path.canonicalize().is_ok_and(|real| real == by_id));
        if same { id.to_owned() } else { String::new() }
    }

    /// A replay can be opened or driven by a mod: a client that is neither a splitscreen guest
    /// nor closing.
    fn replay_available(&self) -> bool {
        self.options.is_client_start()
            && self.client_quit.get() == ClientQuit::Running
            && self.pending_save.is_none()
    }

    /// A session a replay must not displace: a world entry waiting, a single-player world
    /// starting, a server joined or being joined (a replay's own stand-in excepted), or a world
    /// others play in too (open to LAN, or with splitscreen players).
    fn replay_world_busy(&self) -> bool {
        self.pending_entry.is_some()
            || self.integrated_starting.is_some()
            || (!self.replay.playing() && self.integrated.is_none() && self.remote.is_some())
            || self.is_published()
            || self.has_split_players()
            || self.split_window
    }

    /// The session an open is accepted in (`ReplayScope`).
    pub(super) fn replay_scope(&self) -> ReplayScope {
        if self.replay.playing() {
            ReplayScope::Replay(self.replay.generation)
        } else {
            ReplayScope::Session {
                connections: self.replay.connections,
                connected: self.remote.is_some() || self.integrated.is_some(),
            }
        }
    }

    /// The replay this window plays, for a client mod.
    pub(super) fn replay_state(&self) -> Option<ReplayState> {
        let driver = self.replay.driver.as_ref()?;
        Some(mod_api::__private::built::replay_state(
            self.replay.recording.clone(),
            self.replay.standalone,
            driver.position_ms(),
            driver.duration_ms(),
            driver.speed(),
            driver.is_paused(),
            driver.is_finished(),
        ))
    }

    /// Accepts a mod's request to open a recording by id: checks it and opens its file once,
    /// through the recordings folder's handle. One thread reads recordings at a time: the
    /// request waits for it, and a later request takes the place of one not yet ready, which
    /// is told `Superseded` at the frame's end.
    pub(super) fn replay_open(&mut self, recording: &str) -> Result<u32, ReplayError> {
        // A replay whose session has gone is no replay playing: the ordinary guards apply.
        self.drop_stale_replay();
        if !self.replay_available() {
            return Err(ReplayError::Unavailable);
        }
        if self.replay_world_busy() {
            return Err(ReplayError::WorldBusy);
        }
        let (path, file, archive) = open_recording(&self.recordings_dir(), recording)?;
        let request = next_request().ok_or(ReplayError::Unavailable)?;
        let replacing = self.playing_archive_identity();
        let queued = QueuedOpen {
            origin: ReplayOrigin {
                recording: recording.to_owned(),
                request,
                archive: Some(archive),
                identity: Some(archive),
                replacing,
            },
            scope: self.replay_scope(),
            file,
            path,
        };
        if let Some(earlier) = self.replay.queued.replace(queued) {
            self.replay
                .open_failures
                .push((earlier.origin, ReplayError::Superseded));
        }
        if let Some(opening) = &mut self.replay.opening
            && let Some(earlier) = opening.origin.take()
        {
            opening.cancel.store(true, Ordering::Relaxed);
            self.replay
                .open_failures
                .push((earlier, ReplayError::Superseded));
        }
        self.start_queued_open();
        Ok(request)
    }

    /// Hands the waiting request to a reading thread, once no other one runs.
    fn start_queued_open(&mut self) {
        if self.replay.opening.is_some() {
            return;
        }
        let Some(QueuedOpen {
            origin,
            scope,
            file,
            path,
        }) = self.replay.queued.take()
        else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, prepared) = std::sync::mpsc::channel();
        let reading = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("replay-prepare".into())
            .spawn(move || {
                let _ = send.send(ReplayDriver::from_file_cancellable(file, &path, &reading));
            });
        match spawned {
            Ok(thread) => {
                self.replay.opening = Some(ReplayOpening {
                    origin: Some(origin),
                    scope,
                    cancel,
                    prepared,
                    thread: Some(thread),
                });
            }
            Err(error) => {
                log::error!(
                    "Replay {:?}: no thread to read it: {error}",
                    origin.recording
                );
                self.replay
                    .open_failures
                    .push((origin, ReplayError::Unavailable));
            }
        }
    }

    /// Asks to close the replay at the end of this frame.
    pub(super) fn replay_close(&mut self) -> Result<(), ReplayError> {
        if self.replay.driver.is_none() {
            return Err(ReplayError::NotOpen);
        }
        self.replay.request = Some(ReplayRequest::Close {
            generation: self.replay.generation,
        });
        Ok(())
    }

    /// Pauses the replay's clock.
    pub(super) fn replay_pause(&mut self) -> Result<(), ReplayError> {
        let driver = self.replay.driver.as_mut().ok_or(ReplayError::NotOpen)?;
        driver.set_paused(true);
        Ok(())
    }

    /// Starts the replay's clock again.
    pub(super) fn replay_resume(&mut self) -> Result<(), ReplayError> {
        let driver = self.replay.driver.as_mut().ok_or(ReplayError::NotOpen)?;
        driver.set_paused(false);
        Ok(())
    }

    /// Sets the playback speed; a speed out of range, NaN included, changes nothing.
    pub(super) fn replay_set_speed(&mut self, speed: f64) -> Result<(), ReplayError> {
        if !(REPLAY_MIN_SPEED..=REPLAY_MAX_SPEED).contains(&speed) {
            return Err(ReplayError::InvalidSpeed);
        }
        let driver = self.replay.driver.as_mut().ok_or(ReplayError::NotOpen)?;
        driver.set_speed(speed);
        Ok(())
    }

    /// Asks to seek the replay at the end of this frame.
    pub(super) fn replay_seek(&mut self, ms: u32) -> Result<(), ReplayError> {
        let driver = self.replay.driver.as_ref().ok_or(ReplayError::NotOpen)?;
        // A replay on a session it doesn't own can't rebuild an earlier world.
        if !self.replay.standalone && ms.min(driver.duration_ms()) < driver.position_ms() {
            return Err(ReplayError::BackwardSeek);
        }
        self.replay.request = Some(ReplayRequest::Seek {
            generation: self.replay.generation,
            ms,
        });
        Ok(())
    }

    /// The end of the frame: a mod's close or seek runs now, outside every hook, then an open
    /// whose recording is ready, then the next waiting one starts reading, and the requests that
    /// failed since are told. A close or seek of a replay closed or replaced since is dropped.
    pub(in crate::game) fn native_replay_requests(&mut self) {
        if let Some(request) = self.replay.request.take()
            && self.replay_available()
        {
            let current = |game: &Self, generation| {
                game.replay.driver.is_some() && game.replay.generation == generation
            };
            match request {
                ReplayRequest::Close { generation } => {
                    if current(self, generation) {
                        self.close_studio_replay();
                    }
                }
                ReplayRequest::Seek { generation, ms } => {
                    if current(self, generation)
                        && let Err(error) = self.seek_studio_replay(ms)
                    {
                        log::warn!("Replay seek to {ms} ms failed: {error}");
                    }
                }
            }
        }
        // Superseded requests hear so before a later one's outcome.
        self.tell_replay_open_failures();
        self.poll_replay_opening();
        self.start_queued_open();
        self.tell_replay_open_failures();
        // A request a hook made while these ran (a seek from `replay_opened`) waits for the
        // next frame's end; its generation keeps it from reaching another replay.
    }

    fn tell_replay_open_failures(&mut self) {
        for (origin, error) in std::mem::take(&mut self.replay.open_failures) {
            self.replay_open_failed(&origin, error);
        }
    }

    fn replay_open_failed(&mut self, origin: &ReplayOrigin, error: ReplayError) {
        self.native_replay_open_failed(origin.request, &origin.recording, error);
    }

    /// The reading thread's answer: installed if the session it was asked in is still the one
    /// here, else told as failed. A superseded thread's answer, whatever it is, is dropped once
    /// the thread has ended; the next request starts reading only then.
    fn poll_replay_opening(&mut self) {
        let Some(opening) = &mut self.replay.opening else {
            return;
        };
        if opening.origin.is_none() {
            if opening.thread.as_ref().is_none_or(JoinHandle::is_finished) {
                if let Some(thread) = opening.thread.take() {
                    let _ = thread.join();
                }
                self.replay.opening = None;
            }
            return;
        }
        let prepared = match opening.prepared.try_recv() {
            Ok(prepared) => prepared,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(io::Error::other("the reading thread ended")),
        };
        let mut opening = self.replay.opening.take().expect("checked above");
        if let Some(thread) = opening.thread.take() {
            // It has answered: only its return is left.
            let _ = thread.join();
        }
        let origin = opening.origin.take().expect("checked above");
        let refused = if !self.replay_available() {
            Some(ReplayError::Unavailable)
        } else if self.replay_world_busy() || self.replay_scope() != opening.scope {
            Some(ReplayError::WorldBusy)
        } else {
            None
        };
        if let Some(error) = refused {
            return self.replay_open_failed(&origin, error);
        }
        match prepared {
            Ok(driver) => self.open_prepared_replay(driver, origin),
            Err(error) => {
                log::warn!("Replay {:?} could not be read: {error}", origin.recording);
                self.replay_open_failed(&origin, ReplayError::Unreadable);
            }
        }
    }

    /// Drops a mod's read replay waiting as the world entry, if that is what waits, telling the
    /// mod it opened nothing: another entry takes its place (`WorldBusy`) or the person cancels
    /// the wait (`Unavailable`). Every accepted `replay_open` ends in opened or failed.
    pub(in crate::game) fn displace_pending_replay(&mut self, error: ReplayError) {
        if let Some(PendingWorldEntry::PreparedReplay { origin, .. }) = self
            .pending_entry
            .take_if(|entry| matches!(entry, PendingWorldEntry::PreparedReplay { .. }))
        {
            self.replay_open_failed(&origin, error);
        }
    }

    /// A world entry an error dropped: a mod's read replay it carried is told as failed.
    pub(in crate::game) fn drop_world_entry(&mut self, entry: PendingWorldEntry) {
        if let PendingWorldEntry::PreparedReplay { origin, .. } = entry {
            self.replay_open_failed(&origin, ReplayError::Unavailable);
        }
    }

    /// Opens a recording already read for a mod. From a single-player world, the world is
    /// saved and left first and the replay opens on the title (`PendingWorldEntry`), carrying
    /// the read recording, never its name.
    pub(super) fn open_prepared_replay(&mut self, driver: ReplayDriver, origin: ReplayOrigin) {
        self.drop_stale_replay();
        // The camera path edited in the replay playing is saved first; a replacement never
        // costs those edits. If they can't be saved, or the replacement was read from the very
        // archive they were just saved into (so it holds them as they were), the replay playing
        // stays and the request fails.
        let saving = self.studio.camera_track_dirty && self.replay.playing();
        let playing = self.playing_archive_identity();
        let same_archive = saving && origin.archive.is_some() && playing == origin.archive;
        // The same archive asked for again, and the editor saved into it (a marker) after the
        // open was accepted: the copy read holds what was there before, so it isn't installed.
        if origin.archive.is_some()
            && origin.replacing == origin.archive
            && playing != origin.replacing
        {
            log::warn!(
                "Replay {:?} not opened: the editor saved into it after it was asked for",
                origin.recording
            );
            return self.replay_open_failed(&origin, ReplayError::Unavailable);
        }
        if let Err(error) = self.save_studio_camera_track() {
            log::warn!(
                "Replay {:?} not opened: the camera path of the replay playing could not be saved: {error}",
                origin.recording
            );
            return self.replay_open_failed(&origin, ReplayError::Unavailable);
        }
        if same_archive {
            log::warn!(
                "Replay {:?} not opened: it was read before the camera path just saved into it",
                origin.recording
            );
            return self.replay_open_failed(&origin, ReplayError::Unavailable);
        }
        if self.replay.driver.is_none() && (self.remote.is_some() || self.integrated.is_some()) {
            let recording = origin.recording.clone();
            self.pending_entry = Some(PendingWorldEntry::PreparedReplay {
                driver: Box::new(driver),
                origin,
            });
            if let Err(error) = self.close_world() {
                log::error!("Replay {recording:?}: could not leave the world: {error}");
                self.displace_pending_replay(ReplayError::Unavailable);
            }
            return;
        }
        // Replay setup builds the client registries, which need the world tables.
        if self.remote.is_none() && self.res.rest.get().is_none() {
            let entry = PendingWorldEntry::PreparedReplay {
                driver: Box::new(driver),
                origin,
            };
            let (driver, origin) = match self.try_defer_world_entry(entry) {
                Ok(None) => return,
                Ok(Some(PendingWorldEntry::PreparedReplay { driver, origin })) => (*driver, origin),
                Ok(Some(_)) => unreachable!("the entry handed back is the one given"),
                Err(error) => {
                    // The request was told as failed with the entry (`drop_world_entry`).
                    log::error!("Replay: the world tables could not load: {error}");
                    return;
                }
            };
            if let Err(error) = self.join_world_rest() {
                log::error!("Replay {:?}: {error}", origin.recording);
                return self.replay_open_failed(&origin, ReplayError::Unavailable);
            }
            return self.start_prepared_replay(driver, origin);
        }
        self.start_prepared_replay(driver, origin);
    }

    /// The archive file the replay playing read, as of the editor's last save into it: kept in
    /// memory from the handles that read and wrote it, so nothing is opened to tell.
    fn playing_archive_identity(&self) -> Option<FileIdentity> {
        self.replay.identity.filter(|_| self.replay.playing())
    }

    fn start_prepared_replay(&mut self, driver: ReplayDriver, origin: ReplayOrigin) {
        let path = driver.path().display().to_string();
        let (request, recording) = (origin.request, origin.recording.clone());
        let failed = match self.start_studio_replay(driver, origin, &path) {
            Ok(answer) if answer["ok"].as_bool() == Some(true) => return,
            Ok(answer) => answer["error"].as_str().unwrap_or_default().to_owned(),
            Err(error) => error.to_string(),
        };
        log::warn!("Replay {recording:?} could not be opened: {failed}");
        self.native_replay_open_failed(request, &recording, ReplayError::Unavailable);
    }

    /// Tells mods the replay's clock moved: at most once a frame, only when its position differs
    /// from the last one told, and not before `replay_opened`.
    pub(super) fn announce_replay_advance(&mut self) {
        let Some(position) = self.replay.driver.as_ref().map(|r| r.position_ms()) else {
            return;
        };
        if self.replay.reported_ms.is_none_or(|told| told == position) {
            return;
        }
        self.replay.reported_ms = Some(position);
        self.native_replay_advanced(position);
    }
}
