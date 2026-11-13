//! The writer thread: what a folder queues to be written, and waiting for it to be on disk.
use super::*;

mod saved_portals;

/// The background thread that encodes chunks and writes them into the region files, in
/// submission order.
pub(super) struct Writer {
    send: mpsc::Sender<WriteBatch>,
    thread: std::thread::JoinHandle<()>,
}

/// Writes still queued or running, with a condition to wait for none.
#[derive(Default)]
pub(super) struct Outstanding {
    count: Mutex<usize>,
    chunks: AtomicUsize,
    pub(super) error: Mutex<Option<String>>,
    done: Condvar,
    /// What the folder's writes make stale: the portal blocks a search read from saved chunks.
    portals: saved_portals::SavedPortals,
}

impl SaveDir {
    /// Queue `records` for the writer thread; with `wait`, return once
    /// they and every earlier write are on disk.
    pub(super) fn write_records(
        &self,
        folder: &str,
        records: Vec<((i32, i32), Option<NamedTag>)>,
        wait: bool,
    ) -> Result<()> {
        let records = records
            .into_iter()
            .map(|(k, t)| (k, t.map_or(Record::Delete, |t| Record::Tag(Arc::new(t)))))
            .collect();
        self.queue(Folder::of(folder), records, wait)
    }
    pub(super) fn queue(
        &self,
        folder: Folder,
        records: Vec<((i32, i32), Record)>,
        wait: bool,
    ) -> Result<()> {
        if !records.is_empty() {
            let mut batch = Vec::with_capacity(records.len());
            {
                let mut pending = self.pending.lock().map_err(|_| "pending writes poisoned")?;
                for (key, record) in records {
                    let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
                    pending.insert((folder, key.0, key.1), (seq, record.clone()));
                    batch.push((key, record, seq));
                }
            }
            // After the pending writes: a search reads the new chunks from here on.
            self.outstanding
                .portals
                .written(folder, batch.iter().map(|(key, _, _)| *key));
            *self
                .outstanding
                .count
                .lock()
                .map_err(|_| "writer poisoned")? += 1;
            let mut writer = self.writer.lock().map_err(|_| "writer poisoned")?;
            let writer = writer.get_or_insert_with(|| self.start_writer());
            let chunks = if folder == Folder::Region {
                batch.len()
            } else {
                0
            };
            self.outstanding.chunks.fetch_add(chunks, Ordering::Relaxed);
            if writer.send.send((folder, batch)).is_err() {
                self.outstanding.chunks.fetch_sub(chunks, Ordering::Relaxed);
                *self
                    .outstanding
                    .error
                    .lock()
                    .map_err(|_| "writer poisoned")? = Some("the region writer stopped".into());
                *self
                    .outstanding
                    .count
                    .lock()
                    .map_err(|_| "writer poisoned")? -= 1;
                self.outstanding.done.notify_all();
                return Err("the region writer stopped".into());
            }
        }
        if wait {
            self.flush()?;
        }
        Ok(())
    }
    fn start_writer(&self) -> Writer {
        let (send, receive) = mpsc::channel::<WriteBatch>();
        let root = self.root.clone();
        let (regions, entity_regions, poi_regions) = (
            self.regions.clone(),
            self.entity_regions.clone(),
            self.poi_regions.clone(),
        );
        let pending = self.pending.clone();
        let outstanding = self.outstanding.clone();
        let thread = std::thread::Builder::new()
            .name("horizon-region-writer".into())
            .spawn(move || {
                for (kind, batch) in receive {
                    let cache = match kind {
                        Folder::Region => &regions,
                        Folder::Entities => &entity_regions,
                        Folder::Poi => &poi_regions,
                    };
                    let folder = kind.name();
                    // IOWorker.pendingWrites: a chunk queued again since is
                    // written once, by its newest batch.
                    let newest = |k: &(i32, i32), seq: u64| {
                        pending.lock().map_or(true, |p| {
                            p.get(&(kind, k.0, k.1)).is_none_or(|p| p.0 <= seq)
                        })
                    };
                    let resolved: Vec<_> = batch
                        .iter()
                        .filter(|(k, _, seq)| newest(k, *seq))
                        .filter_map(|(k, r, _)| Some((*k, resolve_poi(&root, cache, *k, r)?)))
                        .collect();
                    let records: Vec<_> = resolved.iter().map(|(k, r)| (*k, r)).collect();
                    let result = write_region_records(&root, cache, folder, &records);
                    if let Err(e) = &result {
                        let message = format!("failed to write {folder} files: {e}");
                        log::warn!("Horizon: {message}");
                        *outstanding.error.lock().unwrap() = Some(message);
                        // `reportChunkSaveFailure` for each chunk of the batch.
                        for (k, _, _) in &batch {
                            crate::chunk_io::report_save_failure(&root, k.0, k.1, e);
                        }
                    }
                    // Keep failed snapshots readable; never report them as durable.
                    if result.is_ok()
                        && let Ok(mut pending) = pending.lock()
                    {
                        for (k, _, seq) in &batch {
                            if pending.get(&(kind, k.0, k.1)).is_some_and(|p| p.0 == *seq) {
                                pending.remove(&(kind, k.0, k.1));
                            }
                        }
                    }
                    if kind == Folder::Region {
                        outstanding.chunks.fetch_sub(batch.len(), Ordering::Relaxed);
                    }
                    if let Ok(mut count) = outstanding.count.lock() {
                        *count -= 1;
                        outstanding.done.notify_all();
                    }
                }
            })
            .expect("spawn region writer");
        Writer { send, thread }
    }
    /// Wait until every queued write is on disk.
    pub fn flush(&self) -> Result<()> {
        let mut count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        while *count > 0 {
            count = self
                .outstanding
                .done
                .wait(count)
                .map_err(|_| "writer poisoned")?;
        }
        self.write_result()
    }
    /// `ChunkMap.activeChunkWrites`: chunk records, excluding entity/POI IO.
    pub fn active_chunk_writes(&self) -> usize {
        self.outstanding.chunks.load(Ordering::Relaxed)
    }
    /// Poll durability without holding up the event loop. Errors remain visible
    /// to every waiter, including a later synchronous shutdown.
    pub fn writes_finished(&self) -> Result<bool> {
        let count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        self.write_result()?;
        Ok(*count == 0)
    }
    fn write_result(&self) -> Result<()> {
        match &*self
            .outstanding
            .error
            .lock()
            .map_err(|_| "writer poisoned")?
        {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }
}

impl Drop for SaveDir {
    /// Finish queued writes before the world is let go.
    fn drop(&mut self) {
        if let Some(writer) = self.writer.get_mut().ok().and_then(Option::take) {
            drop(writer.send);
            let _ = writer.thread.join();
        }
    }
}
