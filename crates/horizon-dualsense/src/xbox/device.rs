//! The pad over hidapi and Windows.Gaming.Input, and the thread that writes to it: writes block
//! (a Bluetooth report can take milliseconds), so the game only hands over what the pad should
//! do.
#[cfg(windows)]
use crate::xbox::wgi;
use crate::xbox::{Motors, Target};
#[cfg(not(target_os = "ios"))]
use crate::xbox::{bluetooth, is_xbox_one, usb};
#[cfg(not(target_os = "ios"))]
use hidapi::{BusType, HidApi, HidDevice, HidError};
#[cfg(not(target_os = "ios"))]
use std::fmt;
use std::sync::Arc;
#[cfg(not(target_os = "ios"))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
#[cfg(not(target_os = "ios"))]
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;
#[cfg(not(target_os = "ios"))]
use std::time::Instant;

/// Why a pad could not take a report.
#[cfg(not(target_os = "ios"))]
#[derive(Debug)]
pub enum Error {
    /// hidapi refused the report: a pad that went away, an endpoint another process holds.
    Hid(HidError),
    /// Windows.Gaming.Input refused it.
    #[cfg(windows)]
    Wgi(windows::core::Error),
}

#[cfg(not(target_os = "ios"))]
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Hid(e) => write!(f, "{e}"),
            #[cfg(windows)]
            Error::Wgi(e) => write!(f, "Windows.Gaming.Input: {e}"),
        }
    }
}

#[cfg(not(target_os = "ios"))]
impl std::error::Error for Error {}

#[cfg(not(target_os = "ios"))]
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(not(target_os = "ios"))]
impl From<HidError> for Error {
    fn from(e: HidError) -> Self {
        Error::Hid(e)
    }
}

#[cfg(windows)]
impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Wgi(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
}

/// Whether a pad on this transport takes a HID output report on this OS. On Windows a wired pad's
/// HID endpoint is XGIP's stand-in, which takes none (SDL_hidapi_xboxone.c skips every
/// `\\?\HID#` path there), so Windows.Gaming.Input is the way to its trigger motors ([`wgi`]);
/// one on Bluetooth is a real HID device, which Chromium's own Xbox HID writer drives on every OS.
pub fn output_supported(transport: Transport) -> bool {
    !(cfg!(target_os = "windows") && transport == Transport::Usb)
}

/// An open Xbox One pad: its HID endpoint, or Windows.Gaming.Input on Windows.
#[cfg(not(target_os = "ios"))]
pub struct Xbox {
    backend: Backend,
    transport: Transport,
}

#[cfg(not(target_os = "ios"))]
enum Backend {
    Hid(HidDevice),
    #[cfg(windows)]
    Wgi(wgi::Pad),
}

#[cfg(not(target_os = "ios"))]
impl Xbox {
    /// The pad `target` names, on the link it is on. Windows.Gaming.Input says which link the
    /// pad is on: a wired pad goes through it (its HID endpoint takes no report), and a paired
    /// Bluetooth entry that Windows still lists after the cable goes in can't win. A pad on
    /// Bluetooth takes its own HID report when that opens, as on Linux and macOS, and WGI
    /// otherwise. [`Target::ANY`] keeps the old choice, the first Microsoft Xbox One pad; a
    /// third-party target reaches WGI even though its ids aren't Microsoft's. `None` when there
    /// is none, an error when it can't be opened (Linux without access to its hidraw node).
    pub fn open(api: &HidApi, target: Target) -> Option<Result<Self>> {
        static SKIPPED: AtomicBool = AtomicBool::new(false);
        let transport = |d: &hidapi::DeviceInfo| match d.bus_type() {
            BusType::Bluetooth => Transport::Bluetooth,
            _ => Transport::Usb,
        };
        // The matches, wired first: a pad on a cable can still be listed over Bluetooth for a
        // moment. `index` picks among identical pads of this model.
        let mut pads: Vec<&hidapi::DeviceInfo> = api
            .device_list()
            .filter(|d| {
                is_xbox_one(d.vendor_id(), d.product_id())
                    && target.matches(d.vendor_id(), d.product_id())
            })
            .collect();
        pads.sort_by_key(|d| transport(d) == Transport::Bluetooth);
        let rank = if target == Target::ANY {
            0
        } else {
            target.index
        };
        let chosen = pads.get(rank).or_else(|| pads.first()).copied();
        let skipped = chosen.is_some_and(|info| !output_supported(transport(info)));
        if skipped && !SKIPPED.swap(true, Ordering::Relaxed) {
            log::info!("Horizon: a wired Xbox pad has no output on this system");
        }
        #[cfg(windows)]
        {
            match wgi::Pad::open(target) {
                // A cable: WGI is the only way to its trigger motors.
                Ok(Some(pad)) if !pad.wireless() => {
                    return Some(Ok(Xbox {
                        backend: Backend::Wgi(pad),
                        transport: Transport::Usb,
                    }));
                }
                // Bluetooth: the pad's own report is the proven path for its triggers. WGI takes
                // the pad when that endpoint can't be opened (another process holds it).
                Ok(Some(pad)) => {
                    if let Some(info) = chosen
                        && transport(info) == Transport::Bluetooth
                        && let Ok(device) = info.open_device(api)
                    {
                        return Some(Ok(Xbox {
                            backend: Backend::Hid(device),
                            transport: Transport::Bluetooth,
                        }));
                    }
                    // Another process holds the endpoint (the launcher): WGI takes the pad.
                    return Some(Ok(Xbox {
                        backend: Backend::Wgi(pad),
                        transport: Transport::Bluetooth,
                    }));
                }
                Ok(None) => {}
                Err(e) => log::debug!("Horizon: Windows.Gaming.Input: {e}"),
            }
        }
        if let Some(info) = chosen {
            let transport = transport(info);
            if output_supported(transport) {
                match info.open_device(api) {
                    Ok(device) => {
                        return Some(Ok(Xbox {
                            backend: Backend::Hid(device),
                            transport,
                        }));
                    }
                    Err(e) => return Some(Err(e.into())),
                }
            }
        }
        None
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    pub fn write(&mut self, motors: Motors) -> Result<()> {
        match &mut self.backend {
            Backend::Hid(device) => {
                match self.transport {
                    Transport::Usb => device.write(&usb(motors))?,
                    Transport::Bluetooth => device.write(&bluetooth(motors))?,
                };
            }
            #[cfg(windows)]
            Backend::Wgi(pad) => pad.write(motors)?,
        };
        Ok(())
    }
}

/// A vibration: the motors' levels, each for a while, in order.
pub type Rumble = Vec<(Motors, Duration)>;

// On iOS no thread reads the queue (the Link never opens); the variants stay so the
// Link's API is the same everywhere.
#[cfg_attr(target_os = "ios", allow(dead_code))]
enum Message {
    Rumble(Rumble),
    Scan,
}

/// The output thread. Dropping it stops the motors before it ends; on iOS there is none (no
/// hidapi backend) and the Link stays closed.
pub struct Link {
    tx: Option<Sender<Message>>,
    /// 0: no pad open, else 1 + the transport.
    open: Arc<AtomicU8>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Start the thread and look for the pad `target` names.
    #[cfg(not(target_os = "ios"))]
    pub fn start(target: Target) -> Link {
        let (tx, rx) = mpsc::channel();
        let open = Arc::new(AtomicU8::new(0));
        let shown = open.clone();
        let thread = std::thread::Builder::new()
            .name("xbox".into())
            .spawn(move || Writer::new(shown, target).run(rx))
            .map_err(|e| log::warn!("Horizon: Xbox output: {e}"))
            .ok();
        Link {
            tx: Some(tx),
            open,
            thread,
        }
    }

    /// No output on iOS: hidapi has no backend there. The Link never opens.
    #[cfg(target_os = "ios")]
    pub fn start(_target: Target) -> Link {
        Link {
            tx: None,
            open: Arc::new(AtomicU8::new(0)),
            thread: None,
        }
    }

    /// The pad's transport while one is open and taking reports.
    pub fn open(&self) -> Option<Transport> {
        match self.open.load(Ordering::Relaxed) {
            1 => Some(Transport::Usb),
            2 => Some(Transport::Bluetooth),
            _ => None,
        }
    }

    /// Look for the pad again (a controller connected).
    pub fn scan(&self) {
        self.send(Message::Scan);
    }

    /// Play a vibration, replacing the one playing.
    pub fn rumble(&self, rumble: Rumble) {
        self.send(Message::Rumble(rumble));
    }

    fn send(&self, message: Message) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(message);
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // Closing the channel ends the thread, which stops the motors first.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Looks for the pad this many times, a second apart, after a scan finds nothing (its hidraw
/// node may appear after the controller does).
#[cfg(not(target_os = "ios"))]
const SCAN_TRIES: u32 = 5;

#[cfg(not(target_os = "ios"))]
struct Writer {
    api: Option<HidApi>,
    pad: Option<Box<dyn Handle>>,
    open: Arc<AtomicU8>,
    /// The pad this writer drives, not just the first one there is.
    target: Target,
    rumble: Rumble,
    /// When the vibration's current step ends.
    step_end: Option<Instant>,
    /// The motors last written; `None` after (re)opening.
    sent: Option<Motors>,
    scans_left: u32,
    next_scan: Instant,
}

/// What the writer needs from an open pad. It is a trait so the state machine can be tested
/// without hardware.
#[cfg(not(target_os = "ios"))]
trait Handle: Send {
    fn write(&mut self, motors: Motors) -> std::result::Result<(), String>;
}

#[cfg(not(target_os = "ios"))]
impl Handle for Xbox {
    fn write(&mut self, motors: Motors) -> std::result::Result<(), String> {
        Xbox::write(self, motors).map_err(|e| e.to_string())
    }
}

#[cfg(not(target_os = "ios"))]
impl Writer {
    fn new(open: Arc<AtomicU8>, target: Target) -> Self {
        Writer {
            api: None,
            pad: None,
            open,
            target,
            rumble: Vec::new(),
            step_end: None,
            sent: None,
            scans_left: 1,
            next_scan: Instant::now(),
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Message>) {
        loop {
            let now = Instant::now();
            if self.pad.is_none() && self.scans_left > 0 && now >= self.next_scan {
                self.scan();
            }
            let wake = [
                self.step_end,
                (self.pad.is_none() && self.scans_left > 0).then_some(self.next_scan),
            ]
            .into_iter()
            .flatten()
            .min();
            let message = match wake {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(now)),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match message {
                Ok(Message::Rumble(rumble)) => {
                    self.rumble = rumble;
                    self.rumble.reverse();
                    self.step_end = None;
                }
                Ok(Message::Scan) => {
                    // A pad that was unplugged while idle is found again by a write that fails;
                    // a hidapi zero-timeout read is not a liveness check on Windows (its overlapped
                    // read reports ERROR_BUSY instead of "no report yet"), so keep the handle and
                    // let the write judge it.
                    if self.pad.is_none() {
                        self.scans_left = SCAN_TRIES;
                        self.next_scan = Instant::now();
                    }
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.advance_rumble();
            self.flush();
        }
        if let Some(pad) = self.pad.as_mut() {
            let _ = pad.write(Motors::STOP);
        }
    }

    fn scan(&mut self) {
        self.scans_left -= 1;
        self.next_scan = Instant::now() + Duration::from_secs(1);
        let api = match self.api.as_mut() {
            Some(api) => api.refresh_devices().map(|()| &*api),
            None => HidApi::new().map(|api| &*self.api.insert(api)),
        };
        let found = match api {
            Ok(api) => Xbox::open(api, self.target),
            Err(e) => {
                log::warn!("Horizon: Xbox output: {e}");
                None
            }
        };
        match found {
            Some(Ok(pad)) => {
                log::info!("Xbox output over {:?}", pad.transport());
                self.open
                    .store(1 + pad.transport() as u8, Ordering::Relaxed);
                self.pad = Some(Box::new(pad));
                self.sent = None;
                self.scans_left = 0;
                self.flush();
            }
            Some(Err(e)) => {
                // No access (Linux without a udev rule): vibration stays with gilrs.
                log::info!("Horizon: the Xbox pad can't be opened for output: {e}");
                self.scans_left = 0;
            }
            None => {}
        }
    }

    /// Move to the vibration's next step once this one is over.
    fn advance_rumble(&mut self) {
        let now = Instant::now();
        match self.step_end {
            Some(end) if end > now => return,
            Some(_) => {
                self.rumble.pop();
            }
            None => {}
        }
        self.step_end = self.rumble.last().map(|&(_, length)| now + length);
    }

    /// Send what changed; a failed write means the pad is gone.
    fn flush(&mut self) {
        if self.pad.is_none() {
            return;
        }
        let motors = self
            .rumble
            .last()
            .map_or(Motors::STOP, |&(motors, _)| motors);
        if self.sent == Some(motors) {
            return;
        }
        if self.write(motors) {
            self.sent = Some(motors);
        }
    }

    /// Write a report, dropping the pad on any error: a handle that failed is never written
    /// through again. A zero-timeout read is *not* used as a liveness check first: on Windows
    /// hidapi's overlapped read reports `ERROR_BUSY` (0xAA) whenever no input report is ready,
    /// which closed the link on the first vibration over Bluetooth. hidapi's write returns the
    /// error itself, so a stale handle is dropped then.
    fn write(&mut self, motors: Motors) -> bool {
        let result = match self.pad.as_mut() {
            Some(pad) => pad.write(motors),
            None => return false,
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                self.lost(&e);
                false
            }
        }
    }
    /// Drop a handle that hidapi failed on, and look for the pad again: one that slept or was
    /// unplugged may come back. Nothing is written through the old handle.
    fn lost(&mut self, why: &str) {
        log::info!("Xbox output closed: {why}");
        self.pad = None;
        self.sent = None;
        self.open.store(0, Ordering::Relaxed);
        self.scans_left = SCAN_TRIES;
        self.next_scan = Instant::now();
    }
}

#[cfg(all(test, not(target_os = "ios")))]
mod tests {
    use super::*;

    /// A pad whose writes can fail, so the writer's state machine runs without hardware.
    struct FakePad {
        writes: Arc<std::sync::Mutex<Vec<Motors>>>,
        write_fails: bool,
    }

    impl Handle for FakePad {
        fn write(&mut self, motors: Motors) -> std::result::Result<(), String> {
            self.writes.lock().unwrap().push(motors);
            if self.write_fails {
                Err("gone".into())
            } else {
                Ok(())
            }
        }
    }

    fn writer(
        open: &Arc<AtomicU8>,
        write_fails: bool,
    ) -> (Writer, Arc<std::sync::Mutex<Vec<Motors>>>) {
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut writer = Writer::new(open.clone(), Target::ANY);
        writer.pad = Some(Box::new(FakePad {
            writes: writes.clone(),
            write_fails,
        }));
        (writer, writes)
    }

    #[test]
    fn a_failed_write_drops_the_pad_and_schedules_a_rescan() {
        let open = Arc::new(AtomicU8::new(1));
        let (mut writer, writes) = writer(&open, true);
        writer.flush();
        assert_eq!(writes.lock().unwrap().len(), 1);
        assert!(writer.pad.is_none());
        assert_eq!(open.load(Ordering::Relaxed), 0);
        assert!(writer.sent.is_none());
        assert_eq!(writer.scans_left, SCAN_TRIES);
        // The handle that failed is not written through again.
        writer.flush();
        assert_eq!(writes.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_live_pad_takes_the_motors_and_then_stops() {
        let open = Arc::new(AtomicU8::new(1));
        let (mut writer, writes) = writer(&open, false);
        let motors = Motors::levels(0.5, 0.25, [1.0, 0.0]);
        writer.rumble = vec![
            (Motors::STOP, Duration::from_millis(10)),
            (motors, Duration::from_millis(35)),
        ];
        writer.flush();
        assert_eq!(*writes.lock().unwrap(), vec![motors]);
        // The step is over: the writer stops the motors, and sends nothing more while stopped.
        writer.step_end = Some(Instant::now() - Duration::from_millis(1));
        writer.advance_rumble();
        writer.flush();
        writer.flush();
        assert_eq!(*writes.lock().unwrap(), vec![motors, Motors::STOP]);
        assert!(writer.pad.is_some());
        assert_eq!(open.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_vibration_replaces_the_one_playing() {
        let open = Arc::new(AtomicU8::new(1));
        let (mut writer, writes) = writer(&open, false);
        writer.rumble = vec![(
            Motors::levels(0.2, 0.0, [0.0; 2]),
            Duration::from_millis(100),
        )];
        writer.flush();
        writer.rumble = vec![(
            Motors::levels(0.9, 0.0, [0.0; 2]),
            Duration::from_millis(100),
        )];
        writer.step_end = Some(Instant::now() + Duration::from_millis(100));
        writer.flush();
        assert_eq!(
            *writes.lock().unwrap(),
            vec![
                Motors::levels(0.2, 0.0, [0.0; 2]),
                Motors::levels(0.9, 0.0, [0.0; 2])
            ]
        );
    }
}
