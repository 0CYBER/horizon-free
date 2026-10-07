//! Synchronized typed values with native callback/dirty ordering.
//! Serializer codecs and complete entity schemas are separate.
use crate::compression::invalid;
use std::{fmt, io};
#[derive(Clone, Copy, Debug)]
pub struct DataValue<T> {
    pub id: u8,
    pub value: T,
}
#[derive(Clone, Copy, Debug)]
struct Entry<T> {
    id: u8,
    initial: T,
    value: T,
    dirty: bool,
}
#[derive(Clone, Debug)]
pub struct SyncedValues<T> {
    entries: Vec<Entry<T>>,
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
            Self::UnknownId(id) => write!(f, "unknown synchronized field ID {id}"),
            Self::Observer(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for DataError<E> {}
pub trait Observer<T: SyncedValue = f32> {
    type Error;
    fn updated(&mut self, data: &mut SyncedValues<T>, id: u8) -> Result<(), Self::Error>;
    fn assigned(
        &mut self,
        _data: &mut SyncedValues<T>,
        _values: &[DataValue<T>],
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
/// Equality used by the corresponding boxed native serializer value.
pub trait SyncedValue: Clone {
    fn native_equal(&self, other: &Self) -> bool;
}
impl SyncedValue for f32 {
    fn native_equal(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits() || (self.is_nan() && other.is_nan())
    }
}
impl SyncedValue for u8 {
    fn native_equal(&self, other: &Self) -> bool {
        self == other
    }
}
impl SyncedValue for bool {
    fn native_equal(&self, other: &Self) -> bool {
        self == other
    }
}
impl<T: SyncedValue> SyncedValues<T> {
    pub fn new(values: &[DataValue<T>]) -> io::Result<Self> {
        let mut entries = values
            .iter()
            .map(|v| Entry {
                id: v.id,
                initial: v.value.clone(),
                value: v.value.clone(),
                dirty: false,
            })
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|e| e.id);
        if entries.iter().any(|e| e.id == 255) || entries.windows(2).any(|e| e[0].id == e[1].id) {
            return Err(invalid("duplicate or reserved synchronized field ID"));
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
    pub fn get(&self, id: u8) -> Result<T, u8> {
        Ok(self.entries[self.index(id)?].value.clone())
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn item_dirty(&self, id: u8) -> Result<bool, u8> {
        Ok(self.entries[self.index(id)?].dirty)
    }
    pub fn is_default(&self, id: u8) -> Result<bool, u8> {
        let e = &self.entries[self.index(id)?];
        Ok(e.initial.native_equal(&e.value))
    }
    pub fn set<O: Observer<T>>(
        &mut self,
        id: u8,
        value: T,
        force: bool,
        observer: &mut O,
    ) -> Result<(), DataError<O::Error>> {
        let index = self.index(id).map_err(DataError::UnknownId)?;
        if force || !value.native_equal(&self.entries[index].value) {
            self.entries[index].value = value;
            observer.updated(self, id).map_err(DataError::Observer)?;
            self.entries[index].dirty = true;
            self.dirty = true;
        }
        Ok(())
    }
    /// Received values always notify, even when equal, and do not set/clear dirty
    /// flags. A failure preserves earlier entries and skips later/batch callbacks.
    pub fn assign<O: Observer<T>>(
        &mut self,
        values: &[DataValue<T>],
        observer: &mut O,
    ) -> Result<(), DataError<O::Error>> {
        for v in values {
            let index = self.index(v.id).map_err(DataError::UnknownId)?;
            self.entries[index].value = v.value.clone();
            observer.updated(self, v.id).map_err(DataError::Observer)?;
        }
        observer.assigned(self, values).map_err(DataError::Observer)
    }
    pub fn take_dirty(&mut self) -> Option<Vec<DataValue<T>>> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let mut values = Vec::new();
        for e in &mut self.entries {
            if e.dirty {
                e.dirty = false;
                values.push(DataValue {
                    id: e.id,
                    value: e.value.clone(),
                });
            }
        }
        Some(values)
    }
    pub fn non_default(&self) -> Option<Vec<DataValue<T>>> {
        let values: Vec<_> = self
            .entries
            .iter()
            .filter(|e| !e.initial.native_equal(&e.value))
            .map(|e| DataValue {
                id: e.id,
                value: e.value.clone(),
            })
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values)
        }
    }
}
