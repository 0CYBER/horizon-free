use super::*;
use mod_api::prelude::*;
use mod_api::{ExportError, ExportFormat, ExportRequest, ExportState, ExportStatus};
use std::cell::{Cell, RefCell};

thread_local! {
    static ENDING: Cell<Option<(u32, ExportState, u32)>> = const { Cell::new(None) };
}

pub(super) struct ClientExportHooks;
impl ClientExportHooks {
    pub(super) fn export_ended(
        &self,
        host: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::ExportEndedEvent,
    ) {
        let status = host.export_status().expect("the settled export");
        assert_eq!(status.frame, event.frames);
        assert_eq!(status.state, event.state);
        assert_eq!(status.request, event.request);
        ENDING.set(Some((event.request, event.state, event.frames)));
    }
}

#[derive(Default)]
struct Client {
    status: Option<ExportStatus>,
    refused: Option<ExportError>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(export));

fn status(state: ExportState, frame: u32) -> ExportStatus {
    mod_api::__private::built::export_status(
        19,
        "test-take".into(),
        state,
        [frame, 76],
        [1080, 1920],
        (30_000, 1001),
        0x0123_4567_89ab_cdef,
    )
}

impl ExportClientHost for Client {
    fn start_export(&mut self, request: &ExportRequest) -> Result<u32, ExportError> {
        let mut expected = ExportRequest::new("test-take", [1080, 1920], (30_000, 1001), 76);
        if self.refused.is_none() {
            expected.format = ExportFormat::Encoder;
            expected.hud = true;
        }
        assert_eq!(request, &expected);
        if let Some(error) = self.refused {
            return Err(error);
        }
        self.status = Some(status(ExportState::Running, 0));
        Ok(19)
    }

    fn export_status(&self) -> Option<ExportStatus> {
        self.status.clone()
    }

    fn stop_export(&mut self) -> Result<(), ExportError> {
        let capture = self.status.as_mut().ok_or(ExportError::NotRunning)?;
        capture.state = ExportState::Finished;
        capture.frame = 12;
        Ok(())
    }
}

fn with_guest(client: &mut Client, test: impl FnOnce()) {
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the client's frame is live until the synchronous transport is removed.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    test();
    guest::set_transport(None);
}

#[test]
fn wasm_video_export_requests_status_errors_and_endings_cross_the_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut req = ExportRequest::new("test-take", [1080, 1920], (30_000, 1001), 76);
    let mut guest = guest::Guest;
    let mut client = Client::default();
    with_guest(&mut client, || {
        assert_eq!(guest.export_status(), None);
        assert_eq!(guest.stop_export(), Err(ExportError::NotRunning));
        req.format = ExportFormat::Encoder;
        req.hud = true;
        assert_eq!(guest.start_export(&req), Ok(19));
        assert_eq!(guest.export_status(), Some(status(ExportState::Running, 0)));
        assert_eq!(guest.stop_export(), Ok(()));
        assert_eq!(
            guest.export_status(),
            Some(status(ExportState::Finished, 12))
        );
    });

    req.format = ExportFormat::PngSequence;
    req.hud = false;
    for error in [
        ExportError::InvalidName,
        ExportError::InvalidSize,
        ExportError::InvalidRate,
        ExportError::Exists,
        ExportError::Busy,
        ExportError::BudgetExhausted,
        ExportError::EncoderUnavailable,
        ExportError::Unavailable,
        ExportError::WorldBusy,
        ExportError::NotRunning,
        ExportError::Folder,
    ] {
        // An unsuccessful start still carries the request's other format and exact error tag.
        client.refused = Some(error);
        with_guest(&mut client, || {
            assert_eq!(guest.start_export(&req), Err(error))
        });
    }

    let module = WasmMod {
        group: 0,
        subscribed: &[Some(mod_api::HookId::ExportEnded)],
        listeners: &[],
        corrupt: false,
    };
    for (state, frames) in [(ExportState::Finished, 12), (ExportState::Failed, 1)] {
        client.status = Some(status(state, frames));
        let mut event = mod_api::ExportEndedEvent {
            request: 19,
            state,
            frames,
        };
        module.export_ended(&mut client, &mut event);
        assert_eq!(ENDING.take(), Some((19, state, frames)));
    }
}
