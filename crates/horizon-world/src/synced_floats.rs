//! Multiple synchronized float fields with native callback and dirty ordering.
//! Other serializer types and wire codecs are not represented by this module.
use crate::compression::invalid;
use std::{fmt, io};
#[derive(Clone, Copy, Debug)]
pub struct FloatValue {
    pub id: u8,
    pub value: f32,
}
#[derive(Clone, Copy, Debug)]
struct Entry {
    id: u8,
    initial: f32,
    value: f32,
    dirty: bool,
}
#[derive(Clone, Debug)]
pub struct SyncedFloats {
    entries: Vec<Entry>,
    dirty: bool,
}
#[derive(Debug)]
pub enum DataError<E> {
    UnknownId(u8),
    Observer(E),
}
impl<E: fmt::Display> fmt::Display for DataError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId(id) => write!(f, "unknown synchronized float ID {id}"),
            Self::Observer(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for DataError<E> {}
pub trait Observer {
    type Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> Result<(), Self::Error>;
    fn assigned(
        &mut self,
        _data: &mut SyncedFloats,
        _values: &[FloatValue],
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
fn equal(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}
impl SyncedFloats {
    pub fn new(values: &[FloatValue]) -> io::Result<Self> {
        let mut entries = values
            .iter()
            .map(|v| Entry {
                id: v.id,
                initial: v.value,
                value: v.value,
                dirty: false,
            })
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|e| e.id);
        if entries.iter().any(|e| e.id == 255) || entries.windows(2).any(|e| e[0].id == e[1].id) {
            return Err(invalid("duplicate or reserved synchronized float ID"));
        }
        Ok(Self {
            entries,
            dirty: false,
        })
    }
    fn index(&self, id: u8) -> Result<usize, u8> {
        self.entries
            .binary_search_by_key(&id, |e| e.id)
            .map_err(|_| id)
    }
    pub fn get(&self, id: u8) -> Result<f32, u8> {
        Ok(self.entries[self.index(id)?].value)
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn item_dirty(&self, id: u8) -> Result<bool, u8> {
        Ok(self.entries[self.index(id)?].dirty)
    }
    pub fn is_default(&self, id: u8) -> Result<bool, u8> {
        let e = &self.entries[self.index(id)?];
        Ok(equal(e.initial, e.value))
    }
    pub fn set<O: Observer>(
        &mut self,
        id: u8,
        value: f32,
        force: bool,
        observer: &mut O,
    ) -> Result<(), DataError<O::Error>> {
        let index = self.index(id).map_err(DataError::UnknownId)?;
        if force || !equal(value, self.entries[index].value) {
            self.entries[index].value = value;
            observer.updated(self, id).map_err(DataError::Observer)?;
            self.entries[index].dirty = true;
            self.dirty = true;
        }
        Ok(())
    }
    /// Received values always notify, even when equal, and do not set/clear dirty
    /// flags. A failure preserves earlier entries and skips later/batch callbacks.
    pub fn assign<O: Observer>(
        &mut self,
        values: &[FloatValue],
        observer: &mut O,
    ) -> Result<(), DataError<O::Error>> {
        for v in values {
            let index = self.index(v.id).map_err(DataError::UnknownId)?;
            self.entries[index].value = v.value;
            observer.updated(self, v.id).map_err(DataError::Observer)?;
        }
        observer.assigned(self, values).map_err(DataError::Observer)
    }
    pub fn take_dirty(&mut self) -> Option<Vec<FloatValue>> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let mut values = Vec::new();
        for e in &mut self.entries {
            if e.dirty {
                e.dirty = false;
                values.push(FloatValue {
                    id: e.id,
                    value: e.value,
                });
            }
        }
        Some(values)
    }
    pub fn non_default(&self) -> Option<Vec<FloatValue>> {
        let values: Vec<_> = self
            .entries
            .iter()
            .filter(|e| !equal(e.initial, e.value))
            .map(|e| FloatValue {
                id: e.id,
                value: e.value,
            })
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values)
        }
    }
}
