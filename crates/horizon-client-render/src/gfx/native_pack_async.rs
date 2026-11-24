//! Background worker for Horizon's own shader packs (`Renderer::load_native_shader_pack`).
//!
//! Compiles a native pack (manifest/WGSL check + wgpu shader modules and pipelines) on a single
//! dedicated background thread while the currently running pack keeps drawing on the main thread.
//! A newer load supersedes any in-flight or queued build via a monotonic generation counter and
//! per-job cancellation flag (latest-wins); stale builds drop their GPU resources on the worker
//! thread without touching the main thread.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use super::block_light::LightTable;
use super::native_material::TerrainShape;
use super::native_pack::{Metadata, NativePack};
use super::ray_lights::Sources;

/// Inputs to build a Horizon native shader pack (`NativePack::new`).
pub type NativePackData = (
    horizon_shader_api::Report,
    Vec<(String, f64)>,
    Vec<(u32, u32)>,
    Vec<(u32, u32)>,
    (LightTable, Sources),
);

/// Owned clones of the `wgpu` handles `TerrainShape` borrows, sent to the background worker.
pub(in crate::gfx) struct OwnedTerrainShape {
    pub globals: wgpu::BindGroupLayout,
    pub flat: Option<wgpu::BindGroupLayout>,
    pub sprites: [f64; 2],
    pub sprite_views: Vec<wgpu::TextureView>,
    pub format: wgpu::TextureFormat,
    pub linear: bool,
    pub msaa: bool,
    pub entities: wgpu::BindGroupLayout,
    pub retained_entities: wgpu::BindGroupLayout,
    pub weather_groups: [wgpu::BindGroupLayout; 2],
    pub pbr: Option<[wgpu::Texture; 2]>,
    pub looks_layout: Option<wgpu::BindGroupLayout>,
}

impl OwnedTerrainShape {
    pub fn as_shape<'a>(
        &'a self,
        sprite_tags: &'a [(u32, u32)],
        sprite_materials: &'a [(u32, u32)],
        looks: Option<&'a wgpu::BindGroupLayout>,
    ) -> TerrainShape<'a> {
        TerrainShape {
            globals: &self.globals,
            flat: self.flat.as_ref(),
            sprites: self.sprites,
            sprite_views: &self.sprite_views,
            format: self.format,
            linear: self.linear,
            msaa: self.msaa,
            sprite_tags,
            sprite_materials,
            entities: &self.entities,
            retained_entities: &self.retained_entities,
            weather_groups: [&self.weather_groups[0], &self.weather_groups[1]],
            pbr: self.pbr.as_ref(),
            looks,
        }
    }
}

/// Renderer state a background native pack build was compiled against: if any of these change
/// while compiling (frame format, linear-light encoding, or `sprite_views` mipmap range), the
/// finished build is discarded as [`NativePackOutcome::Stale`] so the caller rebuilds against the
/// current renderer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::gfx) struct BuildSnapshot {
    pub format: wgpu::TextureFormat,
    pub linear: bool,
    pub sprite_views_generation: u64,
}

/// A built `NativePack` ready to install on the main thread.
pub(in crate::gfx) struct BuiltNativePack {
    pub pack: NativePack,
    pub labels: Vec<String>,
    pub metadata: Metadata,
    pub looks_layout: Option<wgpu::BindGroupLayout>,
    pub snapshot: BuildSnapshot,
}

/// What the background worker produced for one generation.
pub(in crate::gfx) enum WorkerOutcome {
    Ready {
        built: Box<BuiltNativePack>,
        meta: Box<dyn Any + Send>,
    },
    Refused(Vec<String>),
    Cancelled,
}

/// Result returned to the caller by `Renderer::poll_native_shader_pack` or
/// `Renderer::wait_native_shader_pack`.
pub enum NativePackOutcome<C, M> {
    /// Built and installed into the renderer.
    Installed {
        context: C,
        labels: Vec<String>,
        meta: M,
    },
    /// Rejected during checking, pipeline creation, or executor install; the previous pack (if any)
    /// stays running unless `replace_executor` could not keep it.
    Refused { context: C, problems: Vec<String> },
    /// Superseded or cancelled before completion (not a pack failure).
    Cancelled,
    /// Completed, but renderer state (frame format, linear encoding, or sprite-view mipmaps)
    /// changed while compiling; the caller should schedule a fresh build against current state.
    Stale,
}

type JobFn = Box<dyn FnOnce(&AtomicBool) -> WorkerOutcome + Send + 'static>;

struct PendingJob {
    generation: u64,
    cancel: Arc<AtomicBool>,
    context: Box<dyn Any + Send>,
    run: JobFn,
}

pub(in crate::gfx) struct CompletedJob {
    pub context: Box<dyn Any + Send>,
    pub outcome: Result<WorkerOutcome, String>,
}

struct StoredCompleted {
    generation: u64,
    job: CompletedJob,
}

#[derive(Default)]
struct WorkerState {
    pending: Option<PendingJob>,
    running_cancel: Option<Arc<AtomicBool>>,
    completed: Option<StoredCompleted>,
    busy: bool,
    shutdown: bool,
}

struct Shared {
    generation: AtomicU64,
    state: Mutex<WorkerState>,
    wake: Condvar,
}

/// Single-worker latest-wins background loader for native shader packs.
///
/// The worker thread is spawned lazily on the first `submit` call, so sessions that never load a
/// native shader pack pay zero thread overhead.
#[derive(Default)]
pub(in crate::gfx) struct NativePackLoader {
    shared: Option<Arc<Shared>>,
    in_flight: bool,
}

impl NativePackLoader {
    fn ensure_worker(&mut self) -> Result<&Arc<Shared>, std::io::Error> {
        if self.shared.is_none() {
            let shared = Arc::new(Shared {
                generation: AtomicU64::new(0),
                state: Mutex::new(WorkerState::default()),
                wake: Condvar::new(),
            });
            let worker_shared = shared.clone();
            std::thread::Builder::new()
                .name("Native shader pack".into())
                .spawn(move || worker_loop(worker_shared))?;
            self.shared = Some(shared);
        }
        Ok(self.shared.as_ref().expect("worker initialized"))
    }

    /// Submits a new native pack build. Any previously queued or in-flight build is cancelled and
    /// its result discarded (latest-wins). Returns `Err` only if the worker thread could not be
    /// spawned.
    pub fn submit<C, F>(&mut self, context: C, run: F) -> Result<(), (C, std::io::Error)>
    where
        C: Send + 'static,
        F: FnOnce(&AtomicBool) -> WorkerOutcome + Send + 'static,
    {
        let shared = match self.ensure_worker() {
            Ok(shared) => shared.clone(),
            Err(e) => return Err((context, e)),
        };
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let generation = shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(cancel) = state.running_cancel.as_ref() {
            cancel.store(true, Ordering::Release);
        }
        state.completed = None;
        state.pending = Some(PendingJob {
            generation,
            cancel: Arc::new(AtomicBool::new(false)),
            context: Box::new(context),
            run: Box::new(run),
        });
        self.in_flight = true;
        shared.wake.notify_all();
        Ok(())
    }

    /// Cancels any queued or in-flight build and discards any unpolled result.
    pub fn cancel(&mut self) {
        self.in_flight = false;
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        shared.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(cancel) = state.running_cancel.as_ref() {
            cancel.store(true, Ordering::Release);
        }
        state.pending = None;
        state.completed = None;
        shared.wake.notify_all();
    }

    /// Whether a build for the latest generation is still in flight or waiting to be polled.
    pub fn is_loading(&self) -> bool {
        self.in_flight
    }

    /// Non-blocking poll for the latest generation's completed build.
    pub fn poll(&mut self) -> Option<CompletedJob> {
        if !self.in_flight {
            return None;
        }
        let shared = self.shared.as_ref()?;
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let target = shared.generation.load(Ordering::Acquire);
        if state
            .completed
            .as_ref()
            .is_some_and(|done| done.generation == target)
        {
            self.in_flight = false;
            return state.completed.take().map(|done| done.job);
        }
        None
    }

    /// Blocks until the latest generation's build finishes and returns it, or `None` if nothing is
    /// in flight. Used by deterministic capture paths (`HORIZON_STILL`, `screenshot-sequence`,
    /// `horizon-ctl screenshot`).
    pub fn wait(&mut self) -> Option<CompletedJob> {
        if !self.in_flight {
            return None;
        }
        let shared = self.shared.as_ref()?;
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let target = shared.generation.load(Ordering::Acquire);
        loop {
            if state
                .completed
                .as_ref()
                .is_some_and(|done| done.generation == target)
            {
                self.in_flight = false;
                return state.completed.take().map(|done| done.job);
            }
            if state.shutdown
                || (state.pending.is_none() && !state.busy && state.completed.is_none())
            {
                self.in_flight = false;
                return None;
            }
            state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for NativePackLoader {
    fn drop(&mut self) {
        self.in_flight = false;
        let Some(shared) = self.shared.take() else {
            return;
        };
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        shared.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(cancel) = state.running_cancel.as_ref() {
            cancel.store(true, Ordering::Release);
        }
        state.pending = None;
        state.completed = None;
        state.shutdown = true;
        shared.wake.notify_all();
    }
}

fn worker_loop(shared: Arc<Shared>) {
    loop {
        let job = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(job) = state.pending.take() {
                    if job.generation != shared.generation.load(Ordering::Acquire) {
                        // Superseded before it even started; drop it outside the lock below?
                        // PendingJob only holds closures/context, not built GPU packs, so dropping
                        // here is immediate.
                        continue;
                    }
                    state.running_cancel = Some(job.cancel.clone());
                    state.busy = true;
                    break job;
                }
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };

        let PendingJob {
            generation,
            cancel,
            context,
            run,
        } = job;
        let outcome = crate::crash::contain(std::panic::AssertUnwindSafe(|| run(&cancel)));

        // Store the result only if this generation is still current and not cancelled. Any stale
        // or cancelled result is dropped after releasing the lock so GPU handles drop on the
        // worker thread without blocking the main thread.
        let discarded = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.running_cancel = None;
            state.busy = false;
            let current = generation == shared.generation.load(Ordering::Acquire)
                && !cancel.load(Ordering::Acquire)
                && !state.shutdown;
            let dropped = if current {
                state.completed = Some(StoredCompleted {
                    generation,
                    job: CompletedJob { context, outcome },
                });
                None
            } else {
                Some((context, outcome))
            };
            shared.wake.notify_all();
            dropped
        };
        drop(discarded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn latest_wins_cancels_running_job_and_coalesces_queued_jobs() {
        let mut loader = NativePackLoader::default();
        let started = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let ran_second = Arc::new(AtomicBool::new(false));
        let first_saw_cancel = Arc::new(AtomicBool::new(false));

        let s1 = started.clone();
        let r1 = release.clone();
        let c1 = first_saw_cancel.clone();
        loader
            .submit(1u32, move |cancel| {
                s1.wait();
                r1.wait();
                c1.store(cancel.load(Ordering::Acquire), Ordering::Release);
                WorkerOutcome::Cancelled
            })
            .unwrap();
        // Wait until job 1 is running on the worker thread.
        started.wait();

        // Queue job 2, then supersede it with job 3 before job 1 finishes.
        let r2 = ran_second.clone();
        loader
            .submit(2u32, move |_| {
                r2.store(true, Ordering::Release);
                WorkerOutcome::Refused(vec!["stale".into()])
            })
            .unwrap();
        loader
            .submit(3u32, |cancel| {
                assert!(!cancel.load(Ordering::Acquire));
                WorkerOutcome::Refused(vec!["latest".into()])
            })
            .unwrap();
        assert!(loader.is_loading());

        // Let job 1 observe cancellation and exit.
        release.wait();
        let done = loader.wait().expect("job 3 completes");
        assert!(!loader.is_loading());
        assert!(first_saw_cancel.load(Ordering::Acquire));
        assert!(
            !ran_second.load(Ordering::Acquire),
            "coalesced job 2 must never execute"
        );
        assert_eq!(*done.context.downcast::<u32>().unwrap(), 3);
        match done.outcome {
            Ok(WorkerOutcome::Refused(problems)) => assert_eq!(problems, vec!["latest"]),
            _ => panic!("expected latest refused outcome"),
        }
    }

    #[test]
    fn cancel_discards_in_flight_job_and_wait_returns_none() {
        let mut loader = NativePackLoader::default();
        let started = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let saw_cancel = Arc::new(AtomicBool::new(false));

        let s = started.clone();
        let r = release.clone();
        let c = saw_cancel.clone();
        loader
            .submit("first", move |cancel| {
                s.wait();
                r.wait();
                c.store(cancel.load(Ordering::Acquire), Ordering::Release);
                WorkerOutcome::Refused(vec!["should be discarded".into()])
            })
            .unwrap();
        started.wait();
        loader.cancel();
        assert!(!loader.is_loading());
        release.wait();
        assert!(loader.poll().is_none());
        assert!(loader.wait().is_none());
        // Submit another job after cancel to prove the worker stayed healthy.
        loader
            .submit("second", |_| WorkerOutcome::Refused(vec!["ok".into()]))
            .unwrap();
        let done = loader.wait().expect("second completes");
        assert!(saw_cancel.load(Ordering::Acquire));
        assert_eq!(*done.context.downcast::<&str>().unwrap(), "second");
    }

    #[test]
    fn panicking_job_is_contained_and_worker_stays_usable() {
        let mut loader = NativePackLoader::default();
        loader
            .submit(42u32, |_| -> WorkerOutcome { panic!("boom") })
            .unwrap();
        let done = loader.wait().expect("panic contained");
        assert_eq!(*done.context.downcast::<u32>().unwrap(), 42);
        let msg = done.outcome.err().expect("panic error");
        assert!(msg.contains("boom"), "{msg}");

        loader
            .submit(43u32, |_| WorkerOutcome::Refused(vec!["recovered".into()]))
            .unwrap();
        let done = loader.wait().expect("worker still runs");
        assert_eq!(*done.context.downcast::<u32>().unwrap(), 43);
        assert!(matches!(done.outcome, Ok(WorkerOutcome::Refused(_))));
    }

    #[test]
    fn submit_discards_unpolled_completed_result_from_earlier_generation() {
        let mut loader = NativePackLoader::default();
        let finished_first = Arc::new(Barrier::new(2));
        let f1 = finished_first.clone();
        loader
            .submit("A", move |_| {
                f1.wait();
                WorkerOutcome::Refused(vec!["result A".into()])
            })
            .unwrap();
        // Let A complete on the worker thread without polling it on the main thread.
        finished_first.wait();
        // Wait until A is definitely stored in `completed`, then supersede it with B before poll.
        let shared = loader.shared.as_ref().unwrap().clone();
        {
            let mut state = shared.state.lock().unwrap();
            while state.completed.is_none() {
                state = shared.wake.wait(state).unwrap();
            }
        }
        loader
            .submit("B", |_| WorkerOutcome::Refused(vec!["result B".into()]))
            .unwrap();
        let done = loader.wait().expect("B completes");
        assert_eq!(*done.context.downcast::<&str>().unwrap(), "B");
        match done.outcome {
            Ok(WorkerOutcome::Refused(problems)) => assert_eq!(problems, vec!["result B"]),
            _ => panic!("expected B outcome"),
        }
    }

    #[test]
    fn build_snapshot_detects_mipmap_and_frame_changes() {
        let base = BuildSnapshot {
            format: wgpu::TextureFormat::Rgba16Float,
            linear: true,
            sprite_views_generation: 0,
        };
        assert_eq!(base, base);
        assert_ne!(
            base,
            BuildSnapshot {
                sprite_views_generation: 1,
                ..base
            }
        );
        assert_ne!(
            base,
            BuildSnapshot {
                linear: false,
                ..base
            }
        );
        assert_ne!(
            base,
            BuildSnapshot {
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                ..base
            }
        );
    }
}
