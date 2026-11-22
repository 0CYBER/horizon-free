//! The mod API queues captures for the fixed-clock exporter between callbacks.

use super::{ClientQuit, Game, Screen, export::Session, native_replay::ReplayScope};
use horizon_client::export::{
    ByteBudget, DirError, EncoderConfig, EncoderPipe, EncoderStartError, ExportClock, ExportFolder,
    ExportSpec, Format, SpecError,
};
use mod_api::{ExportError, ExportFormat, ExportRequest, ExportState, ExportStatus};
use std::sync::Arc;

const MOD_EXPORT_BUDGET: u64 = 64 << 30;
/// Limits even exports that end before producing an image.
const MOD_EXPORT_START_CHARGE: u64 = 1 << 20;

pub(super) struct NativeExport {
    pub(super) budget: Arc<ByteBudget>,
    pub(super) pending: Option<PendingExport>,
    last: Option<ExportStatus>,
    request: u32,
    pub(super) ending: bool,
}

impl Default for NativeExport {
    fn default() -> Self {
        Self {
            budget: Arc::new(ByteBudget::new(MOD_EXPORT_BUDGET)),
            pending: None,
            last: None,
            request: 0,
            ending: false,
        }
    }
}

impl NativeExport {
    pub(super) fn request_stop(
        &mut self,
        session: Option<&mut Session>,
    ) -> Result<(), ExportError> {
        if let Some(pending) = self.pending.as_mut() {
            pending.stop = true;
        } else {
            session
                .filter(|session| session.is_mod_export())
                .ok_or(ExportError::NotRunning)?
                .request_stop();
        }
        Ok(())
    }
}

pub(super) struct PendingExport {
    pub(super) spec: ExportSpec,
    pub(super) folder: Arc<ExportFolder>,
    pub(super) encoder: Option<EncoderPipe>,
    pub(super) clock: ExportClock,
    pub(super) request: u32,
    pub(super) stop: bool,
    pub(super) scope: ReplayScope,
    pub(super) epoch: mod_api::WorldEpoch,
}

impl Game<'_> {
    pub(super) fn mod_export_status(&self) -> Option<ExportStatus> {
        if let Some(status) = self.export.as_ref().and_then(|s| s.mod_status()) {
            Some(status)
        } else if let Some(pending) = &self.native_export.pending {
            Some(status(
                pending.request,
                &pending.spec,
                ExportState::Running,
                0,
                0,
            ))
        } else {
            self.native_export.last.clone()
        }
    }

    pub(super) fn start_mod_export(&mut self, request: &ExportRequest) -> Result<u32, ExportError> {
        let max_texture = self.check_mod_export_start()?;
        let epoch = self
            .native_lifecycle
            .epoch()
            .ok_or(ExportError::WorldBusy)?;
        let spec = export_spec(request, max_texture)?;
        let clock = ExportClock::new(spec.fps).map_err(|_| ExportError::InvalidRate)?;
        let id = self
            .native_export
            .request
            .checked_add(1)
            .ok_or(ExportError::Unavailable)?;
        let (folder, encoder) =
            prepare_output(&Self::game_dir(), &self.native_export.budget, &spec)?;
        self.native_export.request = id;
        self.native_export.pending = Some(PendingExport {
            spec,
            folder,
            encoder,
            clock,
            request: id,
            stop: false,
            scope: self.replay_scope(),
            epoch,
        });
        Ok(id)
    }

    pub(super) fn check_mod_export_start(&self) -> Result<u32, ExportError> {
        let occupied = self.export.is_some()
            || self.native_export.pending.is_some()
            || self.native_export.ending;
        let max_texture = self
            .renderer
            .as_ref()
            .filter(|r| {
                self.options.is_client_start()
                    && self.options.offscreen
                    && !self.options.paced_by_split
                    && r.supports_frame_export()
            })
            .map(|r| r.device.limits().max_texture_dimension_2d);
        let world_ready = self.title.is_none()
            && self.player.is_some()
            && self.screen == Screen::Playing
            && self.pending_save.is_none()
            && self.pending_entry.is_none()
            && self.integrated_starting.is_none()
            && self.native_lifecycle.epoch().is_some()
            && self.client_quit.get() == ClientQuit::Running;
        let capture_busy = self.screenshot_sequence.is_some()
            || self.pending_screenshot.is_some()
            || self.repro.replaying()
            || self.tick_script.playing()
            || !self.ui.screenshot_writes.is_empty()
            || self
                .renderer
                .as_ref()
                .is_some_and(|r| r.screenshot_in_flight());
        check_start_context(occupied, max_texture, world_ready, capture_busy)
    }

    pub(super) fn stop_export(&mut self) -> Result<(), ExportError> {
        self.native_export.request_stop(self.export.as_mut())
    }

    pub(super) fn finish_mod_export(&mut self, status: ExportStatus) {
        let (request, state, frames) = (status.request, status.state, status.frame);
        self.native_export.last = Some(status);
        self.native_export.ending = true;
        self.native_export_ended(request, state, frames);
        self.native_export.ending = false;
    }
}

pub(super) fn status(
    request: u32,
    spec: &ExportSpec,
    state: ExportState,
    frame: u32,
    bytes: u64,
) -> ExportStatus {
    mod_api::__private::built::export_status(
        request,
        spec.name.clone(),
        state,
        [frame, spec.frames],
        spec.size,
        spec.fps,
        bytes,
    )
}

fn check_start_context(
    occupied: bool,
    max_texture: Option<u32>,
    world_ready: bool,
    capture_busy: bool,
) -> Result<u32, ExportError> {
    if occupied {
        return Err(ExportError::Busy);
    }
    let max_texture = max_texture.ok_or(ExportError::Unavailable)?;
    if !world_ready {
        return Err(ExportError::WorldBusy);
    }
    if capture_busy {
        return Err(ExportError::Busy);
    }
    Ok(max_texture)
}

fn export_spec(request: &ExportRequest, max_texture: u32) -> Result<ExportSpec, ExportError> {
    let spec = ExportSpec {
        name: request.name.clone(),
        size: [request.width, request.height],
        fps: (request.fps_num, request.fps_den),
        frames: request.frames,
        format: match request.format {
            ExportFormat::PngSequence => Format::PngSequence,
            ExportFormat::Encoder => Format::Encoder,
            _ => return Err(ExportError::Unavailable),
        },
        hud: request.hud,
    };
    spec.validate(max_texture).map_err(|error| match error {
        SpecError::InvalidName => ExportError::InvalidName,
        SpecError::InvalidSize => ExportError::InvalidSize,
        SpecError::InvalidFps | SpecError::TooManyFrames => ExportError::InvalidRate,
    })?;
    Ok(spec)
}

fn prepare_output(
    game: &std::path::Path,
    budget: &Arc<ByteBudget>,
    spec: &ExportSpec,
) -> Result<(Arc<ExportFolder>, Option<EncoderPipe>), ExportError> {
    check_budget(budget, spec)?;
    let encoder = match spec.format {
        Format::PngSequence => None,
        Format::Encoder => {
            let config =
                EncoderConfig::load(game, false).map_err(|_| ExportError::EncoderUnavailable)?;
            EncoderPipe::check_config(&config).map_err(|_| ExportError::EncoderUnavailable)?;
            Some(config)
        }
    };
    let charge = budget
        .reserve(MOD_EXPORT_START_CHARGE)
        .map_err(|_| ExportError::BudgetExhausted)?;
    let (folder, encoder) = if let Some(config) = encoder {
        let (folder, pipe) =
            EncoderPipe::spawn_for_mod(game, spec, &config, Arc::clone(budget), charge)
                .map_err(encoder_error)?;
        (folder, Some(pipe))
    } else {
        (
            ExportFolder::create_charged(game, &spec.name, charge).map_err(folder_error)?,
            None,
        )
    };
    Ok((Arc::new(folder), encoder))
}

fn check_budget(budget: &ByteBudget, spec: &ExportSpec) -> Result<(), ExportError> {
    let estimate =
        u64::from(spec.size[0]) * u64::from(spec.size[1]) * 3 * u64::from(spec.frames.max(1));
    if budget.fits(MOD_EXPORT_START_CHARGE + estimate) {
        Ok(())
    } else {
        Err(ExportError::BudgetExhausted)
    }
}

fn folder_error(error: DirError) -> ExportError {
    match error {
        DirError::InvalidName => ExportError::InvalidName,
        DirError::Exists => ExportError::Exists,
        DirError::Folder(_) => ExportError::Folder,
    }
}

fn encoder_error(error: EncoderStartError) -> ExportError {
    match error {
        EncoderStartError::Destination(error) => folder_error(error),
        EncoderStartError::Folder(_) => ExportError::Folder,
        EncoderStartError::Encoder(_) => ExportError::EncoderUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_context_reports_busy_world_and_unavailable_clients() {
        let max_texture = Some(8192);
        assert_eq!(
            check_start_context(false, max_texture, true, false),
            Ok(8192)
        );
        assert_eq!(
            check_start_context(true, max_texture, true, false),
            Err(ExportError::Busy)
        );
        assert_eq!(
            check_start_context(false, max_texture, true, true),
            Err(ExportError::Busy)
        );
        assert_eq!(
            check_start_context(false, None, true, false),
            Err(ExportError::Unavailable)
        );
        assert_eq!(
            check_start_context(false, max_texture, false, false),
            Err(ExportError::WorldBusy)
        );
    }

    #[test]
    fn invalid_specifications_report_the_public_refusal() {
        let valid = ExportRequest::new("take", [1920, 1080], (30_000, 1001), 12);
        let mut request = valid.clone();
        request.name = "../take".into();
        assert_eq!(export_spec(&request, 8192), Err(ExportError::InvalidName));
        request = valid.clone();
        request.width = 17;
        assert_eq!(export_spec(&request, 8192), Err(ExportError::InvalidSize));
        assert_eq!(export_spec(&valid, 1024), Err(ExportError::InvalidSize));
        request = valid.clone();
        request.fps_den = 0;
        assert_eq!(export_spec(&request, 8192), Err(ExportError::InvalidRate));
        request = valid;
        request.frames = 216_001;
        assert_eq!(export_spec(&request, 8192), Err(ExportError::InvalidRate));
    }

    #[test]
    fn output_preflight_checks_budget_encoder_and_destination() {
        let game = tempfile::tempdir().unwrap();
        let mut spec = export_spec(
            &ExportRequest::new("take", [8192, 8192], (60, 1), 342),
            8192,
        )
        .unwrap();
        let state = NativeExport::default();
        let refused = |spec: &ExportSpec| prepare_output(game.path(), &state.budget, spec).err();

        // 342 8K RGB frames just exceed 64 GiB; 341 fit.
        assert_eq!(refused(&spec), Some(ExportError::BudgetExhausted));
        assert!(!game.path().join("exports").exists());
        spec.frames = 341;
        assert!(check_budget(&state.budget, &spec).is_ok());

        spec.format = Format::Encoder;
        assert_eq!(refused(&spec), Some(ExportError::EncoderUnavailable));
        spec.format = Format::PngSequence;
        prepare_output(game.path(), &state.budget, &spec).unwrap();
        assert_eq!(refused(&spec), Some(ExportError::Exists));

        let bad_game = game.path().join("file");
        std::fs::write(&bad_game, []).unwrap();
        assert_eq!(
            prepare_output(&bad_game, &state.budget, &spec).err(),
            Some(ExportError::Folder)
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_encoder_without_fd_output_leaves_the_take_name_and_budget_available() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let game = tempfile::tempdir().unwrap();
        let encoder = game.path().join("encoder");
        fs::write(
            &encoder,
            "#!/bin/sh\nprintf 'Input:\\n  fd\\nOutput:\\n  pipe\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&encoder, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(game.path().join("horizon-export.txt"), "encoder=encoder\n").unwrap();
        let state = NativeExport::default();
        let mut spec =
            export_spec(&ExportRequest::new("take", [16, 16], (30, 1), 1), 8192).unwrap();
        spec.format = Format::Encoder;

        assert_eq!(
            prepare_output(game.path(), &state.budget, &spec).err(),
            Some(ExportError::EncoderUnavailable)
        );
        assert!(!game.path().join("exports").exists());
        assert!(state.budget.fits(MOD_EXPORT_BUDGET));
        spec.format = Format::PngSequence;
        prepare_output(game.path(), &state.budget, &spec).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn encoder_launch_failure_refunds_the_charge_and_releases_the_take_name() {
        let game = tempfile::tempdir().unwrap();
        let encoder = encoder_that_disappears_after_probe(game.path());
        let state = NativeExport::default();
        let mut spec =
            export_spec(&ExportRequest::new("take", [16, 16], (30, 1), 1), 8192).unwrap();
        spec.format = Format::Encoder;

        assert_eq!(
            prepare_output(game.path(), &state.budget, &spec).err(),
            Some(ExportError::EncoderUnavailable)
        );
        assert!(!encoder.exists());
        assert_eq!(
            std::fs::read_dir(game.path().join("exports"))
                .unwrap()
                .count(),
            0
        );
        assert!(state.budget.fits(MOD_EXPORT_BUDGET));

        spec.format = Format::PngSequence;
        prepare_output(game.path(), &state.budget, &spec).unwrap();
        assert!(
            state
                .budget
                .fits(MOD_EXPORT_BUDGET - MOD_EXPORT_START_CHARGE)
        );
        assert!(
            !state
                .budget
                .fits(MOD_EXPORT_BUDGET - MOD_EXPORT_START_CHARGE + 1)
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_take_is_refused_before_encoder_launch_and_refunds_the_charge() {
        let game = tempfile::tempdir().unwrap();
        encoder_that_disappears_after_probe(game.path());
        let state = NativeExport::default();
        let mut spec =
            export_spec(&ExportRequest::new("take", [16, 16], (30, 1), 1), 8192).unwrap();
        spec.format = Format::Encoder;
        let take = ExportFolder::create(game.path(), "take").unwrap();
        let existing = take.path().join("video.mp4");
        std::fs::write(&existing, b"existing video").unwrap();

        assert_eq!(
            prepare_output(game.path(), &state.budget, &spec).err(),
            Some(ExportError::Exists)
        );
        assert!(state.budget.fits(MOD_EXPORT_BUDGET));
        assert_eq!(std::fs::read(existing).unwrap(), b"existing video");
        assert_eq!(
            std::fs::read_dir(game.path().join("exports"))
                .unwrap()
                .count(),
            1
        );
    }

    #[cfg(unix)]
    fn encoder_that_disappears_after_probe(game: &std::path::Path) -> std::path::PathBuf {
        use std::{fs, os::unix::fs::PermissionsExt};

        let encoder = game.join("encoder");
        // The probe succeeds, but the next process cannot launch this file.
        fs::write(
            &encoder,
            "#!/bin/sh\nrm -- \"$0\" || exit 1\nprintf 'Output:\\n  fd\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&encoder, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(game.join("horizon-export.txt"), "encoder=encoder\n").unwrap();
        encoder
    }
}
