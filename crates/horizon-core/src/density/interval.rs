//! Selected vanilla interval operations used for density graph range analysis.
use super::arithmetic::{java_max, java_min};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntervalError {
    NanBound,
    InvertedBounds,
    EmptyInput,
    NanMappedBound,
}
impl std::fmt::Display for IntervalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NanBound => "interval endpoint is NaN",
            Self::InvertedBounds => "interval endpoints are reversed",
            Self::EmptyInput => "at least one interval is required",
            Self::NanMappedBound => "monotonic range operator produced NaN",
        })
    }
}
impl std::error::Error for IntervalError {}

#[derive(Clone, Copy, Debug)]
pub struct Interval {
    min: f32,
    max: f32,
}
impl PartialEq for Interval {
    fn eq(&self, other: &Self) -> bool {
        (self.is_nai() && other.is_nai()) || (self.min == other.min && self.max == other.max)
    }
}
impl Eq for Interval {}
impl Interval {
    pub const NAI: Self = Self {
        min: f32::NAN,
        max: f32::NAN,
    };
    pub const INFINITE: Self = Self {
        min: f32::NEG_INFINITY,
        max: f32::INFINITY,
    };
    pub fn new(min: f32, max: f32) -> Result<Self, IntervalError> {
        if max < min {
            return Err(IntervalError::InvertedBounds);
        }
        if min.is_nan() || max.is_nan() {
            return Err(IntervalError::NanBound);
        }
        // The reference returns a cached [0, 1], canonicalizing a negative zero.
        Ok(Self {
            min: if min == 0.0 && max == 1.0 { 0.0 } else { min },
            max,
        })
    }
    pub fn symmetric(value: f32) -> Result<Self, IntervalError> {
        Self::new(-value, value)
    }
    pub fn exact(value: f32) -> Result<Self, IntervalError> {
        Self::new(value, value)
    }
    pub fn min(self) -> f32 {
        self.min
    }
    pub fn max(self) -> f32 {
        self.max
    }
    pub fn is_nai(self) -> bool {
        self.min.is_nan()
    }
    pub fn contains(self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }
    pub fn intersects(self, other: Self) -> bool {
        self.min <= other.max && self.max >= other.min
    }
    /// Kept separate from Rust's Hash contract: Java compares signed zeros equal
    /// here, but its endpoint-based hash can distinguish them.
    pub fn java_hash_code(self) -> i32 {
        let bits = |v: f32| {
            if v.is_nan() {
                0x7fc0_0000_i32
            } else {
                v.to_bits() as i32
            }
        };
        bits(self.min).wrapping_mul(31).wrapping_add(bits(self.max))
    }
    pub fn encapsulating_values(a: f32, b: f32) -> Self {
        if a.is_nan() && b.is_nan() {
            Self::NAI
        } else if a.is_nan() {
            Self::exact(b).unwrap()
        } else if b.is_nan() {
            Self::exact(a).unwrap()
        } else {
            Self::new(java_min(a, b), java_max(a, b)).unwrap()
        }
    }
    pub fn encapsulating(intervals: &[Self]) -> Result<Self, IntervalError> {
        if intervals.is_empty() {
            return Err(IntervalError::EmptyInput);
        }
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for interval in intervals {
            if !interval.is_nai() {
                min = java_min(interval.min, min);
                max = java_max(interval.max, max);
            }
        }
        if max < min {
            Ok(Self::NAI)
        } else {
            Self::new(min, max)
        }
    }
    fn arithmetic_bounds(min: f32, max: f32) -> Self {
        if min.is_nan() || max.is_nan() {
            Self::NAI
        } else {
            Self::new(min, max).expect("ordered interval arithmetic")
        }
    }
    pub fn add_range(self, right: Self) -> Self {
        Self::arithmetic_bounds(self.min + right.min, self.max + right.max)
    }
    pub fn subtract_range(self, right: Self) -> Self {
        Self::arithmetic_bounds(self.min - right.max, self.max - right.min)
    }
    pub fn multiply_range(self, right: Self) -> Self {
        if self.is_nai() || right.is_nai() {
            return Self::NAI;
        }
        let a = mul_bound(self.min, right.min);
        let b = mul_bound(self.min, right.max);
        let c = mul_bound(self.max, right.min);
        let d = mul_bound(self.max, right.max);
        Self::new(
            java_min(java_min(a, b), java_min(c, d)),
            java_max(java_max(a, b), java_max(c, d)),
        )
        .unwrap()
    }
    pub fn reciprocal(self) -> Self {
        if self.is_nai() || (self.min == 0.0 && self.max == 0.0) {
            Self::NAI
        } else if !self.contains(0.0) {
            Self::new(1.0 / self.max, 1.0 / self.min).unwrap()
        } else if self.max == 0.0 {
            Self::new(f32::NEG_INFINITY, 1.0 / self.min).unwrap()
        } else if self.min == 0.0 {
            Self::new(1.0 / self.max, f32::INFINITY).unwrap()
        } else {
            Self::INFINITE
        }
    }
    pub fn divide_range(self, right: Self) -> Self {
        self.multiply_range(right.reciprocal())
    }
    pub fn minimum(self, right: Self) -> Self {
        Self::arithmetic_bounds(java_min(self.min, right.min), java_min(self.max, right.max))
    }
    pub fn maximum(self, right: Self) -> Self {
        Self::arithmetic_bounds(java_max(self.min, right.min), java_max(self.max, right.max))
    }
    pub fn clamp(self, min: f32, max: f32) -> Result<Self, IntervalError> {
        if min > max {
            return Err(IntervalError::InvertedBounds);
        }
        if self.is_nai() {
            Ok(Self::NAI)
        } else if self.min >= max {
            Self::exact(max)
        } else if self.max <= min {
            Self::exact(min)
        } else {
            Self::new(java_max(self.min, min), java_min(self.max, max))
        }
    }
    pub fn abs(self) -> Self {
        self.even_bounds(f32::abs)
    }
    pub fn square(self) -> Self {
        self.even_bounds(|v| v * v)
    }
    pub fn log(self) -> Result<Self, IntervalError> {
        if self.is_nai() || self.max < 0.0 {
            Ok(Self::NAI)
        } else {
            self.maximum(Self::exact(0.0)?)
                .map_monotonic(|v| crate::jmath::log(v as f64) as f32)
        }
    }
    pub fn power(self, exponent: Self) -> Self {
        if self.is_nai() || exponent.is_nai() {
            return Self::NAI;
        }
        if self.min == self.max {
            return Self::power_base(self.min, exponent);
        }
        let mut result = Self::encapsulating(&[
            Self::power_base(self.min, exponent),
            Self::power_base(self.max, exponent),
        ])
        .unwrap();
        if self.contains(0.0) {
            if self.max > 0.0 {
                result = Self::encapsulating(&[result, Self::power_base(0.0, exponent)]).unwrap();
            }
            if self.min < 0.0 {
                result = Self::encapsulating(&[result, Self::power_base(-0.0, exponent)]).unwrap();
            }
        }
        result
    }
    fn including_value(self, value: f32) -> Self {
        if value.is_nan() {
            self
        } else if self.is_nai() {
            Self::exact(value).unwrap()
        } else {
            Self::new(java_min(self.min, value), java_max(self.max, value)).unwrap()
        }
    }
    fn power_base(base: f32, exponent: Self) -> Self {
        if base.is_nan() || exponent.is_nai() {
            return Self::NAI;
        }
        if exponent.min == exponent.max {
            let value = java_power(base, exponent.min);
            return Self::encapsulating_values(value, value);
        }
        if base == 0.0 {
            let range = if exponent.contains(0.0) {
                if exponent.max == 0.0 {
                    Self::new(1.0, f32::INFINITY).unwrap()
                } else if exponent.min == 0.0 {
                    Self::new(0.0, 1.0).unwrap()
                } else {
                    Self::new(0.0, f32::INFINITY).unwrap()
                }
            } else if exponent.max < 0.0 {
                Self::exact(f32::INFINITY).unwrap()
            } else {
                Self::exact(0.0).unwrap()
            };
            return range.multiply_range(Self::exact(1.0_f32.copysign(base)).unwrap());
        }
        if base == 1.0 {
            return Self::exact(1.0).unwrap();
        }
        if base > 0.0 {
            if exponent.min.is_finite() && exponent.max.is_finite() {
                return Self::encapsulating_values(
                    java_power(base, exponent.min),
                    java_power(base, exponent.max),
                );
            }
            if exponent.min.is_infinite() && exponent.max.is_infinite() {
                return Self::new(0.0, f32::INFINITY).unwrap();
            }
            let (min, max) = if exponent.min.is_infinite() {
                if base < 1.0 {
                    (java_power(base, exponent.max), f32::INFINITY)
                } else {
                    (0.0, java_power(base, exponent.max))
                }
            } else if base < 1.0 {
                (0.0, java_power(base, exponent.min))
            } else {
                (java_power(base, exponent.min), f32::INFINITY)
            };
            return Self::new(min, max).unwrap();
        }
        let first = (exponent.min as f64).ceil() as f32;
        let last = (exponent.max as f64).floor() as f32;
        if last < first {
            return Self::NAI;
        }
        let a = java_power(base, first);
        let b = java_power(base, last);
        let mut result = Self::encapsulating_values(a, b);
        if first.is_infinite() {
            result = result.including_value(-a);
        } else if first + 1.0 < last {
            result = result.including_value(java_power(base, first + 1.0));
        }
        if last.is_infinite() {
            result = result.including_value(-b);
        } else if last - 1.0 > first {
            result = result.including_value(java_power(base, last - 1.0));
        }
        result
    }
    fn even_bounds(self, transform: impl Fn(f32) -> f32) -> Self {
        if self.is_nai() {
            return self;
        }
        let a = transform(self.min);
        let b = transform(self.max);
        Self::new(
            if self.contains(0.0) {
                0.0
            } else {
                java_min(a, b)
            },
            java_max(a, b),
        )
        .unwrap()
    }
    /// The caller supplies a monotonic transformation. As in vanilla, this maps
    /// only the endpoints; it does not attempt to prove monotonicity.
    pub fn map_monotonic(
        self,
        mut transform: impl FnMut(f32) -> f32,
    ) -> Result<Self, IntervalError> {
        if self.is_nai() {
            return Ok(self);
        }
        let a = transform(self.min);
        let b = transform(self.max);
        if a.is_nan() || b.is_nan() {
            Err(IntervalError::NanMappedBound)
        } else {
            Self::new(java_min(a, b), java_max(a, b))
        }
    }
    pub fn lerp_ranges(self, first: Self, second: Self) -> Self {
        if self.is_nai() || first.is_nai() || second.is_nai() {
            return Self::NAI;
        }
        Self::encapsulating(&[
            self.lerp_values(first.min, second.min),
            self.lerp_values(first.max, second.min),
            self.lerp_values(first.min, second.max),
            self.lerp_values(first.max, second.max),
        ])
        .unwrap()
    }
    pub fn lerp_values(self, first: f32, second: f32) -> Self {
        if self.is_nai() || first.is_nan() || second.is_nan() {
            return Self::NAI;
        }
        if first.is_finite() && second.is_finite() {
            return Self::encapsulating_values(
                first + mul_bound(self.min, second - first),
                first + mul_bound(self.max, second - first),
            );
        }
        if first == second {
            return Self::exact(first).unwrap();
        }
        let a = lerp_infinite_bound(self.min, first, second);
        let b = lerp_infinite_bound(self.max, first, second);
        if a.is_nan() || b.is_nan() {
            Self::NAI
        } else {
            Self::encapsulating_values(a, b)
        }
    }
    pub fn sign(self) -> Self {
        if self.is_nai() {
            self
        } else if self.min == self.max {
            Self::exact(if self.min == 0.0 {
                self.min
            } else {
                self.min.signum()
            })
            .unwrap()
        } else if self.contains(0.0) {
            Self::new(
                if self.min == 0.0 { 0.0 } else { -1.0 },
                if self.max == 0.0 { 0.0 } else { 1.0 },
            )
            .unwrap()
        } else {
            Self::exact(if self.min > 0.0 { 1.0 } else { -1.0 }).unwrap()
        }
    }
}
fn java_power(base: f32, exponent: f32) -> f32 {
    super::arithmetic::java_power(base as f64, exponent as f64) as f32
}
fn mul_bound(a: f32, b: f32) -> f32 {
    if a == 0.0 || b == 0.0 { 0.0 } else { a * b }
}

fn lerp_infinite_bound(alpha: f32, first: f32, second: f32) -> f32 {
    let a = mul_bound(1.0 - alpha, first);
    let b = mul_bound(alpha, second);
    if a.is_infinite() && b.is_infinite() {
        if alpha <= 0.0 {
            if second > first {
                f32::NEG_INFINITY
            } else {
                f32::INFINITY
            }
        } else if alpha >= 1.0 {
            if second > first {
                f32::INFINITY
            } else {
                f32::NEG_INFINITY
            }
        } else {
            f32::NAN
        }
    } else {
        a + b
    }
}
