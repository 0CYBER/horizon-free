//! `SectionOcclusionGraph`: which sections the camera can see through open
//! faces. Breadth-first from the camera section through the faces each
//! section's VisGraph connects, never stepping back towards the camera.
//!
//! As in vanilla, the full search runs off the render thread: a worker keeps
//! its own copy of every section's connectivity and searches over dense
//! arrays around the camera. Frames use the last finished graph and never wait
//! for the next one. A section meshed in between propagates into the graph the
//! frame has (vanilla's partial update), so what it opens shows at once; what
//! it closes goes at the next full search.
use super::SectionKey;
use crate::mesher::ALL_VISIBLE;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Vanilla's `Direction` order: down, up, north, south, west, east.
const STEPS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
/// In `Graph::directions`: the cell has a node.
const REACHED: u8 = 1 << 7;
/// In `Graph::entered`: the camera's own section, entered through no face.
const NONE: u8 = 6;

/// VisGraph connectivity per column, one entry per section (bottom first).
/// A missing column or section connects every face.
struct Columns {
    sections: i32,
    map: HashMap<(i32, i32), Box<[u64]>>,
}

impl Columns {
    fn new(sections: i32) -> Self {
        Self {
            sections,
            map: HashMap::new(),
        }
    }
    /// Whether the section's connectivity changed.
    fn set(&mut self, key: SectionKey, vis: u64) -> bool {
        if !(0..self.sections).contains(&key.1) {
            return false;
        }
        let column = match self.map.get_mut(&(key.0, key.2)) {
            Some(column) => column,
            None if vis == ALL_VISIBLE => return false,
            None => self
                .map
                .entry((key.0, key.2))
                .or_insert_with(|| vec![ALL_VISIBLE; self.sections as usize].into()),
        };
        let slot = &mut column[key.1 as usize];
        if *slot == vis {
            return false;
        }
        *slot = vis;
        if vis == ALL_VISIBLE && column.iter().all(|&v| v == ALL_VISIBLE) {
            self.map.remove(&(key.0, key.2));
        }
        true
    }
    fn get(&self, key: SectionKey) -> u64 {
        self.map
            .get(&(key.0, key.2))
            .and_then(|column| column.get(usize::try_from(key.1).ok()?))
            .copied()
            .unwrap_or(ALL_VISIBLE)
    }
    fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Set(key, vis) => {
                self.set(key, vis);
            }
            Edit::Clear => self.map.clear(),
        }
    }
}

#[derive(Clone, Copy)]
enum Edit {
    Set(SectionKey, u64),
    Clear,
}

/// One search's result over the box around the camera: vanilla's
/// `GraphStorage.sectionToNodeMap`, as dense arrays.
#[derive(Default)]
struct Graph {
    from: SectionKey,
    radius: i32,
    /// The searched box: lowest section y and its height in sections.
    y0: i32,
    height: i32,
    /// Per cell: `REACHED`, and the directions travelled from the camera to
    /// it (`Node.directions`).
    directions: Vec<u8>,
    /// Per cell: the face it was first entered through (`NONE` for the
    /// camera's). Vanilla's `Node.sourceDirections` also keeps the faces it is
    /// reached through later; Horizon keeps the first only.
    entered: Vec<u8>,
}

impl Graph {
    fn index(&self, key: SectionKey) -> Option<usize> {
        let (dx, dz, dy) = (key.0 - self.from.0, key.2 - self.from.2, key.1 - self.y0);
        let side = 2 * self.radius + 1;
        if dx.abs() > self.radius || dz.abs() > self.radius || !(0..self.height).contains(&dy) {
            return None;
        }
        Some((((dz + self.radius) * side + dx + self.radius) * self.height + dy) as usize)
    }
    fn key(&self, index: usize) -> SectionKey {
        let side = 2 * self.radius + 1;
        let i = index as i32;
        let column = i / self.height;
        (
            self.from.0 + column % side - self.radius,
            self.y0 + i % self.height,
            self.from.2 + column / side - self.radius,
        )
    }
    fn contains(&self, key: SectionKey) -> bool {
        self.index(key)
            .is_some_and(|i| self.directions[i] & REACHED != 0)
    }
    /// The full search from `from` (vanilla's `scheduleFullUpdate`).
    fn search(
        &mut self,
        columns: &Columns,
        from: SectionKey,
        radius: i32,
        vis: &mut Vec<u64>,
        queue: &mut VecDeque<u32>,
    ) {
        self.from = from;
        self.radius = radius;
        self.y0 = from.1.min(-1);
        self.height = from.1.max(columns.sections) - self.y0 + 1;
        let side = (2 * radius + 1) as usize;
        let size = side * side * self.height as usize;
        self.directions.clear();
        self.directions.resize(size, 0);
        self.entered.clear();
        self.entered.resize(size, NONE);
        vis.clear();
        vis.resize(size, ALL_VISIBLE);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let Some(column) = columns.map.get(&(from.0 + dx, from.2 + dz)) else {
                    continue;
                };
                let base = self
                    .index((from.0 + dx, 0, from.2 + dz))
                    .expect("column inside the searched box");
                vis[base..base + column.len()].copy_from_slice(column);
            }
        }
        let start = self.index(from).expect("camera inside the searched box");
        self.directions[start] = REACHED;
        queue.clear();
        queue.push_back(start as u32);
        self.propagate(queue, columns.sections, |i, _| vis[i]);
    }
    /// Vanilla's `runUpdates`: take nodes from the queue and step through
    /// every face their entry face connects to, away from the camera.
    fn propagate(
        &mut self,
        queue: &mut VecDeque<u32>,
        sections: i32,
        vis: impl Fn(usize, SectionKey) -> u64,
    ) {
        while let Some(index) = queue.pop_front() {
            let i = index as usize;
            let key = self.key(i);
            let directions = self.directions[i];
            let entered = self.entered[i];
            let exits = if entered == NONE {
                0x3f
            } else {
                (vis(i, key) >> (u32::from(entered) * 6)) as u8 & 0x3f
            };
            for (d, step) in STEPS.iter().enumerate() {
                if exits & (1 << d) == 0 || directions & (1 << (d ^ 1)) != 0 {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > sections {
                    continue;
                }
                let Some(n) = self.index(next) else { continue };
                if self.directions[n] & REACHED == 0 {
                    self.directions[n] = directions | 1 << d;
                    self.entered[n] = (d ^ 1) as u8;
                    queue.push_back(n as u32);
                }
            }
        }
    }
}

struct Job {
    edits: Vec<Edit>,
    from: SectionKey,
    radius: i32,
    /// Buffers to reuse.
    graph: Graph,
}

/// The thread that runs full searches over its own copy of the columns.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(Graph, Vec<Edit>)>,
}

impl Worker {
    fn spawn(sections: i32) -> Option<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("occlusion".into())
            .spawn(move || {
                let mut columns = Columns::new(sections);
                let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
                while let Ok(mut job) = job_rx.recv() {
                    for edit in job.edits.drain(..) {
                        columns.apply(edit);
                    }
                    job.graph
                        .search(&columns, job.from, job.radius, &mut vis, &mut queue);
                    if done_tx.send((job.graph, job.edits)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { jobs, done })
    }
}

pub struct Occlusion {
    /// The render thread's copy, for partial updates (and searches when the
    /// worker is gone).
    columns: Columns,
    /// Edits the worker has not been sent yet.
    edits: Vec<Edit>,
    /// A full search is due: sections changed since the last one started.
    dirty: bool,
    /// The last finished search; `None` until one finishes, or after the
    /// columns were cleared.
    graph: Option<Graph>,
    spare: Graph,
    worker: Option<Worker>,
    /// The camera section and radius of the running search.
    running: Option<(SectionKey, i32)>,
    /// The running search started before the columns were cleared: drop it.
    discard: bool,
    /// The camera section and radius `graph`'s search started from (or is
    /// starting from).
    searched: Option<(SectionKey, i32)>,
    /// Sections changed since `graph`'s search started, to propagate from.
    changed: Vec<SectionKey>,
    /// Sections changed since the running search started: they propagate into
    /// its graph when it lands.
    changed_late: Vec<SectionKey>,
    vis: Vec<u64>,
    queue: VecDeque<u32>,
}

impl Occlusion {
    pub fn new(sections: i32) -> Self {
        Self {
            columns: Columns::new(sections),
            edits: Vec::new(),
            dirty: true,
            graph: None,
            spare: Graph::default(),
            worker: Worker::spawn(sections),
            running: None,
            discard: false,
            searched: None,
            changed: Vec::new(),
            changed_late: Vec::new(),
            vis: Vec::new(),
            queue: VecDeque::new(),
        }
    }
    pub fn set(&mut self, key: SectionKey, vis: u64) {
        if self.columns.set(key, vis) {
            self.edits.push(Edit::Set(key, vis));
            self.dirty = true;
            self.changed.push(key);
            if self.running.is_some() {
                self.changed_late.push(key);
            }
        }
    }
    pub fn remove(&mut self, key: SectionKey) {
        self.set(key, ALL_VISIBLE);
    }
    pub fn clear(&mut self) {
        self.columns.map.clear();
        self.edits.clear();
        self.edits.push(Edit::Clear);
        self.dirty = true;
        // Another world: the old graph says nothing about it.
        if let Some(graph) = self.graph.take() {
            self.spare = graph;
        }
        self.searched = None;
        self.discard = self.running.is_some();
        self.changed.clear();
        self.changed_late.clear();
    }
    /// Whether `key` is visible from the camera. Without a usable graph
    /// (before the first search lands, or with the camera somewhere the graph
    /// never reached) every section is.
    pub fn contains(&self, key: SectionKey) -> bool {
        self.graph.as_ref().is_none_or(|g| g.contains(key))
    }
    /// Take a finished search, propagate changed sections into the graph,
    /// and start a search when sections changed or the camera moved to
    /// another section. Never waits for the worker.
    pub fn update(&mut self, from: SectionKey, radius: i32) {
        self.collect(false);
        self.propagate_changes();
        if self.running.is_none() && (self.dirty || self.searched != Some((from, radius))) {
            self.start(from, radius);
        }
        // A graph that never reached the camera's section hides everything
        // around it: until the new one lands, cull by the frustum alone.
        if self
            .graph
            .as_ref()
            .is_some_and(|g| g.from != from && !g.contains(from))
            && let Some(graph) = self.graph.take()
        {
            self.spare = graph;
        }
    }
    fn start(&mut self, from: SectionKey, radius: i32) {
        self.dirty = false;
        self.searched = Some((from, radius));
        let graph = std::mem::take(&mut self.spare);
        if let Some(worker) = &self.worker {
            let job = Job {
                edits: std::mem::take(&mut self.edits),
                from,
                radius,
                graph,
            };
            if worker.jobs.send(job).is_ok() {
                self.running = Some((from, radius));
                self.changed_late.clear();
                return;
            }
            log::warn!("Horizon: the occlusion worker stopped; culling on the render thread");
            self.worker = None;
        }
        self.edits.clear();
        let mut graph = Graph::default();
        graph.search(&self.columns, from, radius, &mut self.vis, &mut self.queue);
        self.land(graph);
    }
    fn collect(&mut self, wait: bool) {
        let Some(worker) = &self.worker else { return };
        if self.running.is_none() {
            return;
        }
        let result = if wait {
            worker.done.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            worker.done.try_recv()
        };
        match result {
            Ok((graph, edits)) => {
                self.running = None;
                if self.edits.is_empty() {
                    self.edits = edits;
                }
                if std::mem::take(&mut self.discard) {
                    self.spare = graph;
                    return;
                }
                self.land(graph);
                self.changed.clear();
                self.changed.append(&mut self.changed_late);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The edits it held are lost with it: search here from now on.
                self.worker = None;
                self.running = None;
                self.discard = false;
                self.dirty = true;
            }
        }
    }
    fn land(&mut self, graph: Graph) {
        if let Some(old) = self.graph.replace(graph) {
            self.spare = old;
        }
    }
    /// Vanilla's `runPartialUpdate`: what changed sections now connect to.
    fn propagate_changes(&mut self) {
        let Some(graph) = &mut self.graph else {
            self.changed.clear();
            return;
        };
        self.queue.clear();
        for key in self.changed.drain(..) {
            if let Some(i) = graph.index(key)
                && graph.directions[i] & REACHED != 0
            {
                self.queue.push_back(i as u32);
            }
        }
        let columns = &self.columns;
        graph.propagate(&mut self.queue, columns.sections, |_, key| columns.get(key));
    }
    /// Wait for the running search: tests, and A/B timing of the old
    /// blocking search.
    #[cfg(test)]
    fn settle(&mut self, from: SectionKey, radius: i32) {
        self.update(from, radius);
        self.collect(true);
        self.propagate_changes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_space_reaches_the_whole_box() {
        let mut o = Occlusion::new(4);
        o.settle((0, 1, 0), 2);
        for x in -2..=2 {
            for z in -2..=2 {
                for y in -1..=4 {
                    assert!(o.contains((x, y, z)), "{x} {y} {z}");
                }
            }
        }
        assert!(!o.contains((3, 1, 0)));
    }

    #[test]
    fn closed_neighbours_hide_what_is_behind_them() {
        let mut o = Occlusion::new(4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle((0, 1, 0), 3);
        assert!(o.contains((1, 1, 0)), "a neighbour is seen");
        assert!(!o.contains((2, 1, 0)), "nothing shows through it");
        o.set((1, 1, 0), ALL_VISIBLE);
        o.settle((0, 1, 0), 3);
        assert!(o.contains((2, 1, 0)));
    }

    #[test]
    fn frames_never_wait_and_partial_updates_open_at_once() {
        let mut o = Occlusion::new(4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle((0, 1, 0), 3);
        assert!(!o.contains((2, 1, 0)));
        // Opening the wall shows what is behind it in the same frame, from the
        // graph the frame has, whether or not the full search has finished.
        o.set((1, 1, 0), ALL_VISIBLE);
        o.update((0, 1, 0), 3);
        assert!(o.contains((2, 1, 0)));
        assert!(o.contains((3, 1, 0)));
        // Closing it again lasts until the next full search lands.
        o.set((1, 1, 0), 0);
        o.settle((0, 1, 0), 3);
        o.settle((0, 1, 0), 3);
        assert!(!o.contains((2, 1, 0)));
    }

    #[test]
    fn a_graph_that_lost_the_camera_culls_nothing() {
        let mut o = Occlusion::new(4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle((0, 1, 0), 3);
        assert!(!o.contains((3, 1, 0)));
        // The camera jumped past the wall; the next search is still running.
        let o = &mut o;
        o.update((3, 1, 0), 3);
        if o.running.is_some() {
            assert!(o.contains((3, 1, 0)));
        }
        o.settle((3, 1, 0), 3);
        assert!(o.contains((3, 1, 0)));
        assert!(
            !o.contains((0, 1, 0)),
            "the old camera section is walled in"
        );
    }

    /// The search before it moved to the worker (main at ce38c8d7), over the
    /// same box: whether each cell is reached.
    fn reference(columns: &Columns, from: SectionKey, radius: i32) -> Vec<bool> {
        let y0 = from.1.min(-1);
        let height = from.1.max(columns.sections) - y0 + 1;
        let side = 2 * radius + 1;
        let index = |k: SectionKey| {
            let (dx, dz, dy) = (k.0 - from.0, k.2 - from.2, k.1 - y0);
            (dx.abs() <= radius && dz.abs() <= radius && (0..height).contains(&dy))
                .then(|| (((dz + radius) * side + dx + radius) * height + dy) as usize)
        };
        let size = (side * side * height) as usize;
        let mut reached = vec![false; size];
        let start = index(from).unwrap();
        reached[start] = true;
        let mut queue = VecDeque::from([(from, NONE, 0u8)]);
        while let Some((key, entered, travelled)) = queue.pop_front() {
            let vis = columns.get(key);
            for (d, step) in STEPS.iter().enumerate() {
                let opposite = d ^ 1;
                if travelled & (1 << opposite) != 0 {
                    continue;
                }
                if entered != NONE && vis & (1 << (usize::from(entered) * 6 + d)) == 0 {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > columns.sections {
                    continue;
                }
                let Some(n) = index(next) else { continue };
                if !reached[n] {
                    reached[n] = true;
                    queue.push_back((next, opposite as u8, travelled | (1 << d)));
                }
            }
        }
        reached
    }

    fn random_world(o: &mut Occlusion, seed: u64, count: usize) {
        let mut seed = seed;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..count {
            let r = next();
            let key = (
                (r % 13) as i32 - 6,
                ((r >> 8) % 6) as i32,
                ((r >> 16) % 13) as i32 - 6,
            );
            // Open, half open or nearly closed sections.
            let vis = match r >> 40 & 3 {
                0 => next() & next() & next(),
                1 => next(),
                _ => next() | next(),
            };
            o.set(key, vis & ALL_VISIBLE);
        }
    }

    /// The graph is the search the render thread did before, cell for cell.
    #[test]
    fn same_sections_as_before() {
        let mut hidden = 0;
        for seed in [0x9e37_79b9, 7, 0xdead_beef] {
            let mut o = Occlusion::new(6);
            random_world(&mut o, seed, 900);
            for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4), (1, -1, 1), (0, 7, 0)] {
                o.settle(from, 6);
                o.settle(from, 6);
                let graph = o.graph.as_ref().expect("searched");
                let reached: Vec<bool> =
                    graph.directions.iter().map(|&d| d & REACHED != 0).collect();
                assert_eq!(reached, reference(&o.columns, from, 6), "{seed} {from:?}");
                hidden += reached.iter().filter(|&&r| !r).count();
            }
        }
        assert!(hidden > 1000, "the worlds hide sections ({hidden})");
    }

    /// The worker's graph is the one the render thread would compute.
    #[test]
    fn the_worker_matches_a_search_on_the_render_thread() {
        let mut o = Occlusion::new(6);
        random_world(&mut o, 0x9e37_79b9, 800);
        for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4)] {
            o.settle(from, 6);
            o.settle(from, 6);
            let mut local = Graph::default();
            local.search(&o.columns, from, 6, &mut Vec::new(), &mut VecDeque::new());
            let graph = o.graph.as_ref().expect("searched");
            assert_eq!(graph.directions, local.directions);
            assert_eq!(graph.entered, local.entered);
        }
    }
}
