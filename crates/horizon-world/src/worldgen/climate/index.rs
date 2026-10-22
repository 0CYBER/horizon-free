//! Immutable climate search tree with an explicit per-worker previous result.
use super::{TargetPoint, quantize};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub min: i64,
    pub max: i64,
}
impl Parameter {
    /// The record constructor does not validate endpoint order.
    pub const fn new(min: i64, max: i64) -> Self {
        Self { min, max }
    }
    pub fn point(value: f32) -> Self {
        let value = quantize(value);
        Self::new(value, value)
    }
    pub fn span(min: f32, max: f32) -> Result<Self, IndexError> {
        if min > max {
            return Err(IndexError::InvertedSpan);
        }
        Ok(Self::new(quantize(min), quantize(max)))
    }
    pub fn distance(self, value: i64) -> i64 {
        let above = value.wrapping_sub(self.max);
        if above > 0 {
            above
        } else {
            self.min.wrapping_sub(value).max(0)
        }
    }
    fn union(self, other: Self) -> Self {
        Self::new(self.min.min(other.min), self.max.max(other.max))
    }
    fn center(self) -> i64 {
        self.min.wrapping_add(self.max) / 2
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParameterPoint {
    /// Temperature, humidity, continentalness, erosion, depth, weirdness.
    pub coordinates: [Parameter; 6],
    pub offset: i64,
}
impl ParameterPoint {
    pub fn fitness(self, target: TargetPoint) -> i64 {
        self.coordinates.iter().zip(target.parameters()).fold(
            self.offset.wrapping_mul(self.offset),
            |sum, (parameter, value)| {
                let distance = parameter.distance(value);
                sum.wrapping_add(distance.wrapping_mul(distance))
            },
        )
    }
    pub fn parameter_space(self) -> [Parameter; 7] {
        std::array::from_fn(|i| {
            if i < 6 {
                self.coordinates[i]
            } else {
                Parameter::new(self.offset, self.offset)
            }
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexError {
    Empty,
    TooManyEntries,
    InvalidBranching,
    InvertedSpan,
    NoSplit,
    NoResult,
}
impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "climate search requires at least one entry",
            Self::TooManyEntries => "climate search is limited to 65536 entries",
            Self::InvalidBranching => "climate branching must be in 2..=32",
            Self::InvertedSpan => "climate interval endpoints are inverted",
            Self::NoSplit => "climate tree could not select a split",
            Self::NoResult => "climate search found no result",
        })
    }
}
impl std::error::Error for IndexError {}

#[derive(Debug)]
enum Children {
    Leaf(usize),
    Branch(Vec<usize>),
}
#[derive(Debug)]
struct Node {
    space: [Parameter; 7],
    children: Children,
}
fn distance(space: &[Parameter; 7], values: &[i64; 7]) -> i64 {
    space
        .iter()
        .zip(values)
        .fold(0i64, |sum, (parameter, value)| {
            let distance = parameter.distance(*value);
            sum.wrapping_add(distance.wrapping_mul(distance))
        })
}
fn compare_space(a: &[Parameter; 7], b: &[Parameter; 7], axis: usize, absolute: bool) -> Ordering {
    for i in 0..7 {
        let coordinate = (axis + i) % 7;
        let key = |p: Parameter| {
            if absolute {
                p.center().wrapping_abs()
            } else {
                p.center()
            }
        };
        let result = key(a[coordinate]).cmp(&key(b[coordinate]));
        if result != Ordering::Equal {
            return result;
        }
    }
    Ordering::Equal
}
#[derive(Debug)]
struct Bucket {
    space: [Parameter; 7],
    children: Vec<usize>,
}
fn space(nodes: &[Node], children: &[usize]) -> [Parameter; 7] {
    let mut space = nodes[children[0]].space;
    for child in &children[1..] {
        for (parameter, other) in space.iter_mut().zip(nodes[*child].space) {
            *parameter = parameter.union(other);
        }
    }
    space
}
fn build(
    nodes: &mut Vec<Node>,
    mut children: Vec<usize>,
    branching: usize,
) -> Result<usize, IndexError> {
    if children.len() == 1 {
        return Ok(children[0]);
    }
    if children.len() <= branching {
        // Stable sorting matters when centers and total center distance tie.
        children.sort_by_key(|id| {
            nodes[*id]
                .space
                .iter()
                .fold(0i64, |sum, p| sum.wrapping_add(p.center().wrapping_abs()))
        });
    } else {
        // For this bounded domain this is the same bucket size as vanilla's
        // pow(branching,floor(log(length-0.01)/log(branching))). Integer powers
        // avoid depending on platform double logarithms during construction.
        let mut bucket_size = 1;
        while bucket_size * branching < children.len() {
            bucket_size *= branching;
        }
        let mut best_cost = i64::MAX;
        let mut best = None;
        for axis in 0..7 {
            children.sort_by(|a, b| compare_space(&nodes[*a].space, &nodes[*b].space, axis, false));
            let buckets: Vec<_> = children
                .chunks(bucket_size)
                .map(|children| Bucket {
                    space: space(nodes, children),
                    children: children.to_vec(),
                })
                .collect();
            let cost = buckets.iter().fold(0i64, |sum, bucket| {
                bucket.space.iter().fold(sum, |sum, p| {
                    sum.wrapping_add(p.max.wrapping_sub(p.min).wrapping_abs())
                })
            });
            if best_cost > cost {
                best_cost = cost;
                best = Some((axis, buckets));
            }
        }
        let (axis, mut buckets) = best.ok_or(IndexError::NoSplit)?;
        buckets.sort_by(|a, b| compare_space(&a.space, &b.space, axis, true));
        children = buckets
            .into_iter()
            .map(|bucket| build(nodes, bucket.children, branching))
            .collect::<Result<_, _>>()?;
    }
    let space = space(nodes, &children);
    let id = nodes.len();
    nodes.push(Node {
        space,
        children: Children::Branch(children),
    });
    Ok(id)
}

/// The index is immutable and can be shared; each searcher owns its cache.
/// Entry count and branching are explicit limits of this bounded Rust API.
#[derive(Debug)]
pub struct ClimateIndex<T> {
    entries: Vec<(ParameterPoint, T)>,
    /// The built tree renumbered breadth first, root at 0, so each branch's
    /// children are one contiguous run and a search walks memory in order.
    spaces: Vec<[Parameter; 7]>,
    /// Per node: its first child and child count, or its entry and 0 for a leaf.
    links: Vec<(u32, u32)>,
}
impl<T> ClimateIndex<T> {
    pub fn new(entries: Vec<(ParameterPoint, T)>) -> Result<Self, IndexError> {
        Self::with_branching(entries, 19)
    }
    pub fn with_branching(
        entries: Vec<(ParameterPoint, T)>,
        branching: usize,
    ) -> Result<Self, IndexError> {
        if entries.is_empty() {
            return Err(IndexError::Empty);
        }
        if entries.len() > 65536 {
            return Err(IndexError::TooManyEntries);
        }
        if !(2..=32).contains(&branching) {
            return Err(IndexError::InvalidBranching);
        }
        let mut nodes: Vec<_> = entries
            .iter()
            .enumerate()
            .map(|(id, (point, _))| Node {
                space: point.parameter_space(),
                children: Children::Leaf(id),
            })
            .collect();
        let children = (0..nodes.len()).collect();
        let root = build(&mut nodes, children, branching)?;
        let mut order = vec![root];
        let mut links = Vec::with_capacity(nodes.len());
        let mut next = 0;
        while next < order.len() {
            let node = &nodes[order[next]];
            next += 1;
            links.push(match &node.children {
                Children::Leaf(entry) => (*entry as u32, 0),
                Children::Branch(children) => {
                    let first = order.len() as u32;
                    order.extend_from_slice(children);
                    (first, children.len() as u32)
                }
            });
        }
        Ok(Self {
            entries,
            spaces: order.iter().map(|id| nodes[*id].space).collect(),
            links,
        })
    }
    pub fn entries(&self) -> &[(ParameterPoint, T)] {
        &self.entries
    }
    pub fn searcher(&self) -> ClimateSearch<'_, T> {
        ClimateSearch {
            index: self,
            last: None,
        }
    }
    /// Vanilla's separate reference path: ties retain input-list order and do
    /// not consult or update the searcher's previous indexed result.
    pub fn brute_force(&self, target: TargetPoint) -> &T {
        let mut best = 0;
        let mut fitness = self.entries[0].0.fitness(target);
        for (id, (point, _)) in self.entries.iter().enumerate().skip(1) {
            let next = point.fitness(target);
            if next < fitness {
                fitness = next;
                best = id;
            }
        }
        &self.entries[best].1
    }
    /// Preorder leaf entry indices expose stable tree layout for comparison.
    pub fn leaf_order(&self) -> Vec<usize> {
        fn walk(links: &[(u32, u32)], id: usize, out: &mut Vec<usize>) {
            match links[id] {
                (entry, 0) => out.push(entry as usize),
                (first, count) => {
                    for child in first..first + count {
                        walk(links, child as usize, out);
                    }
                }
            }
        }
        let mut result = Vec::with_capacity(self.entries.len());
        walk(&self.links, 0, &mut result);
        result
    }
    fn search(
        &self,
        id: usize,
        values: &[i64; 7],
        previous: Option<usize>,
    ) -> Result<usize, IndexError> {
        let (first, count) = self.links[id];
        if count == 0 {
            return Ok(id);
        }
        let mut best = previous;
        let mut fitness = previous.map_or(i64::MAX, |id| distance(&self.spaces[id], values));
        for child in first as usize..(first + count) as usize {
            let lower = distance(&self.spaces[child], values);
            if fitness > lower {
                let result = self.search(child, values, best)?;
                let next = if result == child {
                    lower
                } else {
                    distance(&self.spaces[result], values)
                };
                if fitness > next {
                    fitness = next;
                    best = Some(result);
                }
            }
        }
        best.ok_or(IndexError::NoResult)
    }
}
#[derive(Debug)]
pub struct ClimateSearch<'a, T> {
    index: &'a ClimateIndex<T>,
    last: Option<usize>,
}
impl<'a, T> ClimateSearch<'a, T> {
    pub fn find(&mut self, target: TargetPoint) -> Result<&'a T, IndexError> {
        let result = self.index.search(0, &target.parameters(), self.last)?;
        self.last = Some(result);
        let (entry, 0) = self.index.links[result] else {
            unreachable!("search returns a leaf");
        };
        Ok(&self.index.entries[entry as usize].1)
    }
}
