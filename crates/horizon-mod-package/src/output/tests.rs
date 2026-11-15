use super::*;
use std::{sync::Mutex, time::Duration};
struct Stalled {
    gate: Option<Receiver<()>>,
    entered: mpsc::Sender<()>,
    bytes: Arc<Mutex<Vec<u8>>>,
}
impl Write for Stalled {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(gate) = self.gate.take() {
            self.entered.send(()).unwrap();
            gate.recv().map_err(|_| io::ErrorKind::BrokenPipe)?;
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct End(os_pipe::PipeReader, mpsc::Sender<()>);
impl Read for End {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.0.read(bytes)?;
        if count == 0 {
            self.1.send(()).unwrap();
        }
        Ok(count)
    }
}
#[test]
fn stalled_disk_retains_oldest_queued_bytes_and_never_blocks_the_game_pipe() {
    let (reader, mut writer) = os_pipe::pipe().unwrap();
    let (release, gate) = mpsc::channel();
    let (entered, stalled) = mpsc::channel();
    let (ended, eof) = mpsc::channel();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let done = drain(
        End(reader, ended),
        Stalled {
            gate: Some(gate),
            entered,
            bytes: bytes.clone(),
        },
        "stall-test",
    )
    .unwrap();
    let (produced, progress) = mpsc::channel();
    let input: Vec<u8> = (0..256)
        .flat_map(|index| format!("{index:08x}").repeat(2048).into_bytes())
        .collect();
    let expected = input.clone();
    // Establish the disk stall before filling the queue: otherwise an initially unscheduled
    // disk worker can free a slot after overflow and legitimately retain a later frame.
    writer.write_all(&input[..1]).unwrap();
    stalled.recv_timeout(Duration::from_secs(2)).unwrap();
    let producer = std::thread::spawn(move || {
        writer.write_all(&input[1..]).unwrap();
        produced.send(()).unwrap();
    });
    let prompt = progress.recv_timeout(Duration::from_secs(2)).is_ok()
        && eof.recv_timeout(Duration::from_secs(2)).is_ok();
    release.send(()).unwrap();
    producer.join().unwrap();
    done.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        prompt,
        "game-side write blocked behind the stalled filesystem"
    );
    let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    let mut chunks = text.split("\n[horizon.output.dropped_bytes=");
    let mut retained = chunks.next().unwrap().to_owned();
    let mut dropped = 0;
    for chunk in chunks {
        let (count, rest) = chunk.split_once("]\n").unwrap();
        dropped += count.parse::<usize>().unwrap();
        retained.push_str(rest);
    }
    assert!(dropped > 0);
    assert_eq!(retained.len() + dropped, 4 * 1024 * 1024);
    assert!(
        retained.len() <= 9 * 16 * 1024,
        "queue exceeded its byte budget"
    );
    assert_eq!(
        retained.as_bytes(),
        &expected[..retained.len()],
        "retained newer rather than oldest queued input"
    );
}
#[test]
fn loss_notice_crosses_a_real_rotation_boundary_under_the_file_cap() {
    let cap = 16 * 1024 * 1024;
    let root = tempfile::tempdir().unwrap();
    let directory = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    for name in ["stdout.log", "stderr.log"] {
        std::fs::File::create(root.path().join(name))
            .unwrap()
            .set_len(cap - 8)
            .unwrap();
        let sink = Rotating::open(directory.try_clone().unwrap(), name);
        assert!(sink.is_ok());
        let mut sink = sink.unwrap();
        let marker = b"\n[horizon.output.dropped_bytes=42]\n";
        notice(&mut sink, &AtomicU64::new(42));
        assert!(sink.file.metadata().unwrap().len() <= cap);
        let previous = std::fs::read(root.path().join(format!("{name}.1"))).unwrap();
        assert_eq!(previous.len(), cap as usize);
        assert_eq!(&previous[previous.len() - 8..], &marker[..8]);
        assert_eq!(std::fs::read(root.path().join(name)).unwrap(), &marker[8..]);
        io::copy(&mut io::repeat(b'B').take(2 * cap + 23), &mut sink).unwrap();
        let previous = std::fs::read(root.path().join(format!("{name}.1"))).unwrap();
        assert_eq!(previous.len(), cap as usize);
        assert!(previous.iter().all(|&byte| byte == b'B'));
        assert_eq!(
            std::fs::read(root.path().join(name)).unwrap(),
            vec![b'B'; marker.len() - 8 + 23]
        );
    }
}
#[test]
fn flushes_sink_and_survives_io_error() {
    struct Failing(Arc<std::sync::atomic::AtomicBool>);
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(5))
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (reader, mut writer) = os_pipe::pipe().unwrap();
    let done = drain(reader, Failing(flag.clone()), "error-test").unwrap();
    writer.write_all(b"pending").unwrap();
    drop(writer);
    done.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(flag.load(Ordering::SeqCst));
}
