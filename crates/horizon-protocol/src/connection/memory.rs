use super::*;
use std::sync::{Condvar, Mutex};

// Capacity is flow control, not a limit on packet size.
const CAPACITY: usize = 64 * 1024;

/// Called when a [`Pipe`] end has bytes to read or was closed ([`Pipe::on_ready`]).
type Notify = Arc<dyn Fn() + Send + Sync>;

/// One direction of a [`Pipe`]: the bytes written and not read yet, and
/// whether either end closed it.
#[derive(Default)]
struct Channel {
    state: Mutex<(std::collections::VecDeque<u8>, bool)>,
    ready: Condvar,
    /// The reading end's [`Pipe::on_ready`].
    notify: Mutex<Option<Notify>>,
}
impl Channel {
    fn close(&self) {
        self.state.lock().unwrap().1 = true;
        self.ready.notify_all();
        self.notify();
    }
    /// Tell a reader that doesn't block, outside the state's lock.
    fn notify(&self) {
        let notify = self.notify.lock().unwrap().clone();
        if let Some(notify) = notify {
            notify();
        }
    }
}
/// The channels of one end; dropping the end's last clone closes them, as
/// a socket closes with its process.
struct End {
    incoming: Arc<Channel>,
    outgoing: Arc<Channel>,
}
impl Drop for End {
    fn drop(&mut self) {
        self.incoming.close();
        self.outgoing.close();
    }
}
/// One end of an in-memory byte pipe: what one end writes the other reads.
pub struct Pipe {
    end: Arc<End>,
    /// The read timeout (`set_read_timeout`).
    pub(super) timeout: Mutex<Option<Duration>>,
    /// Reads with nothing to read fail with `WouldBlock` instead of waiting.
    nonblocking: bool,
}
impl Pipe {
    /// Two connected ends.
    pub fn pair() -> (Self, Self) {
        let (a, b) = (Arc::new(Channel::default()), Arc::new(Channel::default()));
        let end = |incoming, outgoing| Self {
            end: Arc::new(End { incoming, outgoing }),
            timeout: Mutex::new(None),
            nonblocking: false,
        };
        (end(a.clone(), b.clone()), end(b, a))
    }
    pub(super) fn try_clone(&self) -> Self {
        Self {
            end: self.end.clone(),
            timeout: Mutex::new(*self.timeout.lock().unwrap()),
            nonblocking: self.nonblocking,
        }
    }
    /// Reads with nothing to read fail with `WouldBlock`, as a non-blocking socket's do.
    pub fn set_nonblocking(&mut self, nonblocking: bool) {
        self.nonblocking = nonblocking;
    }
    /// Call `notify` whenever bytes arrive, outgoing capacity is released, or
    /// the pipe closes. Also notify now if readable, writable, or closed.
    /// A non-blocking poller then reads and drains queued writes until
    /// `WouldBlock`; notifications run outside the channel state locks.
    pub fn on_ready(&self, notify: impl Fn() + Send + Sync + 'static) {
        let incoming = &self.end.incoming;
        *incoming.notify.lock().unwrap() = Some(Arc::new(notify));
        let state = incoming.state.lock().unwrap();
        let readable = !state.0.is_empty() || state.1;
        drop(state);
        let state = self.end.outgoing.state.lock().unwrap();
        let writable = state.0.len() < CAPACITY || state.1;
        drop(state);
        if readable || writable {
            incoming.notify();
        }
    }
    /// Close both directions: reads at either end then see the end of the stream.
    pub fn shutdown(&self) {
        self.end.incoming.close();
        self.end.outgoing.close();
    }
}
impl Read for Pipe {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let deadline = self.timeout.lock().unwrap().map(|t| Instant::now() + t);
        let mut state = self.end.incoming.state.lock().unwrap();
        if self.nonblocking && state.0.is_empty() && !state.1 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        while state.0.is_empty() && !state.1 {
            state = match deadline {
                None => self.end.incoming.ready.wait(state).unwrap(),
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(std::io::ErrorKind::TimedOut.into());
                    }
                    self.end.incoming.ready.wait_timeout(state, left).unwrap().0
                }
            };
        }
        let n = state.0.read(out)?;
        self.end.incoming.ready.notify_all();
        drop(state);
        if n != 0 {
            // The peer registered its callback on its incoming channel, which
            // is this end's outgoing channel. Wake it to drain queued writes.
            self.end.outgoing.notify();
        }
        Ok(n)
    }
}
impl Write for Pipe {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        let mut state = self.end.outgoing.state.lock().unwrap();
        while state.0.len() == CAPACITY && !state.1 {
            if self.nonblocking {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            state = self.end.outgoing.ready.wait(state).unwrap();
        }
        if state.1 {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        let n = data.len().min(CAPACITY - state.0.len());
        state.0.extend(&data[..n]);
        self.end.outgoing.ready.notify_all();
        drop(state);
        self.end.outgoing.notify();
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::{
        custom_payload::{CustomPayload, PayloadCodecs, PayloadFlow, PayloadProtocol},
        play,
    };

    #[test]
    fn draining_full_pipe_wakes_nonblocking_writer_without_reply() {
        let (mut writer, mut reader) = Pipe::pair();
        writer.set_nonblocking(true);
        writer.write_all(&vec![1; CAPACITY]).unwrap();
        assert_eq!(
            writer.write(&[2]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let (tx, rx) = std::sync::mpsc::channel();
        // Probe the released capacity from inside the callback as well: this
        // would deadlock if a drain notified while holding its state lock.
        let probe = writer.end.outgoing.clone();
        writer.on_ready(move || {
            let available = CAPACITY - probe.state.lock().unwrap().0.len();
            let _ = tx.send(available);
        });
        assert!(rx.try_recv().is_err());
        let mut bytes = [0; 3];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [1; 3]);
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), 3);
        assert_eq!(writer.write(&[2, 3, 4, 5]).unwrap(), 3);
        assert_eq!(
            writer.write(&[5]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(writer.end.incoming.state.lock().unwrap().0.is_empty());
    }

    #[test]
    fn registered_large_packet_and_detached_roundtrip() {
        for detached in [false, true] {
            let codecs = PayloadCodecs::from_channels([(
                "test:raw".into(),
                PayloadProtocol::Play,
                PayloadFlow::ToServer,
            )]);
            let packet = play::Serverbound::CustomPayload(CustomPayload {
                channel: "test:raw".into(),
                data: vec![0x39; frame::MAX_UNCOMPRESSED + 1],
            });
            let body = packet.try_encode_with_payload_codecs(&codecs).unwrap();
            let (a, b) = Pipe::pair();
            let sender = std::thread::spawn(move || {
                let mut c = Connection::local(a);
                c.set_compression(1);
                if detached {
                    let (Socket::Memory(mut pipe), _, mut writer) = c.detach() else {
                        panic!()
                    };
                    let contents = writer.contents(&body).unwrap();
                    let mut bytes = Vec::new();
                    writer.frame_into(&contents, &mut bytes);
                    pipe.write_all(&bytes).unwrap();
                } else {
                    c.send(&body).unwrap();
                    c.send(&[7, 8]).unwrap();
                }
            });
            let mut c = Connection::local(b);
            c.set_compression(1);
            let (id, data) = if detached {
                let (Socket::Memory(mut pipe), mut reader, _) = c.detach() else {
                    panic!()
                };
                loop {
                    if let Some(packet) = reader.buffered().unwrap() {
                        break packet;
                    }
                    assert!(reader.read_from(&mut pipe).unwrap() > 0);
                }
            } else {
                let packet = c.recv().unwrap();
                assert_eq!(c.recv().unwrap(), (7, vec![8]));
                packet
            };
            assert_eq!(
                play::Serverbound::decode_with_payload_codecs(id, &data, &codecs).unwrap(),
                packet
            );
            sender.join().unwrap();
        }
    }

    #[test]
    fn bounded_pipe_drains_and_shutdown_wakes_writer() {
        let (mut a, mut b) = Pipe::pair();
        assert_eq!(a.write(&vec![1; CAPACITY * 2]).unwrap(), CAPACITY);
        assert_eq!(a.end.outgoing.state.lock().unwrap().0.len(), CAPACITY);
        let closer = a.try_clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            tx.send(()).unwrap();
            a.write_all(&vec![2; CAPACITY * 2])
        });
        rx.recv().unwrap();
        let mut bytes = vec![0; CAPACITY];
        b.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, vec![1; CAPACITY]);
        b.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, vec![2; CAPACITY]);
        closer.shutdown();
        // The producer may finish its final chunk or see closure, but must wake.
        let _ = producer.join().unwrap();
    }

    #[test]
    fn closed_full_pipe_releases_blocked_writer() {
        let (mut a, b) = Pipe::pair();
        a.write_all(&vec![0; CAPACITY]).unwrap();
        let producer = std::thread::spawn(move || a.write_all(&[1]));
        drop(b);
        assert_eq!(
            producer.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
}
