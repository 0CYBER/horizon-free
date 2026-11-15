use super::*;
use crate::{local_control::Endpoint, locks::Release, selection::ModSelectionContext};
use std::{io::Read, process::Child};
pub struct Prepared {
    request: Request,
    _snapshots: Vec<Snapshot>,
    _claims: Vec<Claim>,
    _record: File,
}
impl Prepared {
    pub fn new(
        data: PathBuf,
        trees: Vec<(PathBuf, bool)>,
        program: Program,
        selection: &ModSelectionContext,
    ) -> io::Result<Self> {
        if !data.is_absolute() || !program.directory.is_absolute() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let registry = Registry::with_release(&data, Release::Close).map_err(lease_error)?;
        let mut snapshots = trees
            .into_iter()
            .map(|(path, complete)| Snapshot::capture(&path, complete))
            .collect::<io::Result<Vec<_>>>()?;
        let mut entry = Entry {
            version: crate::local_control::VERSION,
            data,
            trees: snapshots
                .iter()
                .map(|snapshot| snapshot.tree.clone())
                .collect(),
        };
        entry.validate(15)?;
        use cap_fs_ext::DirExt;
        use sha2::Digest;
        let directory = open_directory(&entry.data).map_err(lease_error)?;
        match directory.create_dir("run-records") {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
        let records = directory.open_dir_nofollow("run-records")?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(io::Error::other)?;
        let name = format!("{:x}", sha2::Sha256::digest(nonce));
        let mut builder = cap_std::fs::DirBuilder::new();
        builder.recursive(false);
        #[cfg(unix)]
        cap_std::fs::DirBuilderExt::mode(&mut builder, 0o700);
        records.create_dir_with(&name, &builder)?;
        let record = Snapshot::capture(&entry.data.join("run-records").join(name), false)?;
        entry.trees.push(record.tree.clone());
        snapshots.push(record);
        let claims = snapshots
            .iter()
            .map(|snapshot| {
                registry
                    .lock(&snapshot.directory, LockMode::Shared)
                    .map_err(lease_error)
            })
            .collect::<io::Result<_>>()?;
        entry.recheck()?;
        let (reference, file) =
            record::Reference::publish(snapshots.last().unwrap(), &entry, selection)?;
        Ok(Self {
            request: Request {
                entry,
                program,
                record: reference,
            },
            _snapshots: snapshots,
            _claims: claims,
            _record: file,
        })
    }
}
/// Capability bytes probe format support, never executable authenticity.
pub fn capable(path: &Path) -> io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0; 8192];
    let mut retained = 0;
    loop {
        let read = file.read(&mut buffer[retained..])?;
        if read == 0 {
            return Ok(false);
        }
        let length = retained + read;
        if buffer[..length]
            .windows(CAPABILITY.len())
            .any(|bytes| bytes == CAPABILITY)
        {
            return Ok(true);
        }
        retained = length.min(CAPABILITY.len() - 1);
        buffer.copy_within(length - retained..length, 0);
    }
}
pub enum Started {
    Legacy(Command),
    Holder { child: Child },
}
pub fn launch(
    mut holder: Command,
    marker: &Marker,
    prepared: &Prepared,
    deadline: Instant,
) -> io::Result<Started> {
    if cfg!(windows) || !capable(Path::new(holder.get_program()))? {
        return Ok(Started::Legacy(prepared.request.program.command()?));
    }
    prepared
        .request
        .entry
        .check_image(Path::new(holder.get_program()))?;
    let mut endpoint = Endpoint::bind(marker).map_err(|error| {
        io::Error::new(error.kind(), format!("bind holder control socket: {error}"))
    })?;
    holder.env(
        ENV,
        serde_json::to_string(marker).map_err(io::Error::other)?,
    );
    let child = holder.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "execute holder {}: {error}",
                Path::new(holder.get_program()).display()
            ),
        )
    })?;
    let handshake = (|| {
        let stream = endpoint.accept(deadline)?;
        let channel =
            Channel::authenticate(stream, marker, true, deadline)?.send(&prepared.request)?;
        let (channel, reply) = channel.receive::<Reply>()?;
        if !matches!(reply, Reply::Acquired) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        prepared.request.entry.recheck()?;
        channel.send(&Reply::Commit).map(drop)
    })();
    if let Err(error) = handshake {
        retire(child, deadline, Child::kill, |mut child| {
            let _ = child.wait();
        });
        return Err(error);
    }
    Ok(Started::Holder { child })
}

// Only the launch path polls within the remaining budget. The background owner only waits:
// it never signals a PID, and retains the unreaped Child even when termination failed.
pub(super) fn retire(
    mut child: Child,
    deadline: Instant,
    kill: impl FnOnce(&mut Child) -> io::Result<()>,
    reap: impl FnOnce(Child) + Send + 'static,
) {
    let _ = kill(&mut child);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::park_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5)),
        );
    }
    handoff(child, reap, |job| {
        std::thread::Builder::new()
            .name("holder-reaper".into())
            .spawn(job)
            .map(drop)
    });
}

pub(super) fn handoff<T: Send + 'static>(
    child: T,
    reap: impl FnOnce(T) + Send + 'static,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
) {
    let pending = std::sync::Arc::new(std::sync::Mutex::new(Some(child)));
    let worker = pending.clone();
    if let Err(error) = spawn(Box::new(move || {
        reap(worker.lock().unwrap().take().unwrap())
    })) {
        log::error!(
            "Cannot start holder-reaper: {error}; retaining unreaped child until process exit"
        );
        // The failed spawner dropped its closure; keep our remaining owner deliberately alive.
        std::mem::forget(pending);
    }
}
