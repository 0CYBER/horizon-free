use super::*;
use std::collections::HashMap;

const INLINE_COORDINATES: usize = 8;
const LINEAR_SCAN_LIMIT: usize = 16;
const EMPTY_BUFFER: Option<Vec<f32>> = None;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplineError {
    EmptyPoints,
    NestingTooDeep,
    MissingCoordinate,
    NanConstant,
}
impl std::fmt::Display for SplineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NanConstant => "a compiled constant spline cannot be NaN",
            Self::EmptyPoints => "a multipoint spline requires at least one point",
            Self::NestingTooDeep => "spline nesting exceeds 256 levels",
            Self::MissingCoordinate => "spline coordinate has no density sampler",
        })
    }
}
impl std::error::Error for SplineError {}

#[derive(Clone, Debug)]
pub struct SplinePoint<C = usize> {
    pub location: f32,
    pub value: CubicSpline<C>,
    pub derivative: f32,
}
#[derive(Clone, Debug)]
pub struct MultipointSpline<C = usize> {
    pub(super) coordinate: C,
    pub(super) points: Vec<SplinePoint<C>>,
    pub(super) depth: usize,
}
/// An immutable spline definition. Coordinate indices are resolved against a
/// sampler table at compilation. Point order is retained, including NaNs and
/// equal locations; codec ordering constraints belong to serialized definitions.
#[derive(Clone, Debug)]
pub enum CubicSpline<C = usize> {
    Constant(f32),
    Multipoint(Arc<MultipointSpline<C>>),
}
impl<C: Copy> CubicSpline<C> {
    pub fn multipoint(coordinate: C, points: Vec<SplinePoint<C>>) -> Result<Self, SplineError> {
        let depth = points
            .iter()
            .map(|p| match &p.value {
                Self::Constant(_) => 0,
                Self::Multipoint(node) => node.depth,
            })
            .max()
            .ok_or(SplineError::EmptyPoints)?
            + 1;
        if depth > 256 {
            return Err(SplineError::NestingTooDeep);
        }
        Ok(Self::Multipoint(Arc::new(MultipointSpline {
            coordinate,
            points,
            depth,
        })))
    }
    #[cfg(test)]
    fn sample(
        &self,
        coordinate: &mut impl FnMut(C) -> Result<f32, SampleError>,
    ) -> Result<f32, SampleError> {
        let node = match self {
            Self::Constant(value) => return Ok(*value),
            Self::Multipoint(node) => node,
        };
        let input = coordinate(node.coordinate)?;
        // Vanilla binarySearch finds the first strictly greater location. Keep
        // this search even for manually supplied unsorted or NaN locations.
        let mut first = 0;
        let mut remaining = node.points.len();
        while remaining != 0 {
            let half = remaining / 2;
            let middle = first + half;
            if input < node.points[middle].location {
                remaining = half;
            } else {
                first = middle + 1;
                remaining -= half + 1;
            }
        }
        if first == 0 || first == node.points.len() {
            let endpoint = if first == 0 { 0 } else { first - 1 };
            let point = &node.points[endpoint];
            let value = point.value.sample(coordinate)?;
            return Ok(if point.derivative == 0.0 {
                value
            } else {
                value + point.derivative * (input - point.location)
            });
        }
        let left = &node.points[first - 1];
        let right = &node.points[first];
        let width = right.location - left.location;
        let alpha = (input - left.location) / width;
        let left_value = left.value.sample(coordinate)?;
        let right_value = right.value.sample(coordinate)?;
        let left_correction = left.derivative * width - (right_value - left_value);
        let right_correction = -right.derivative * width + (right_value - left_value);
        let linear = left_value + alpha * (right_value - left_value);
        let correction = left_correction + alpha * (right_correction - left_correction);
        Ok(linear + (alpha * (1.0 - alpha)) * correction)
    }
    /// Vanilla's conservative range analysis, with coordinate ranges supplied
    /// by the enclosing graph. Child ranges are evaluated in reference order.
    pub fn range(
        &self,
        coordinate: &mut impl FnMut(C) -> Interval,
    ) -> Result<Interval, IntervalError> {
        self.try_range(&mut |id| Ok(coordinate(id)))
    }
    /// Fallible coordinate range lookup, preserving early returns and error order.
    pub fn try_range<E: From<IntervalError>>(
        &self,
        coordinate: &mut impl FnMut(C) -> Result<Interval, E>,
    ) -> Result<Interval, E> {
        use super::arithmetic::{java_max, java_min};
        let node = match self {
            Self::Constant(value) => return Interval::exact(*value).map_err(E::from),
            Self::Multipoint(node) => node,
        };
        let input = coordinate(node.coordinate)?;
        if input.is_nai() {
            return Ok(input);
        }
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        let first = &node.points[0];
        let last = node.points.last().unwrap();
        let extend = |point: &SplinePoint<C>, input: f32, value: f32| {
            if point.derivative == 0.0 {
                value
            } else {
                value + point.derivative * (input - point.location)
            }
        };
        if input.min() < first.location {
            let value = first.value.try_range(coordinate)?;
            let low = extend(first, input.min(), value.min());
            let high = extend(first, input.min(), value.max());
            minimum = java_min(minimum, java_min(low, high));
            maximum = java_max(maximum, java_max(low, high));
        }
        if input.max() > last.location {
            let value = last.value.try_range(coordinate)?;
            let low = extend(last, input.max(), value.min());
            let high = extend(last, input.max(), value.max());
            minimum = java_min(minimum, java_min(low, high));
            maximum = java_max(maximum, java_max(low, high));
        }
        let ranges = node
            .points
            .iter()
            .map(|point| point.value.try_range(coordinate))
            .collect::<Result<Vec<_>, _>>()?;
        for range in &ranges {
            minimum = java_min(minimum, range.min());
            maximum = java_max(maximum, range.max());
        }
        for (points, ranges) in node.points.windows(2).zip(ranges.windows(2)) {
            let width = points[1].location - points[0].location;
            let left = ranges[0];
            let right = ranges[1];
            let derivative_left = points[0].derivative;
            let derivative_right = points[1].derivative;
            if derivative_left == 0.0 && derivative_right == 0.0 {
                continue;
            }
            let tangent_left = derivative_left * width;
            let tangent_right = derivative_right * width;
            let value_min = java_min(left.min(), right.min());
            let value_max = java_max(left.max(), right.max());
            let low_left = tangent_left - right.max() + left.min();
            let high_left = tangent_left - right.min() + left.max();
            let low_right = -tangent_right + right.min() - left.max();
            let high_right = -tangent_right + right.max() - left.min();
            minimum = java_min(minimum, value_min + 0.25 * java_min(low_left, low_right));
            maximum = java_max(maximum, value_max + 0.25 * java_max(high_left, high_right));
        }
        Interval::new(minimum, maximum).map_err(E::from)
    }
    pub(super) fn coordinates_mut(&mut self) -> Vec<&mut C> {
        match self {
            Self::Constant(_) => Vec::new(),
            Self::Multipoint(node) => {
                let node = Arc::make_mut(node);
                let mut coordinates = vec![&mut node.coordinate];
                for point in &mut node.points {
                    coordinates.extend(point.value.coordinates_mut());
                }
                coordinates
            }
        }
    }
    pub(super) fn try_map_coordinates<D: Copy, E>(
        &self,
        map: &mut impl FnMut(C) -> Result<D, E>,
    ) -> Result<CubicSpline<D>, E> {
        Ok(match self {
            Self::Constant(value) => CubicSpline::Constant(*value),
            Self::Multipoint(node) => {
                let coordinate = map(node.coordinate)?;
                let points = node
                    .points
                    .iter()
                    .map(|point| {
                        Ok(SplinePoint {
                            location: point.location,
                            value: point.value.try_map_coordinates(map)?,
                            derivative: point.derivative,
                        })
                    })
                    .collect::<Result<Vec<_>, E>>()?;
                CubicSpline::Multipoint(Arc::new(MultipointSpline {
                    coordinate,
                    points,
                    depth: node.depth,
                }))
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum CompiledChild {
    Constant(f32),
    Node(u32),
}

#[derive(Clone, Copy, Debug)]
struct CompiledNode {
    coordinate: u32,
    start: u32,
    len: u32,
    linear_scan: bool,
}

#[derive(Clone, Debug)]
struct CompiledSpline {
    root: CompiledChild,
    nodes: Box<[CompiledNode]>,
    locations: Box<[f32]>,
    derivatives: Box<[f32]>,
    children: Box<[CompiledChild]>,
}

#[derive(Default)]
struct CompiledSplineBuilder {
    nodes: Vec<CompiledNode>,
    locations: Vec<f32>,
    derivatives: Vec<f32>,
    children: Vec<CompiledChild>,
    ids: HashMap<usize, usize>,
    seen_nodes: HashMap<usize, u32>,
    coordinates: Vec<Arc<dyn DensitySampler>>,
}

impl CompiledSplineBuilder {
    fn compile(
        &mut self,
        spline: &CubicSpline,
        inputs: &[Arc<dyn DensitySampler>],
        depth: usize,
    ) -> Result<CompiledChild, SplineError> {
        let node = match spline {
            CubicSpline::Constant(value) => return Ok(CompiledChild::Constant(*value)),
            CubicSpline::Multipoint(node) => node,
        };
        if node.points.is_empty() {
            return Err(SplineError::EmptyPoints);
        }
        if depth >= 256 || depth.saturating_add(node.depth) > 256 {
            return Err(SplineError::NestingTooDeep);
        }
        let key = Arc::as_ptr(node) as usize;
        if let Some(&existing) = self.seen_nodes.get(&key) {
            return Ok(CompiledChild::Node(existing));
        }
        let coordinate = if let Some(&index) = self.ids.get(&node.coordinate) {
            index
        } else {
            let input = inputs
                .get(node.coordinate)
                .ok_or(SplineError::MissingCoordinate)?;
            let index = self.coordinates.len();
            self.coordinates.push(Arc::clone(input));
            self.ids.insert(node.coordinate, index);
            index
        };
        let node_index =
            u32::try_from(self.nodes.len()).expect("compiled spline node count exceeds u32::MAX");
        let start_usize = self.locations.len();
        let start =
            u32::try_from(start_usize).expect("compiled spline point count exceeds u32::MAX");
        let len = u32::try_from(node.points.len())
            .expect("compiled spline node point count exceeds u32::MAX");
        let coord_u32 =
            u32::try_from(coordinate).expect("compiled spline coordinate count exceeds u32::MAX");

        let mut linear_scan = node.points.len() <= LINEAR_SCAN_LIMIT;
        let mut prev_loc = f32::NEG_INFINITY;
        self.locations.reserve(node.points.len());
        self.derivatives.reserve(node.points.len());
        self.children.reserve(node.points.len());
        for (i, point) in node.points.iter().enumerate() {
            let loc = point.location;
            if loc.is_nan() || (i > 0 && prev_loc > loc) {
                linear_scan = false;
            }
            prev_loc = loc;
            self.locations.push(loc);
            self.derivatives.push(point.derivative);
            self.children.push(CompiledChild::Constant(0.0));
        }
        self.nodes.push(CompiledNode {
            coordinate: coord_u32,
            start,
            len,
            linear_scan,
        });
        for (offset, point) in node.points.iter().enumerate() {
            let child = self.compile(&point.value, inputs, depth + 1)?;
            self.children[start_usize + offset] = child;
        }
        self.seen_nodes.insert(key, node_index);
        Ok(CompiledChild::Node(node_index))
    }

    fn finish(self, root: CompiledChild) -> (CompiledSpline, Box<[Arc<dyn DensitySampler>]>) {
        (
            CompiledSpline {
                root,
                nodes: self.nodes.into_boxed_slice(),
                locations: self.locations.into_boxed_slice(),
                derivatives: self.derivatives.into_boxed_slice(),
                children: self.children.into_boxed_slice(),
            },
            self.coordinates.into_boxed_slice(),
        )
    }
}

#[inline]
fn find_interval(locations: &[f32], input: f32, linear_scan: bool) -> usize {
    if linear_scan {
        // For non-decreasing, NaN-free locations, `locations.len() - count(input < loc)`
        // matches vanilla's binarySearch for every input, including NaN (`greater == 0`).
        let mut greater = 0;
        for &location in locations {
            greater += usize::from(input < location);
        }
        return locations.len() - greater;
    }
    // Vanilla binarySearch finds the first strictly greater location. Keep
    // this search even for manually supplied unsorted or NaN locations.
    let mut first = 0;
    let mut remaining = locations.len();
    while remaining != 0 {
        let half = remaining / 2;
        let middle = first + half;
        if input < locations[middle] {
            remaining = half;
        } else {
            first = middle + 1;
            remaining -= half + 1;
        }
    }
    first
}

#[derive(Clone, Copy)]
struct NeededTail<'a> {
    mask: &'a [u64],
    size_y: usize,
    valid_y_mask: u64,
    start_col: usize,
    first_col_bits: u64,
}

#[inline]
fn eval_needed_cells(
    output: &mut [f32],
    tail: NeededTail<'_>,
    mut eval_at: impl FnMut(usize) -> f32,
) {
    let mut bits = tail.first_col_bits;
    let base = tail.start_col * tail.size_y;
    while bits != 0 {
        let y = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        let index = base + y;
        output[index] = eval_at(index);
    }
    for (offset, &col_mask) in tail.mask[tail.start_col + 1..].iter().enumerate() {
        let mut bits = col_mask & tail.valid_y_mask;
        if bits == 0 {
            continue;
        }
        let base = (tail.start_col + 1 + offset) * tail.size_y;
        while bits != 0 {
            let y = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let index = base + y;
            output[index] = eval_at(index);
        }
    }
}

impl CompiledSpline {
    fn eval_node<F>(&self, mut node_index: u32, coordinate: &mut F) -> Result<f32, SampleError>
    where
        F: FnMut(usize) -> Result<f32, SampleError>,
    {
        loop {
            let node = self.nodes[node_index as usize];
            let input = coordinate(node.coordinate as usize)?;
            let start = node.start as usize;
            let len = node.len as usize;
            let locations = &self.locations[start..start + len];
            let first = find_interval(locations, input, node.linear_scan);
            if first == 0 || first == len {
                let endpoint = if first == 0 { 0 } else { first - 1 };
                let idx = start + endpoint;
                let derivative = self.derivatives[idx];
                match self.children[idx] {
                    CompiledChild::Constant(value) => {
                        return Ok(if derivative == 0.0 {
                            value
                        } else {
                            value + derivative * (input - locations[endpoint])
                        });
                    }
                    CompiledChild::Node(child) if derivative == 0.0 => {
                        node_index = child;
                        continue;
                    }
                    CompiledChild::Node(child) => {
                        let value = self.eval_node(child, coordinate)?;
                        return Ok(value + derivative * (input - locations[endpoint]));
                    }
                }
            }
            let left_idx = start + first - 1;
            let right_idx = start + first;
            let left_loc = locations[first - 1];
            let right_loc = locations[first];
            let width = right_loc - left_loc;
            let alpha = (input - left_loc) / width;
            let left_value = match self.children[left_idx] {
                CompiledChild::Constant(value) => value,
                CompiledChild::Node(child) => self.eval_node(child, coordinate)?,
            };
            let right_value = match self.children[right_idx] {
                CompiledChild::Constant(value) => value,
                CompiledChild::Node(child) => self.eval_node(child, coordinate)?,
            };
            let delta = right_value - left_value;
            let left_correction = self.derivatives[left_idx] * width - delta;
            let right_correction = -self.derivatives[right_idx] * width + delta;
            let linear = left_value + alpha * delta;
            let correction = left_correction + alpha * (right_correction - left_correction);
            return Ok(linear + (alpha * (1.0 - alpha)) * correction);
        }
    }

    fn eval_node_infallible(&self, mut node_index: u32, coords: &[f32]) -> f32 {
        loop {
            let node = self.nodes[node_index as usize];
            let input = coords[node.coordinate as usize];
            let start = node.start as usize;
            let len = node.len as usize;
            let locations = &self.locations[start..start + len];
            let first = find_interval(locations, input, node.linear_scan);
            if first == 0 || first == len {
                let endpoint = if first == 0 { 0 } else { first - 1 };
                let idx = start + endpoint;
                let derivative = self.derivatives[idx];
                match self.children[idx] {
                    CompiledChild::Constant(value) => {
                        return if derivative == 0.0 {
                            value
                        } else {
                            value + derivative * (input - locations[endpoint])
                        };
                    }
                    CompiledChild::Node(child) if derivative == 0.0 => {
                        node_index = child;
                        continue;
                    }
                    CompiledChild::Node(child) => {
                        let value = self.eval_node_infallible(child, coords);
                        return value + derivative * (input - locations[endpoint]);
                    }
                }
            }
            let left_idx = start + first - 1;
            let right_idx = start + first;
            let left_loc = locations[first - 1];
            let right_loc = locations[first];
            let width = right_loc - left_loc;
            let alpha = (input - left_loc) / width;
            let left_value = match self.children[left_idx] {
                CompiledChild::Constant(value) => value,
                CompiledChild::Node(child) => self.eval_node_infallible(child, coords),
            };
            let right_value = match self.children[right_idx] {
                CompiledChild::Constant(value) => value,
                CompiledChild::Node(child) => self.eval_node_infallible(child, coords),
            };
            let delta = right_value - left_value;
            let left_correction = self.derivatives[left_idx] * width - delta;
            let right_correction = -self.derivatives[right_idx] * width + delta;
            let linear = left_value + alpha * delta;
            let correction = left_correction + alpha * (right_correction - left_correction);
            return linear + (alpha * (1.0 - alpha)) * correction;
        }
    }

    fn eval_volume_infallible(
        &self,
        root: u32,
        slices: &[&[f32]],
        output: &mut [f32],
        start_index: usize,
    ) {
        let end = start_index + output.len();
        match slices {
            [c0] => {
                for (dst, &v0) in output.iter_mut().zip(&c0[start_index..end]) {
                    *dst = self.eval_node_infallible(root, &[v0]);
                }
            }
            [c0, c1] => {
                for ((dst, &v0), &v1) in output
                    .iter_mut()
                    .zip(&c0[start_index..end])
                    .zip(&c1[start_index..end])
                {
                    *dst = self.eval_node_infallible(root, &[v0, v1]);
                }
            }
            [c0, c1, c2] => {
                for (((dst, &v0), &v1), &v2) in output
                    .iter_mut()
                    .zip(&c0[start_index..end])
                    .zip(&c1[start_index..end])
                    .zip(&c2[start_index..end])
                {
                    *dst = self.eval_node_infallible(root, &[v0, v1, v2]);
                }
            }
            [c0, c1, c2, c3] => {
                for ((((dst, &v0), &v1), &v2), &v3) in output
                    .iter_mut()
                    .zip(&c0[start_index..end])
                    .zip(&c1[start_index..end])
                    .zip(&c2[start_index..end])
                    .zip(&c3[start_index..end])
                {
                    *dst = self.eval_node_infallible(root, &[v0, v1, v2, v3]);
                }
            }
            _ => {
                let mut coords = [0.0_f32; INLINE_COORDINATES];
                let count = slices.len();
                for (offset, dst) in output.iter_mut().enumerate() {
                    let cell = start_index + offset;
                    for (slot, slice) in coords[..count].iter_mut().zip(slices) {
                        *slot = slice[cell];
                    }
                    *dst = self.eval_node_infallible(root, &coords[..count]);
                }
            }
        }
    }

    fn eval_needed_infallible(
        &self,
        root: u32,
        slices: &[&[f32]],
        output: &mut [f32],
        tail: NeededTail<'_>,
    ) {
        match slices {
            [c0] => eval_needed_cells(output, tail, |i| self.eval_node_infallible(root, &[c0[i]])),
            [c0, c1] => eval_needed_cells(output, tail, |i| {
                self.eval_node_infallible(root, &[c0[i], c1[i]])
            }),
            [c0, c1, c2] => eval_needed_cells(output, tail, |i| {
                self.eval_node_infallible(root, &[c0[i], c1[i], c2[i]])
            }),
            [c0, c1, c2, c3] => eval_needed_cells(output, tail, |i| {
                self.eval_node_infallible(root, &[c0[i], c1[i], c2[i], c3[i]])
            }),
            _ => {
                let mut coords = [0.0_f32; INLINE_COORDINATES];
                let count = slices.len();
                eval_needed_cells(output, tail, |i| {
                    for (slot, slice) in coords[..count].iter_mut().zip(slices) {
                        *slot = slice[i];
                    }
                    self.eval_node_infallible(root, &coords[..count])
                });
            }
        }
    }
}

enum BufferSlots {
    Inline([Option<Vec<f32>>; INLINE_COORDINATES]),
    Heap(Vec<Option<Vec<f32>>>),
}

impl BufferSlots {
    fn new(count: usize) -> Self {
        if count <= INLINE_COORDINATES {
            Self::Inline([EMPTY_BUFFER; INLINE_COORDINATES])
        } else {
            let mut slots = Vec::with_capacity(count);
            slots.resize_with(count, Option::default);
            Self::Heap(slots)
        }
    }

    fn as_mut_slice(&mut self) -> &mut [Option<Vec<f32>>] {
        match self {
            Self::Inline(slots) => slots,
            Self::Heap(slots) => slots,
        }
    }

    fn release(self, context: &mut SamplerContext) {
        match self {
            Self::Inline(slots) => context.scratch.extend(slots.into_iter().flatten()),
            Self::Heap(slots) => context.scratch.extend(slots.into_iter().flatten()),
        }
    }
}

#[cold]
#[inline(never)]
fn fill_coordinate(
    slot: &mut Option<Vec<f32>>,
    sampler: &dyn DensitySampler,
    context: &mut SamplerContext,
    volume: DensityVolume,
    needed: Option<&[u64]>,
    index: usize,
) -> Result<f32, SampleError> {
    let mut buffer = context.scratch.pop().unwrap_or_default();
    buffer.resize(volume.size(), 0.0);
    if let Err(error) = sample_with(sampler, context, &mut buffer, volume, needed) {
        context.scratch.push(buffer);
        return Err(error);
    }
    let value = buffer[index];
    *slot = Some(buffer);
    Ok(value)
}

#[derive(Clone, Debug)]
pub struct SplineSampler {
    compiled: CompiledSpline,
    coordinates: Box<[Arc<dyn DensitySampler>]>,
}
impl SplineSampler {
    /// Equal coordinate IDs share evaluation results. Structural function
    /// identity and deduplication are supplied by the density graph compiler.
    pub fn new(
        spline: &CubicSpline,
        inputs: &[Arc<dyn DensitySampler>],
    ) -> Result<Self, SplineError> {
        if matches!(spline, CubicSpline::Constant(value) if value.is_nan()) {
            return Err(SplineError::NanConstant);
        }
        let mut builder = CompiledSplineBuilder::default();
        let root = builder.compile(spline, inputs, 0)?;
        let (compiled, coordinates) = builder.finish(root);
        Ok(Self {
            compiled,
            coordinates,
        })
    }
    pub fn coordinate_count(&self) -> usize {
        self.coordinates.len()
    }

    fn sample_spline_volume(
        &self,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
        needed: Option<&[u64]>,
    ) -> Result<(), SampleError> {
        check_buffer(output, volume)?;
        let root = match self.compiled.root {
            CompiledChild::Constant(value) => {
                output.fill(value);
                return Ok(());
            }
            CompiledChild::Node(root) => root,
        };
        check_exact_buffer(output, volume)?;
        let mut buffer_slots = BufferSlots::new(self.coordinates.len());
        let result = match needed {
            None => {
                self.sample_unmasked(root, context, output, volume, buffer_slots.as_mut_slice())
            }
            Some(mask) => self.sample_masked(
                root,
                context,
                output,
                volume,
                mask,
                buffer_slots.as_mut_slice(),
            ),
        };
        // Release in coordinate-ID order, also after a later lazy child fails.
        buffer_slots.release(context);
        result
    }

    fn sample_unmasked(
        &self,
        root: u32,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
        slots: &mut [Option<Vec<f32>>],
    ) -> Result<(), SampleError> {
        let coord_count = self.coordinates.len();
        let can_inline = coord_count <= INLINE_COORDINATES;
        let mut loaded = 0_usize;
        let mut index = 0_usize;
        while index < output.len() {
            let value = self.compiled.eval_node(root, &mut |id| {
                if let Some(buf) = slots[id].as_deref() {
                    return Ok(buf[index]);
                }
                let value = fill_coordinate(
                    &mut slots[id],
                    &*self.coordinates[id],
                    context,
                    volume,
                    None,
                    index,
                )?;
                loaded += 1;
                Ok(value)
            })?;
            output[index] = value;
            index += 1;
            if can_inline && loaded == coord_count {
                break;
            }
        }
        if index < output.len() {
            let mut slices: [&[f32]; INLINE_COORDINATES] = [&[]; INLINE_COORDINATES];
            for (dst, src) in slices[..coord_count].iter_mut().zip(&slots[..coord_count]) {
                *dst = src.as_deref().unwrap();
            }
            self.compiled.eval_volume_infallible(
                root,
                &slices[..coord_count],
                &mut output[index..],
                index,
            );
        }
        Ok(())
    }

    fn sample_masked(
        &self,
        root: u32,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
        mask: &[u64],
        slots: &mut [Option<Vec<f32>>],
    ) -> Result<(), SampleError> {
        let [size_x, size_y, size_z] = volume.sizes();
        let columns = size_x * size_z;
        let active_mask = &mask[..columns];
        let valid_y_mask = u64::MAX >> (64 - size_y);
        let coord_count = self.coordinates.len();
        let can_inline = coord_count <= INLINE_COORDINATES;
        let mut loaded = 0_usize;
        let mut col = 0_usize;
        let mut pending_bits = 0_u64;
        'lazy: while col < columns {
            let mut bits = active_mask[col] & valid_y_mask;
            let base = col * size_y;
            while bits != 0 {
                let y = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let index = base + y;
                let value = self.compiled.eval_node(root, &mut |id| {
                    if let Some(buf) = slots[id].as_deref() {
                        return Ok(buf[index]);
                    }
                    let value = fill_coordinate(
                        &mut slots[id],
                        &*self.coordinates[id],
                        context,
                        volume,
                        Some(mask),
                        index,
                    )?;
                    loaded += 1;
                    Ok(value)
                })?;
                output[index] = value;
                if can_inline && loaded == coord_count {
                    pending_bits = bits;
                    break 'lazy;
                }
            }
            col += 1;
        }
        if col < columns {
            let mut slices: [&[f32]; INLINE_COORDINATES] = [&[]; INLINE_COORDINATES];
            for (dst, src) in slices[..coord_count].iter_mut().zip(&slots[..coord_count]) {
                *dst = src.as_deref().unwrap();
            }
            self.compiled.eval_needed_infallible(
                root,
                &slices[..coord_count],
                output,
                NeededTail {
                    mask: active_mask,
                    size_y,
                    valid_y_mask,
                    start_col: col,
                    first_col_bits: pending_bits,
                },
            );
        }
        Ok(())
    }
}
impl DensitySampler for SplineSampler {
    fn pointwise_inputs(&self) -> Option<Vec<&Arc<dyn DensitySampler>>> {
        Some(self.coordinates.iter().collect())
    }
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let root = match self.compiled.root {
            CompiledChild::Constant(value) => return Ok(value),
            CompiledChild::Node(root) => root,
        };
        if self.coordinates.len() <= INLINE_COORDINATES {
            let mut values = [f32::NAN; INLINE_COORDINATES];
            return self.compiled.eval_node(root, &mut |id| {
                if values[id].is_nan() {
                    values[id] = self.coordinates[id].sample_value(context, position)?;
                }
                Ok(values[id])
            });
        }
        context.with_buffer(self.coordinates.len(), |context, values| {
            values.fill(f32::NAN);
            self.compiled.eval_node(root, &mut |id| {
                if values[id].is_nan() {
                    values[id] = self.coordinates[id].sample_value(context, position)?;
                }
                Ok(values[id])
            })
        })
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        self.sample_spline_volume(context, output, volume, None)
    }
    fn sample_needed(
        &self,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
        needed: &[u64],
    ) -> Result<(), SampleError> {
        self.sample_spline_volume(context, output, volume, usable_mask(needed, volume))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{noise::PerlinNoise, random::LegacyRandom};
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn pt(location: f32, derivative: f32, value: CubicSpline) -> SplinePoint {
        SplinePoint {
            location,
            value,
            derivative,
        }
    }

    fn cpt(location: f32, derivative: f32, value: f32) -> SplinePoint {
        pt(location, derivative, CubicSpline::Constant(value))
    }

    fn assert_f32_exact(actual: f32, expected: f32, label: &str) {
        if actual.is_nan() && expected.is_nan() {
            return;
        }
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{label}: actual={actual} ({:#010x}) expected={expected} ({:#010x})",
            actual.to_bits(),
            expected.to_bits(),
        );
    }

    fn noise_input(seed: i64, xz_scale: f64, y_scale: f64) -> Arc<dyn DensitySampler> {
        Arc::new(NoiseSampler {
            noise: Arc::new(PerlinNoise::new(&mut LegacyRandom::new(seed))),
            xz_scale,
            y_scale,
        })
    }

    /// Depth-4, 4-coordinate Overworld-style spline exercising constant leaves,
    /// shared `Arc` subtrees, zero / `-0.0` / non-zero derivatives, and 11-point nodes.
    fn build_depth_4_overworld_style_spline() -> CubicSpline {
        let leaf_ridges = CubicSpline::multipoint(
            3,
            vec![
                cpt(-0.75, 0.4, -0.35),
                cpt(-0.2, 0.0, 0.05),
                cpt(0.15, -0.0, 0.42),
                cpt(0.6, -0.5, 0.95),
            ],
        )
        .unwrap();

        let shared_ridges_folded = CubicSpline::multipoint(
            2,
            vec![
                cpt(-0.9, 0.25, -0.5),
                pt(-0.3, 0.0, leaf_ridges.clone()),
                pt(0.25, 0.75, leaf_ridges),
                cpt(0.8, -0.2, 1.2),
            ],
        )
        .unwrap();

        let erosion_branch = CubicSpline::multipoint(
            1,
            vec![
                pt(-0.85, 0.0, shared_ridges_folded.clone()),
                cpt(-0.4, 0.3, 0.1),
                pt(0.0, -0.25, shared_ridges_folded.clone()),
                cpt(0.45, 0.0, -0.15),
                pt(0.9, 0.6, shared_ridges_folded),
            ],
        )
        .unwrap();

        // 11-point root (same max width as overworld/offset and overworld/factor).
        CubicSpline::multipoint(
            0,
            vec![
                cpt(-1.1, 0.0, -0.8),
                cpt(-0.85, 0.15, -0.45),
                cpt(-0.6, 0.0, -0.15),
                pt(-0.35, 0.4, erosion_branch.clone()),
                cpt(-0.15, 0.0, 0.02),
                pt(0.05, -0.3, erosion_branch.clone()),
                cpt(0.25, 0.5, 0.3),
                pt(0.45, 0.0, erosion_branch),
                cpt(0.65, -0.2, 0.6),
                cpt(0.85, 0.35, 0.9),
                cpt(1.05, -0.45, 1.15),
            ],
        )
        .unwrap()
    }

    #[test]
    fn compiled_arena_matches_recursive_reference_bit_for_bit_across_depth_4_splines() {
        let root = build_depth_4_overworld_style_spline();
        let inputs = vec![
            noise_input(11, 0.09, 0.05),
            noise_input(23, 0.13, 0.07),
            noise_input(37, 0.17, 0.09),
            noise_input(53, 0.21, 0.11),
        ];
        let sampler = SplineSampler::new(&root, &inputs).unwrap();
        assert_eq!(sampler.coordinate_count(), 4);
        // Shared `Arc` subtrees are deduplicated into 4 unique compiled nodes.
        assert_eq!(sampler.compiled.nodes.len(), 4);
        assert!(sampler.compiled.nodes.iter().all(|node| node.linear_scan));

        // Scalar evaluation for <= INLINE_COORDINATES uses the stack array without scratch.
        let mut scalar_context = SamplerContext::default();
        let volume = DensityVolume::new([5, 9, 5], [-12, -32, 7], [3, 5, 3]).unwrap();
        let mut context = SamplerContext::default();
        let mut bulk = vec![0.0; volume.size()];
        sampler
            .sample_volume(&mut context, &mut bulk, volume)
            .unwrap();

        let mut coord_volumes = vec![vec![0.0; volume.size()]; inputs.len()];
        for (input, buf) in inputs.iter().zip(&mut coord_volumes) {
            input.sample_volume(&mut context, buf, volume).unwrap();
        }

        visit(volume, |index, position| {
            let expected = root.sample(&mut |id| Ok(coord_volumes[id][index])).unwrap();
            assert_f32_exact(bulk[index], expected, "depth-4 bulk");

            let scalar = sampler.sample_value(&mut scalar_context, position).unwrap();
            let expected_scalar = root
                .sample(&mut |id| inputs[id].sample_value(&mut context, position))
                .unwrap();
            assert_f32_exact(scalar, expected_scalar, "depth-4 scalar");
            Ok(())
        })
        .unwrap();
        assert!(scalar_context.scratch.is_empty());

        // Constant root spline: oversized buffer tail fill + zero coordinates + NaN rejection.
        let constant = SplineSampler::new(&CubicSpline::Constant(-0.0), &inputs).unwrap();
        assert_eq!(constant.coordinate_count(), 0);
        assert_eq!(
            constant
                .sample_value(&mut scalar_context, [1, 2, 3])
                .unwrap()
                .to_bits(),
            (-0.0_f32).to_bits()
        );
        let mut oversized = vec![19.0; volume.size() + 3];
        constant
            .sample_volume(&mut context, &mut oversized, volume)
            .unwrap();
        assert!(
            oversized
                .iter()
                .all(|v| v.to_bits() == (-0.0_f32).to_bits())
        );
        let mut oversized_needed = vec![19.0; volume.size() + 2];
        let mask = vec![0b101_u64; 25];
        constant
            .sample_needed(&mut context, &mut oversized_needed, volume, &mask)
            .unwrap();
        assert!(
            oversized_needed
                .iter()
                .all(|v| v.to_bits() == (-0.0_f32).to_bits())
        );
        assert_eq!(
            SplineSampler::new(&CubicSpline::Constant(f32::NAN), &inputs).unwrap_err(),
            SplineError::NanConstant
        );
    }

    #[test]
    fn compiled_arena_preserves_nan_unsorted_duplicate_and_wide_point_semantics() {
        #[derive(Debug)]
        struct FixedValues(Vec<f32>);
        impl DensitySampler for FixedValues {
            fn sample_value(
                &self,
                _: &mut SamplerContext,
                position: [i32; 3],
            ) -> Result<f32, SampleError> {
                Ok(self.0[position[0] as usize])
            }
            fn sample_volume(
                &self,
                _: &mut SamplerContext,
                output: &mut [f32],
                volume: DensityVolume,
            ) -> Result<(), SampleError> {
                check_buffer(output, volume)?;
                output[..volume.size()].copy_from_slice(&self.0[..volume.size()]);
                Ok(())
            }
        }

        let probe_inputs = vec![
            f32::NEG_INFINITY,
            -10.0,
            -2.0,
            -1.0,
            -0.5,
            -0.0,
            0.0,
            0.25,
            1.0,
            2.5,
            5.0,
            7.5,
            10.0,
            20.0,
            f32::INFINITY,
            f32::NAN,
        ];
        let volume = DensityVolume::new([probe_inputs.len() as i32, 1, 1], [0; 3], [1; 3]).unwrap();
        let coord: Arc<dyn DensitySampler> = Arc::new(FixedValues(probe_inputs.clone()));

        let unsorted = CubicSpline::multipoint(
            0,
            vec![
                cpt(1.0, 0.5, 10.0),
                cpt(-1.0, -0.25, 20.0),
                cpt(0.0, 1.0, 30.0),
            ],
        )
        .unwrap();

        let nan_location = CubicSpline::multipoint(
            0,
            vec![
                cpt(5.0, 0.5, 1.0),
                cpt(f32::NAN, 0.0, 2.0),
                cpt(10.0, -0.5, 3.0),
            ],
        )
        .unwrap();

        let duplicate_zeros = CubicSpline::multipoint(
            0,
            vec![
                cpt(-0.0, 0.0, 1.0),
                cpt(0.0, 2.0, 2.0),
                cpt(0.0, -0.0, 3.0),
                cpt(1.0, 1.0, 4.0),
            ],
        )
        .unwrap();

        let wide_25 = CubicSpline::multipoint(
            0,
            (0..25_usize)
                .map(|i| {
                    cpt(
                        -2.0 + 0.5 * i as f32,
                        if i.is_multiple_of(2) {
                            0.2 * i as f32
                        } else {
                            -0.15 * i as f32
                        },
                        i as f32 * 1.25,
                    )
                })
                .collect(),
        )
        .unwrap();

        // NaN input must choose the LAST endpoint (`first == len`), not index 0.
        let nan_endpoint =
            CubicSpline::multipoint(0, vec![cpt(-1.0, 0.0, 100.0), cpt(1.0, -0.0, 200.0)]).unwrap();

        // Nested Constant(NaN) is legal in a multipoint spline and propagates only when hit.
        let nested_nan_const =
            CubicSpline::multipoint(0, vec![cpt(0.0, 0.0, 42.0), cpt(2.0, 0.0, f32::NAN)]).unwrap();

        let cases = [
            ("unsorted", unsorted, false),
            ("nan_location", nan_location, false),
            ("duplicate_zeros", duplicate_zeros, true),
            ("wide_25", wide_25, false),
            ("nan_endpoint", nan_endpoint, true),
            ("nested_nan_const", nested_nan_const, true),
        ];

        let mut context = SamplerContext::default();
        for (name, spline, expect_linear) in cases {
            let sampler = SplineSampler::new(&spline, std::slice::from_ref(&coord)).unwrap();
            assert_eq!(
                sampler.compiled.nodes[0].linear_scan, expect_linear,
                "{name}"
            );

            let mut bulk = vec![0.0; volume.size()];
            sampler
                .sample_volume(&mut context, &mut bulk, volume)
                .unwrap();

            for (i, &input) in probe_inputs.iter().enumerate() {
                let expected = spline.sample(&mut |_| Ok(input)).unwrap();
                let scalar = sampler
                    .sample_value(&mut context, [i as i32, 0, 0])
                    .unwrap();
                assert_f32_exact(bulk[i], expected, &format!("{name} bulk[{i}] ({input})"));
                assert_f32_exact(scalar, expected, &format!("{name} scalar[{i}] ({input})"));
            }
        }
    }

    #[test]
    fn compiled_arena_overflow_coordinates_above_inline_limit_matches_reference() {
        const COORDS: usize = INLINE_COORDINATES + 4;
        let inputs: Vec<Arc<dyn DensitySampler>> = (0..COORDS)
            .map(|i| noise_input(100 + i as i64, 0.11 + 0.01 * i as f64, 0.07))
            .collect();

        // 12-coordinate spline tree exercising overflow scratch paths (> INLINE_COORDINATES).
        let mut spline =
            CubicSpline::multipoint(COORDS - 1, vec![cpt(-0.5, 0.2, -1.5), cpt(0.5, -0.3, 2.25)])
                .unwrap();
        for coord in (0..COORDS - 1).rev() {
            let first = if coord.is_multiple_of(2) {
                cpt(-0.6, 0.0, coord as f32 * 0.25)
            } else {
                pt(-0.6, 0.0, spline.clone())
            };
            spline = CubicSpline::multipoint(
                coord,
                vec![first, pt(0.0, 0.4, spline.clone()), pt(0.6, -0.2, spline)],
            )
            .unwrap();
        }

        let sampler = SplineSampler::new(&spline, &inputs).unwrap();
        assert_eq!(sampler.coordinate_count(), COORDS);
        assert_eq!(sampler.compiled.nodes.len(), COORDS);

        let volume = DensityVolume::new([3, 5, 3], [-4, -8, 2], [2, 3, 2]).unwrap();
        let mut context = SamplerContext::default();
        let mut bulk = vec![0.0; volume.size()];
        sampler
            .sample_volume(&mut context, &mut bulk, volume)
            .unwrap();

        let mask = [
            0b10101_u64,
            0b01010,
            0,
            0b11111,
            0b00100,
            0b10001,
            0,
            0b01110,
            0b11011,
        ];
        let mut masked = vec![19.0; volume.size()];
        sampler
            .sample_needed(&mut context, &mut masked, volume, &mask)
            .unwrap();

        let mut coord_volumes = vec![vec![0.0; volume.size()]; COORDS];
        for (input, buf) in inputs.iter().zip(&mut coord_volumes) {
            input.sample_volume(&mut context, buf, volume).unwrap();
        }

        let mut scalar_context = SamplerContext::default();
        visit(volume, |index, position| {
            let expected_bulk = spline
                .sample(&mut |id| Ok(coord_volumes[id][index]))
                .unwrap();
            assert_f32_exact(bulk[index], expected_bulk, "overflow bulk");
            let scalar = sampler.sample_value(&mut scalar_context, position).unwrap();
            let expected_scalar = spline
                .sample(&mut |id| inputs[id].sample_value(&mut context, position))
                .unwrap();
            assert_f32_exact(scalar, expected_scalar, "overflow scalar");
            let col = index / 5;
            let y = index % 5;
            if (mask[col] >> y) & 1 == 1 {
                assert_f32_exact(masked[index], expected_bulk, "overflow sample_needed");
            } else {
                assert_eq!(masked[index].to_bits(), 19.0_f32.to_bits());
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(scalar_context.scratch.len(), 1);

        // Overflow `sample_value` path still retries NaN coordinates (id >= INLINE_COORDINATES).
        #[derive(Debug)]
        struct NanCounter(AtomicUsize);
        impl DensitySampler for NanCounter {
            fn sample_value(
                &self,
                _: &mut SamplerContext,
                _: [i32; 3],
            ) -> Result<f32, SampleError> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(f32::NAN)
            }
        }
        let nan_coord = Arc::new(NanCounter(AtomicUsize::new(0)));
        let inner = CubicSpline::multipoint(COORDS - 1, vec![cpt(0.0, -0.0, 9.0)]).unwrap();
        let mut chain = CubicSpline::multipoint(
            COORDS - 1,
            vec![pt(-1.0, 0.0, inner.clone()), pt(1.0, 0.0, inner)],
        )
        .unwrap();
        for coord in (0..COORDS - 1).rev() {
            chain = CubicSpline::multipoint(coord, vec![pt(0.0, 0.0, chain)]).unwrap();
        }
        let mut nan_inputs: Vec<Arc<dyn DensitySampler>> = (0..COORDS - 1)
            .map(|_| Arc::new(ConstantSampler(0.0)) as Arc<dyn DensitySampler>)
            .collect();
        nan_inputs.push(Arc::clone(&nan_coord) as Arc<dyn DensitySampler>);
        let nan_sampler = SplineSampler::new(&chain, &nan_inputs).unwrap();
        assert_eq!(nan_sampler.sample_value(&mut context, [0; 3]), Ok(9.0));
        assert_eq!(nan_coord.0.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn lazy_coordinate_evaluation_skips_unreachable_branches_and_reuses_warm_scratch() {
        #[derive(Debug)]
        struct LoggedCoord {
            id: usize,
            inner: Arc<dyn DensitySampler>,
            log: Arc<Mutex<Vec<usize>>>,
            scalar_calls: AtomicUsize,
            bulk_calls: AtomicUsize,
        }
        impl DensitySampler for LoggedCoord {
            fn sample_value(
                &self,
                context: &mut SamplerContext,
                position: [i32; 3],
            ) -> Result<f32, SampleError> {
                self.scalar_calls.fetch_add(1, Ordering::Relaxed);
                self.log.lock().unwrap().push(self.id);
                self.inner.sample_value(context, position)
            }
            fn sample_volume(
                &self,
                context: &mut SamplerContext,
                buffer: &mut [f32],
                volume: DensityVolume,
            ) -> Result<(), SampleError> {
                self.bulk_calls.fetch_add(1, Ordering::Relaxed);
                self.log.lock().unwrap().push(self.id);
                self.inner.sample_volume(context, buffer, volume)
            }
        }

        // Root reads coord 0. Below -0.5 -> Constant(3.0). Between 0.5 and 1.5 ->
        // interpolates left = coord 2, right = coord 1. Above 1.5 -> coord 3.
        let make_leaf = |coord: usize, val: f32| {
            CubicSpline::multipoint(coord, vec![cpt(0.0, 1.0, val)]).unwrap()
        };
        let root = CubicSpline::multipoint(
            0,
            vec![
                cpt(-0.5, 0.0, 3.0),
                pt(0.5, 0.0, make_leaf(2, 10.0)),
                pt(1.5, 0.0, make_leaf(1, 20.0)),
                pt(2.5, 0.0, make_leaf(3, 30.0)),
            ],
        )
        .unwrap();

        let log = Arc::new(Mutex::new(Vec::new()));
        let wrap = |id: usize, inner: Arc<dyn DensitySampler>| {
            Arc::new(LoggedCoord {
                id,
                inner,
                log: Arc::clone(&log),
                scalar_calls: AtomicUsize::new(0),
                bulk_calls: AtomicUsize::new(0),
            })
        };

        // Case 1: Ocean-like tile (coord 0 == -1.0 everywhere) -> coords 1, 2, 3 never evaluated.
        let ocean_coords = [
            wrap(0, Arc::new(ConstantSampler(-1.0))),
            wrap(1, Arc::new(ConstantSampler(0.0))),
            wrap(2, Arc::new(ConstantSampler(0.0))),
            wrap(3, Arc::new(ConstantSampler(0.0))),
        ];
        let ocean_inputs: Vec<Arc<dyn DensitySampler>> = ocean_coords
            .iter()
            .map(|c| Arc::clone(c) as Arc<dyn DensitySampler>)
            .collect();
        let ocean_sampler = SplineSampler::new(&root, &ocean_inputs).unwrap();
        let mut context = SamplerContext::default();
        let volume = DensityVolume::new([4, 2, 4], [0; 3], [1; 3]).unwrap();
        let mut output = vec![0.0; volume.size()];
        ocean_sampler
            .sample_volume(&mut context, &mut output, volume)
            .unwrap();
        assert!(output.iter().all(|&v| v == 3.0));
        assert_eq!(ocean_coords[0].bulk_calls.load(Ordering::Relaxed), 1);
        assert_eq!(ocean_coords[1].bulk_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ocean_coords[2].bulk_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ocean_coords[3].bulk_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ocean_sampler.sample_value(&mut context, [0; 3]), Ok(3.0));
        assert_eq!(ocean_coords[1].scalar_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ocean_coords[2].scalar_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ocean_coords[3].scalar_calls.load(Ordering::Relaxed), 0);

        // Case 2: Interior interval (coord 0 == 1.0 -> left=coord 2, right=coord 1) verifies
        // exact lazy evaluation order [0, 2, 1] and leaves coord 3 untouched.
        log.lock().unwrap().clear();
        let mid_coords = [
            wrap(0, Arc::new(ConstantSampler(1.0))),
            wrap(1, Arc::new(ConstantSampler(0.25))),
            wrap(2, Arc::new(ConstantSampler(-0.5))),
            wrap(3, Arc::new(ConstantSampler(0.0))),
        ];
        let mid_inputs: Vec<Arc<dyn DensitySampler>> = mid_coords
            .iter()
            .map(|c| Arc::clone(c) as Arc<dyn DensitySampler>)
            .collect();
        let mid_sampler = SplineSampler::new(&root, &mid_inputs).unwrap();
        mid_sampler
            .sample_volume(&mut context, &mut output, volume)
            .unwrap();
        assert_eq!(*log.lock().unwrap(), vec![0, 2, 1]);
        assert_eq!(mid_coords[3].bulk_calls.load(Ordering::Relaxed), 0);

        // Warm scratch reuse: repeated bulk calls reuse the same retained Vec allocations.
        let all_coords = [
            wrap(
                0,
                Arc::new(GradientSampler::new(
                    Axis::X,
                    TilingMode::ClampToEdge,
                    0,
                    3,
                    -1.0,
                    3.0,
                )),
            ),
            wrap(1, Arc::new(ConstantSampler(0.25))),
            wrap(2, Arc::new(ConstantSampler(-0.5))),
            wrap(3, Arc::new(ConstantSampler(0.75))),
        ];
        let all_inputs: Vec<Arc<dyn DensitySampler>> = all_coords
            .iter()
            .map(|c| Arc::clone(c) as Arc<dyn DensitySampler>)
            .collect();
        let all_sampler = SplineSampler::new(&root, &all_inputs).unwrap();
        all_sampler
            .sample_volume(&mut context, &mut output, volume)
            .unwrap();
        let retained_allocs: Vec<(usize, usize)> = context
            .scratch
            .iter()
            .map(|buf| (buf.as_ptr() as usize, buf.capacity()))
            .collect();
        assert_eq!(retained_allocs.len(), 4);
        all_sampler
            .sample_volume(&mut context, &mut output, volume)
            .unwrap();
        let after_allocs: Vec<(usize, usize)> = context
            .scratch
            .iter()
            .map(|buf| (buf.as_ptr() as usize, buf.capacity()))
            .collect();
        assert_eq!(after_allocs.len(), retained_allocs.len());
        for entry in &after_allocs {
            assert!(
                retained_allocs.contains(entry),
                "warm scratch buffer was reallocated: {entry:?}"
            );
        }
    }

    #[test]
    fn sample_needed_evaluates_only_masked_cells_and_forwards_mask_to_coordinates() {
        #[derive(Debug)]
        struct Spoiled {
            inner: NoiseSampler,
            spoiled: AtomicUsize,
            needed_calls: AtomicUsize,
            volume_calls: AtomicUsize,
        }
        impl DensitySampler for Spoiled {
            fn sample_value(
                &self,
                context: &mut SamplerContext,
                position: [i32; 3],
            ) -> Result<f32, SampleError> {
                self.inner.sample_value(context, position)
            }
            fn sample_volume(
                &self,
                context: &mut SamplerContext,
                buffer: &mut [f32],
                volume: DensityVolume,
            ) -> Result<(), SampleError> {
                self.volume_calls.fetch_add(1, Ordering::Relaxed);
                self.inner.sample_volume(context, buffer, volume)
            }
            fn sample_needed(
                &self,
                context: &mut SamplerContext,
                buffer: &mut [f32],
                volume: DensityVolume,
                needed: &[u64],
            ) -> Result<(), SampleError> {
                self.needed_calls.fetch_add(1, Ordering::Relaxed);
                self.inner.sample_volume(context, buffer, volume)?;
                let size_y = volume.sizes()[1];
                for (index, value) in buffer[..volume.size()].iter_mut().enumerate() {
                    if (needed[index / size_y] >> (index % size_y)) & 1 == 0 {
                        *value = f32::NAN;
                        self.spoiled.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
        }

        let root = build_depth_4_overworld_style_spline();
        let spoiled_coords: [Arc<Spoiled>; 4] = std::array::from_fn(|i| {
            Arc::new(Spoiled {
                inner: NoiseSampler {
                    noise: Arc::new(PerlinNoise::new(&mut LegacyRandom::new(200 + i as i64))),
                    xz_scale: 0.08 + 0.02 * i as f64,
                    y_scale: 0.11,
                },
                spoiled: AtomicUsize::new(0),
                needed_calls: AtomicUsize::new(0),
                volume_calls: AtomicUsize::new(0),
            })
        });
        let inputs: Vec<Arc<dyn DensitySampler>> = spoiled_coords
            .iter()
            .map(|s| Arc::clone(s) as Arc<dyn DensitySampler>)
            .collect();
        let sampler = SplineSampler::new(&root, &inputs).unwrap();
        let mut context = SamplerContext::default();

        // Test regular masked volume, exact `size_y == 64` boundary, high-bit mask clamping,
        // and extra trailing mask entries.
        for (sizes, minimum, steps, extra_mask) in [
            ([4, 49, 4], [-8, -64, 4], [4, 8, 4], 0_usize),
            ([3, 64, 2], [32, -64, -16], [4, 6, 4], 3_usize),
            ([2, 5, 2], [0, 0, 0], [1, 1, 1], 2_usize),
        ] {
            let volume = DensityVolume::new(sizes, minimum, steps).unwrap();
            let [sx, sy, sz] = volume.sizes();
            let columns = sx * sz;
            let mut full = vec![0.0; volume.size()];
            sampler
                .sample_volume(&mut context, &mut full, volume)
                .unwrap();

            let mut mask: Vec<u64> = (0..columns + extra_mask)
                .map(|col| match col % 4 {
                    0 => 0,
                    1 => 1_u64 | (1_u64 << (sy - 1)),
                    2 => 0x5555_5555_5555_5555_u64,
                    _ => u64::MAX,
                })
                .collect();
            mask[0] = 0;

            for coord in &spoiled_coords {
                coord.spoiled.store(0, Ordering::Relaxed);
                coord.needed_calls.store(0, Ordering::Relaxed);
                coord.volume_calls.store(0, Ordering::Relaxed);
            }

            let mut masked = vec![19.0; volume.size()];
            sampler
                .sample_needed(&mut context, &mut masked, volume, &mask)
                .unwrap();

            let valid_y_mask = u64::MAX >> (64 - sy);
            for (col, &col_mask) in mask[..columns].iter().enumerate() {
                let wanted = col_mask & valid_y_mask;
                for y in 0..sy {
                    let index = col * sy + y;
                    if (wanted >> y) & 1 == 1 {
                        assert_f32_exact(
                            masked[index],
                            full[index],
                            &format!("sample_needed {sizes:?} col={col} y={y}"),
                        );
                    } else {
                        assert_eq!(
                            masked[index].to_bits(),
                            19.0_f32.to_bits(),
                            "unneeded cell overwritten at {sizes:?} col={col} y={y}"
                        );
                    }
                }
            }
            assert_eq!(spoiled_coords[0].needed_calls.load(Ordering::Relaxed), 1);
            assert_eq!(spoiled_coords[0].volume_calls.load(Ordering::Relaxed), 0);
            assert!(spoiled_coords[0].spoiled.load(Ordering::Relaxed) > 0);
        }

        // All-zero mask: evaluates zero coordinates and writes zero output cells.
        let volume = DensityVolume::new([3, 8, 3], [0; 3], [1; 3]).unwrap();
        for coord in &spoiled_coords {
            coord.needed_calls.store(0, Ordering::Relaxed);
            coord.volume_calls.store(0, Ordering::Relaxed);
        }
        let mut untouched = vec![19.0; volume.size()];
        sampler
            .sample_needed(&mut context, &mut untouched, volume, &[0; 9])
            .unwrap();
        assert!(untouched.iter().all(|v| v.to_bits() == 19.0_f32.to_bits()));
        for coord in &spoiled_coords {
            assert_eq!(coord.needed_calls.load(Ordering::Relaxed), 0);
            assert_eq!(coord.volume_calls.load(Ordering::Relaxed), 0);
        }

        // Unusable masks (`size_y == 65` or short mask slice) fall back to full `sample_volume`.
        let tall = DensityVolume::new([2, 65, 2], [0; 3], [1; 3]).unwrap();
        let mut tall_full = vec![0.0; tall.size()];
        let mut tall_needed = vec![19.0; tall.size()];
        sampler
            .sample_volume(&mut context, &mut tall_full, tall)
            .unwrap();
        for coord in &spoiled_coords {
            coord.needed_calls.store(0, Ordering::Relaxed);
            coord.volume_calls.store(0, Ordering::Relaxed);
        }
        sampler
            .sample_needed(&mut context, &mut tall_needed, tall, &[1; 4])
            .unwrap();
        assert_eq!(
            tall_needed
                .iter()
                .copied()
                .map(f32::to_bits)
                .collect::<Vec<_>>(),
            tall_full
                .iter()
                .copied()
                .map(f32::to_bits)
                .collect::<Vec<_>>()
        );
        assert_eq!(spoiled_coords[0].needed_calls.load(Ordering::Relaxed), 0);
        assert_eq!(spoiled_coords[0].volume_calls.load(Ordering::Relaxed), 1);

        let short_mask_vol = DensityVolume::new([2, 4, 2], [0; 3], [1; 3]).unwrap();
        let mut short_full = vec![0.0; short_mask_vol.size()];
        let mut short_needed = vec![19.0; short_mask_vol.size()];
        sampler
            .sample_volume(&mut context, &mut short_full, short_mask_vol)
            .unwrap();
        sampler
            .sample_needed(&mut context, &mut short_needed, short_mask_vol, &[0; 3])
            .unwrap();
        assert_eq!(
            short_needed
                .iter()
                .copied()
                .map(f32::to_bits)
                .collect::<Vec<_>>(),
            short_full
                .iter()
                .copied()
                .map(f32::to_bits)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn sample_needed_skips_lazy_coordinates_only_referenced_in_unneeded_cells_and_handles_errors() {
        let fail_branch = CubicSpline::multipoint(1, vec![cpt(0.0, 0.0, 11.0)]).unwrap();
        let root = CubicSpline::multipoint(0, vec![cpt(-1.0, 0.0, 7.0), pt(1.0, 0.0, fail_branch)])
            .unwrap();
        let inputs: Vec<Arc<dyn DensitySampler>> = vec![
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::ClampToEdge,
                0,
                1,
                -2.0,
                0.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::Repeat,
                0,
                0,
                0.0,
                1.0,
            )),
        ];
        let sampler = SplineSampler::new(&root, &inputs).unwrap();
        let volume = DensityVolume::new([2, 2, 1], [0; 3], [1; 3]).unwrap();

        // Only column 0 (x = 0, coord0 == -2.0 -> Constant(7.0)) is needed: failing coord 1 is skipped.
        let mut context = SamplerContext::default();
        let mut output = [19.0; 4];
        sampler
            .sample_needed(&mut context, &mut output, volume, &[0b01, 0])
            .unwrap();
        assert_eq!(output, [7.0, 19.0, 19.0, 19.0]);
        assert_eq!(context.scratch.len(), 1);

        // When column 1 IS needed, completed column 0 cell is preserved and both scratch buffers return.
        context.clear_scratch();
        let mut output_err = [19.0; 4];
        assert_eq!(
            sampler.sample_needed(&mut context, &mut output_err, volume, &[0b10, 0b01]),
            Err(SampleError::ZeroGradientPeriod)
        );
        assert_eq!(output_err, [19.0, 7.0, 19.0, 19.0]);
        assert_eq!(context.scratch.len(), 2);

        // Buffer length errors reject before any child evaluation.
        let mut too_short = [19.0; 3];
        assert_eq!(
            sampler.sample_needed(&mut context, &mut too_short, volume, &[0b11, 0b11]),
            Err(SampleError::Volume(DensityError::BufferTooShort))
        );
        let mut too_long = [19.0; 5];
        assert_eq!(
            sampler.sample_needed(&mut context, &mut too_long, volume, &[0b11, 0b11]),
            Err(SampleError::ExactBufferSizeRequired)
        );
    }

    #[test]
    fn spline_sampler_preserves_range_and_public_api_contracts() {
        let spline = build_depth_4_overworld_style_spline();
        let bounds = spline
            .range(&mut |_| Interval::new(-1.5, 1.5).unwrap())
            .unwrap();
        assert!(bounds.min().is_finite() && bounds.max().is_finite());
        assert!(bounds.min() <= bounds.max());

        let inputs: Vec<Arc<dyn DensitySampler>> = (0..4)
            .map(|i| Arc::new(ConstantSampler(i as f32 * 0.25)) as Arc<dyn DensitySampler>)
            .collect();
        let sampler = SplineSampler::new(&spline, &inputs).unwrap();
        let cloned = sampler.clone();
        assert_eq!(cloned.coordinate_count(), 4);
        assert!(!format!("{cloned:?}").is_empty());
        let pointwise = cloned.pointwise_inputs().unwrap();
        assert_eq!(pointwise.len(), 4);
        for (actual, expected) in pointwise.into_iter().zip(&inputs) {
            assert!(Arc::ptr_eq(actual, expected));
        }
    }
}
