//! Bundle weight arithmetic and lazy caching. The checked integer operations
//! retain the pinned Fraction behavior, including intermediate overflow errors.
use std::{cell::OnceCell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fraction {
    numerator: i32,
    denominator: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExcessiveWeight;
fn gcd(a: i32, b: i32) -> i32 {
    let (mut a, mut b) = (i64::from(a).abs(), i64::from(b).abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    // Every call has a positive, representable denominator as one operand.
    a as i32
}
impl Fraction {
    pub fn numerator(self) -> i32 {
        self.numerator
    }
    pub fn denominator(self) -> i32 {
        self.denominator
    }
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    pub fn new(mut numerator: i32, mut denominator: i32) -> Result<Self, ExcessiveWeight> {
        if denominator == 0 {
            return Err(ExcessiveWeight);
        }
        if denominator < 0 {
            numerator = numerator.checked_neg().ok_or(ExcessiveWeight)?;
            denominator = denominator.checked_neg().ok_or(ExcessiveWeight)?;
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
    pub fn checked_add(self, other: Self) -> Result<Self, ExcessiveWeight> {
        if self.numerator == 0 {
            return Ok(other);
        }
        if other.numerator == 0 {
            return Ok(self);
        }
        let d1 = gcd(self.denominator, other.denominator);
        if d1 == 1 {
            let a = self
                .numerator
                .checked_mul(other.denominator)
                .ok_or(ExcessiveWeight)?;
            let b = other
                .numerator
                .checked_mul(self.denominator)
                .ok_or(ExcessiveWeight)?;
            Ok(Self {
                numerator: a.checked_add(b).ok_or(ExcessiveWeight)?,
                denominator: self
                    .denominator
                    .checked_mul(other.denominator)
                    .ok_or(ExcessiveWeight)?,
            })
        } else {
            // Native uses BigInteger here. Products and their sum fit i64 for
            // i32 numerators and positive i32 denominators.
            let t = i64::from(self.numerator) * i64::from(other.denominator / d1)
                + i64::from(other.numerator) * i64::from(self.denominator / d1);
            let residue = t.rem_euclid(i64::from(d1)) as i32;
            let d2 = if residue == 0 { d1 } else { gcd(residue, d1) };
            Ok(Self {
                numerator: i32::try_from(t / i64::from(d2)).map_err(|_| ExcessiveWeight)?,
                denominator: (self.denominator / d1)
                    .checked_mul(other.denominator / d2)
                    .ok_or(ExcessiveWeight)?,
            })
        }
    }
    pub fn checked_multiply(self, other: Self) -> Result<Self, ExcessiveWeight> {
        if self.numerator == 0 || other.numerator == 0 {
            return Ok(Self::ZERO);
        }
        let d1 = gcd(self.numerator, other.denominator);
        let d2 = gcd(other.numerator, self.denominator);
        let numerator = (self.numerator / d1)
            .checked_mul(other.numerator / d2)
            .ok_or(ExcessiveWeight)?;
        let denominator = (self.denominator / d2)
            .checked_mul(other.denominator / d1)
            .ok_or(ExcessiveWeight)?;
        let d = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / d,
            denominator: denominator / d,
        })
    }
}
#[derive(Debug)]
struct Data<T> {
    items: Vec<T>,
    weight: OnceCell<Result<Fraction, ExcessiveWeight>>,
}
#[derive(Debug)]
pub struct BundleWeight<T>(Rc<Data<T>>);
impl<T> Clone for BundleWeight<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
pub trait ItemWeights<T> {
    fn count(&self, item: &T) -> i32;
    fn max_stack_size(&self, item: &T) -> i32;
    fn has_bees(&self, item: &T) -> bool;
    fn bundle(&self, item: &T) -> Option<BundleWeight<T>>;
}
impl<T> BundleWeight<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self(Rc::new(Data {
            items,
            weight: OnceCell::new(),
        }))
    }
    pub fn items(&self) -> &[T] {
        &self.0.items
    }
    pub fn weight(&self, view: &impl ItemWeights<T>) -> Result<Fraction, ExcessiveWeight> {
        *self.0.weight.get_or_init(|| self.compute(view))
    }
    fn compute(&self, view: &impl ItemWeights<T>) -> Result<Fraction, ExcessiveWeight> {
        let mut total = Fraction::ZERO;
        for item in self.items() {
            let weight = if let Some(bundle) = view.bundle(item) {
                bundle.weight(view)?.checked_add(Fraction {
                    numerator: 1,
                    denominator: 16,
                })?
            } else if view.has_bees(item) {
                Fraction::ONE
            } else {
                Fraction::new(1, view.max_stack_size(item))?
            };
            let count = Fraction::new(view.count(item), 1)?;
            total = total.checked_add(weight.checked_multiply(count)?)?;
        }
        Ok(total)
    }
}

/// Contained stack sizes are checked without recursively validating their
/// components. Bundle weight is evaluated only after all its item sizes pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentsError {
    Size { count: i32, maximum: i32 },
    Weight(ExcessiveWeight),
}
pub fn validate_sizes<T>(items: &[T], view: &impl ItemWeights<T>) -> Result<(), ContentsError> {
    for item in items {
        let count = view.count(item);
        let maximum = view.max_stack_size(item);
        if count > maximum {
            return Err(ContentsError::Size { count, maximum });
        }
    }
    Ok(())
}
pub fn validate_contents<T>(
    container: Option<&[T]>,
    bundle: Option<&BundleWeight<T>>,
    projectiles: Option<&[T]>,
    view: &impl ItemWeights<T>,
) -> Result<(), ContentsError> {
    if let Some(items) = container {
        validate_sizes(items, view)?;
    }
    if let Some(bundle) = bundle {
        validate_sizes(bundle.items(), view)?;
        bundle.weight(view).map_err(ContentsError::Weight)?;
    }
    if let Some(items) = projectiles {
        validate_sizes(items, view)?;
    }
    Ok(())
}
