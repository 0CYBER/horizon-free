//! The control socket's `screenshot`, waiting for the next drawn frame: it fails after
//! [`WAIT`] when no frame was drawn, naming why, instead of waiting for good (#19436).

use horizon_client::gfx::{NotDrawn, ScreenshotRequest};
use std::{
    path::PathBuf,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

/// How long a control screenshot waits for a drawn frame, in wall time.
pub(super) const WAIT: Duration = Duration::from_secs(5);

/// A control screenshot asked of the renderer, not yet answered.
pub(super) struct Pending {
    pub reply: Sender<String>,
    path: PathBuf,
    since: Instant,
}

/// Asks the renderer (`request`, its `screenshot`) for the next frame as `path`.
pub(super) fn begin(
    pending: &mut Option<Pending>,
    request: &mut Option<ScreenshotRequest>,
    path: PathBuf,
    reply: &Sender<String>,
    now: Instant,
) -> Result<(), &'static str> {
    if pending.is_some() {
        return Err("a screenshot is already pending");
    }
    *request = Some(ScreenshotRequest {
        path: path.clone(),
        size: None,
        scaled: false,
        background: false,
    });
    *pending = Some(Pending {
        reply: reply.clone(),
        path,
        since: now,
    });
    Ok(())
}

/// Fails the pending screenshot once it has waited [`WAIT`], and takes its request back from
/// the renderer, so no later frame answers a newer request with it. Its readback is read in
/// the frame that takes it and answered before the control socket is polled again, so none is
/// in flight here. A request in the slot that isn't this one (F2's, the world icon's) stays.
pub(super) fn expire(
    pending: &mut Option<Pending>,
    request: Option<&mut Option<ScreenshotRequest>>,
    not_drawn: Option<NotDrawn>,
    now: Instant,
) {
    let Some(waited) = pending.take_if(|p| now.duration_since(p.since) >= WAIT) else {
        return;
    };
    if let Some(request) = request {
        request.take_if(|r| !r.background && r.path == waited.path);
    }
    let _ = waited
        .reply
        .send(serde_json::json!({ "error": error(not_drawn) }).to_string());
}

fn error(not_drawn: Option<NotDrawn>) -> String {
    let secs = WAIT.as_secs();
    match not_drawn {
        Some(why) => format!(
            "screenshot: no frame was drawn in {secs} s ({})",
            why.as_str()
        ),
        // Frames are drawn, and none took it: a renderer made again dropped the request.
        None => format!("screenshot: no frame took the request in {secs} s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshot_no_frame_takes_fails_with_the_reason_and_frees_the_slot() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (mut pending, mut request) = (None, None);
        let start = Instant::now();
        begin(&mut pending, &mut request, "a.png".into(), &tx, start).unwrap();
        assert_eq!(
            begin(&mut pending, &mut request, "b.png".into(), &tx, start),
            Err("a screenshot is already pending")
        );

        let refused = Some(NotDrawn::Suspended);
        expire(&mut pending, Some(&mut request), refused, start + WAIT / 2);
        assert!(pending.is_some() && request.is_some());
        assert!(rx.try_recv().is_err());

        expire(&mut pending, Some(&mut request), refused, start + WAIT);
        let reply: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(
            reply["error"],
            "screenshot: no frame was drawn in 5 s (suspended)"
        );
        assert!(pending.is_none() && request.is_none());

        let later = start + WAIT * 2;
        begin(&mut pending, &mut request, "c.png".into(), &tx, later).unwrap();
        assert_eq!(request.as_ref().unwrap().path, PathBuf::from("c.png"));
    }

    #[test]
    fn expiry_leaves_another_request_in_the_renderer() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (mut pending, mut request) = (None, None);
        let start = Instant::now();
        begin(&mut pending, &mut request, "a.png".into(), &tx, start).unwrap();
        // A background request (F2's, the world icon's) to the same path took the slot meanwhile.
        request = Some(ScreenshotRequest {
            path: "a.png".into(),
            size: Some([64, 64]),
            scaled: false,
            background: true,
        });
        expire(&mut pending, Some(&mut request), None, start + WAIT);
        assert!(pending.is_none());
        let kept = request.expect("the background request is still queued");
        assert!(kept.background);
        assert_eq!(kept.path, PathBuf::from("a.png"));
    }
}
