//! The pad over hidapi, and the thread that writes to it: writes block (a Bluetooth report can
//! take milliseconds), so the game only hands over what the pad should do.
use crate::{Output, TRIGGER_OFF, TriggerEffect, Vibration, is_dualsense};
use hidapi::{BusType, HidApi, HidDevice, HidResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
}

/// An open DualSense.
pub struct DualSense {
    device: HidDevice,
    transport: Transport,
    vibration: Vibration,
    /// The Bluetooth report's sequence number.
    seq: u8,
}

impl DualSense {
    /// The first DualSense hidapi lists; `None` when there is none, an error when it can't be
    /// opened (Linux without access to its hidraw node).
    pub fn open(api: &HidApi) -> Option<HidResult<Self>> {
        let info = api
            .device_list()
            .find(|d| is_dualsense(d.vendor_id(), d.product_id()))?;
        let transport = match info.bus_type() {
            BusType::Bluetooth => Transport::Bluetooth,
            _ => Transport::Usb,
        };
        Some(info.open_device(api).map(|device| {
            let mut firmware = [0; 64];
            firmware[0] = 0x20;
            // The update version decides how vibration is asked for; without it, the kernel's
            // default.
            let vibration = match device.get_feature_report(&mut firmware) {
                Ok(n) if n >= 46 => {
                    Vibration::of_update_version(u16::from_le_bytes([firmware[44], firmware[45]]))
                }
                _ => Vibration::Legacy,
            };
            DualSense {
                device,
                transport,
                vibration,
                seq: 0,
            }
        }))
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    pub fn write(&mut self, out: &Output) -> HidResult<()> {
        match self.transport {
            Transport::Usb => self.device.write(&out.usb(self.vibration))?,
            Transport::Bluetooth => {
                let report = out.bluetooth(self.vibration, self.seq);
                self.seq = (self.seq + 1) & 0x0f;
                self.device.write(&report)?
            }
        };
        Ok(())
    }

    /// An input report (USB 0x01, Bluetooth 0x31), waiting up to `millis` (-1: forever).
    pub fn read(&self, buf: &mut [u8], millis: i32) -> HidResult<usize> {
        self.device.read_timeout(buf, millis)
    }
}

/// What the pad should show while nothing else is asked: the triggers' effects (left, right),
/// the lightbar and the player LEDs (`None`: the pad's own).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub triggers: [TriggerEffect; 2],
    pub lightbar: Option<[u8; 3]>,
    pub player_leds: Option<u8>,
}

impl Default for State {
    fn default() -> Self {
        State {
            triggers: [TRIGGER_OFF; 2],
            lightbar: None,
            player_leds: None,
        }
    }
}

/// A vibration: the motors' levels (left, right), each for a while, in order.
pub type Rumble = Vec<([u8; 2], Duration)>;

enum Message {
    State(State),
    Rumble(Rumble),
    Scan,
}

/// The writer thread. Dropping it frees the triggers and stops the motors before it ends.
pub struct Link {
    tx: Option<Sender<Message>>,
    /// 0: no pad open, else 1 + the transport.
    open: Arc<AtomicU8>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Start the thread and look for a DualSense.
    pub fn start() -> Link {
        let (tx, rx) = mpsc::channel();
        let open = Arc::new(AtomicU8::new(0));
        let shown = open.clone();
        let thread = std::thread::Builder::new()
            .name("dualsense".into())
            .spawn(move || Writer::new(shown).run(rx))
            .map_err(|e| log::warn!("Horizon: DualSense output: {e}"))
            .ok();
        Link {
            tx: Some(tx),
            open,
            thread,
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

    /// What the pad shows from now on; sent again whenever it reconnects.
    pub fn set(&self, state: State) {
        self.send(Message::State(state));
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
        // Closing the channel ends the thread, which releases the pad first.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Looks for the pad this many more times, a second apart, after a scan finds nothing (its
/// hidraw node may appear after the controller does).
const SCAN_TRIES: u32 = 5;

struct Writer {
    api: Option<HidApi>,
    pad: Option<DualSense>,
    open: Arc<AtomicU8>,
    state: State,
    /// What the pad was last sent; `None` after (re)opening.
    sent: Option<(State, [u8; 2])>,
    rumble: Rumble,
    /// When the vibration's current step ends.
    step_end: Option<Instant>,
    scans_left: u32,
    next_scan: Instant,
}

impl Writer {
    fn new(open: Arc<AtomicU8>) -> Self {
        Writer {
            api: None,
            pad: None,
            open,
            state: State::default(),
            sent: None,
            rumble: Vec::new(),
            step_end: None,
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
                Ok(Message::State(state)) => self.state = state,
                Ok(Message::Rumble(rumble)) => {
                    self.rumble = rumble;
                    self.rumble.reverse();
                    self.step_end = None;
                }
                Ok(Message::Scan) => {
                    // A pad that was unplugged while idle fails to read.
                    let mut buf = [0; 80];
                    if self
                        .pad
                        .as_ref()
                        .is_some_and(|pad| pad.read(&mut buf, 0).is_err())
                    {
                        self.lost("unplugged");
                    }
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
            let _ = pad.write(&Output::release());
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
            Ok(api) => DualSense::open(api),
            Err(e) => {
                log::warn!("Horizon: DualSense output: {e}");
                None
            }
        };
        match found {
            Some(Ok(pad)) => {
                log::info!("DualSense output over {:?}", pad.transport());
                self.open
                    .store(1 + pad.transport() as u8, Ordering::Relaxed);
                self.pad = Some(pad);
                self.sent = None;
                self.scans_left = 0;
                self.flush();
            }
            Some(Err(e)) => {
                // No access (Linux without a udev rule): vibration stays with gilrs.
                log::info!("Horizon: the DualSense can't be opened for output: {e}");
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
        let Some(pad) = self.pad.as_mut() else {
            return;
        };
        let motors = self.rumble.last().map_or([0, 0], |(m, _)| *m);
        let (state, sent) = (self.state, self.sent);
        if sent == Some((state, motors)) {
            return;
        }
        let mut out = Output {
            rumble: (sent.is_none_or(|(_, m)| m != motors)).then_some(motors),
            release_leds: sent.is_none() && state.lightbar.is_some(),
            ..Output::default()
        };
        let old = sent.map(|(s, _)| s);
        if old.is_none_or(|s| s.triggers != state.triggers) {
            out.left_trigger = Some(state.triggers[0]);
            out.right_trigger = Some(state.triggers[1]);
        }
        if old.is_none_or(|s| s.lightbar != state.lightbar) {
            out.lightbar = state.lightbar;
        }
        if old.is_none_or(|s| s.player_leds != state.player_leds) {
            out.player_leds = state.player_leds;
        }
        match pad.write(&out) {
            Ok(()) => self.sent = Some((state, motors)),
            Err(e) => self.lost(&e.to_string()),
        }
    }

    fn lost(&mut self, why: &str) {
        log::info!("DualSense output closed: {why}");
        self.pad = None;
        self.open.store(0, Ordering::Relaxed);
    }
}
