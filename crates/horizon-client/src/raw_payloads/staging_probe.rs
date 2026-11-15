//! Opt-in test observations after successful NETWORK preparation, before publication.
use std::sync::Mutex;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub work: usize,
    pub replies: usize,
}
struct Pause {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
struct Watched {
    channel: String,
    counts: Counts,
    endpoints: Vec<std::sync::Arc<super::endpoint::Observation>>,
    pause: Option<Pause>,
    failures: Vec<String>,
}
static WATCH: Mutex<Option<Watched>> = Mutex::new(None);
pub struct Watch;
impl Watch {
    pub fn begin(channel: &str) -> Self {
        let mut watch = WATCH.lock().unwrap();
        assert!(watch.is_none());
        *watch = Some(Watched {
            channel: channel.into(),
            counts: Counts::default(),
            endpoints: Vec::new(),
            pause: None,
            failures: Vec::new(),
        });
        Self
    }
    /// Live observations, not an atomic cross-queue snapshot. Observations retain
    /// only weak ownership handles, never connection owners or payload credit leases.
    pub fn diagnostics(&self) -> Vec<String> {
        let endpoints = match WATCH.try_lock() {
            Ok(watch) => watch
                .as_ref()
                .map(|w| w.endpoints.clone())
                .unwrap_or_default(),
            Err(_) => return vec!["watch busy/poisoned".into()],
        };
        endpoints
            .iter()
            .map(|observation| observation.diagnostics())
            .collect()
    }
    pub fn snapshot(&self) -> Counts {
        WATCH.lock().unwrap().as_ref().unwrap().counts
    }
    /// Stop inside the real guest's next successful host preparation. Both sides
    /// use channels so scheduling, not a wall-clock sleep, orders the proof.
    pub fn pause_next(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, observed) = std::sync::mpsc::channel();
        let (resume, release) = std::sync::mpsc::channel();
        WATCH.lock().unwrap().as_mut().unwrap().pause = Some(Pause { entered, release });
        (observed, resume)
    }
    pub fn failures(&self) -> Vec<String> {
        WATCH.lock().unwrap().as_ref().unwrap().failures.clone()
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        *WATCH.lock().unwrap() = None;
    }
}
pub(super) fn prepared(channel: &str, work: bool) {
    let pause = {
        let mut watch = WATCH.lock().unwrap();
        match watch.as_mut().filter(|watch| watch.channel == channel) {
            Some(watch) => {
                if work {
                    watch.counts.work += 1;
                } else {
                    watch.counts.replies += 1;
                }
                watch.pause.take()
            }
            None => None,
        }
    };
    if let Some(pause) = pause {
        pause.entered.send(()).unwrap();
        pause.release.recv().unwrap();
    }
}

pub(super) fn capture(channel: &str, endpoint: &super::Endpoint) {
    if let Some(watch) = WATCH.lock().unwrap().as_mut()
        && watch.channel == channel
        && !watch.endpoints.iter().any(|e| e.same_target(endpoint))
    {
        watch
            .endpoints
            .push(std::sync::Arc::new(endpoint.observation()));
    }
}

pub(super) fn failure(channel: &str, diagnostic: &str) {
    if let Some(watch) = WATCH.lock().unwrap().as_mut()
        && watch.channel == channel
    {
        watch.failures.push(diagnostic.into());
    }
}
