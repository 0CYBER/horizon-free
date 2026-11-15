//! Typed mod payloads: the pure value codec.
//! A payload's value type says how it is written to and read from a payload
//! body in vanilla's network encoding (big-endian numbers, VarInt lengths),
//! through a writer and a reader that enforce the codec's bounds. A decoder
//! reads untrusted bytes: every length, count, depth and allocation is
//! checked before anything is reserved or copied, and nothing here panics on
//! any input. The codec touches no `Host` and no world, so it runs the same
//! in a native mod and in a wasm guest. Registration and delivery are separate from encoding.
use super::payload_budget::Budget;
// The helpers a derived codec's schema calls, beside the codec it names as its crate.
pub use super::payload_schema::{payload_schema_leaf, payload_schema_mix, payload_schema_named};

/// The deepest a payload value nests: each box, each list, each present option and
/// each [`PayloadReader::nested`] level counts one.
pub const PAYLOAD_MAX_DEPTH: u16 = 64;
/// The most UTF-8 bytes of a string read with [`PayloadReader::string`].
pub const PAYLOAD_MAX_STRING_BYTES: usize = 4096;
/// The most entries of a list read through [`PayloadCodec`] for `Vec<T>`.
pub const PAYLOAD_MAX_LIST_ENTRIES: usize = 4096;
/// The most bytes one payload value may make its decoder allocate: lists
/// reserve against it by their decoded size, owned strings by their length, and
/// boxes by the size of their inner value, each size as a 64-bit target has it. The
/// budget equals the largest uncompressed packet; structured values can require more
/// allocation than their encoded body and are refused when they exhaust it. A writer
/// charges the same budget, so it refuses what the peer's reader would.
pub const PAYLOAD_DECODE_BUDGET: usize = 8_388_608;

/// Why a payload body is not a value of its type. The value is dropped and
/// its handler does not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum PayloadDecodeError {
    /// The body ends inside a value.
    UnexpectedEnd,
    /// A VarInt of more than 5 bytes, or a VarLong of more than 10.
    VarIntTooLong,
    /// A length or count below zero.
    NegativeLength {
        /// The length as read.
        length: i32,
    },
    /// A length or count past its bound.
    LengthOverLimit {
        /// The length as read.
        length: u32,
        /// The most the codec accepts there.
        limit: u32,
    },
    /// A string that is not UTF-8.
    Utf8,
    /// Values nested deeper than [`PAYLOAD_MAX_DEPTH`].
    TooDeep,
    /// A boolean, option or enum tag its type does not have.
    InvalidDiscriminant {
        /// The tag as read.
        value: i32,
    },
    /// A number that must be finite is NaN or infinite.
    NonFinite,
    /// The value would allocate past the reader's budget.
    BudgetExhausted,
    /// Bytes left over after the value.
    Trailing {
        /// How many bytes follow the value.
        bytes: u64,
    },
    /// A value its own codec refuses (a number outside its domain, an
    /// unknown id).
    InvalidValue,
}

/// Why a value was not encoded. Nothing is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum PayloadEncodeError {
    /// The encoded body would pass the writer's byte limit.
    TooLarge {
        /// The writer's limit.
        limit: u64,
    },
    /// A string, byte array or list longer than its bound, so the peer
    /// would refuse it.
    LengthOverLimit {
        /// The length of the value.
        length: u64,
        /// The most the codec accepts there.
        limit: u32,
    },
    /// Values nested deeper than [`PAYLOAD_MAX_DEPTH`].
    TooDeep,
    /// A number that must be finite is NaN or infinite.
    NonFinite,
    /// The value would make the peer's decoder allocate past [`PAYLOAD_DECODE_BUDGET`].
    BudgetExhausted,
}

/// A value with a payload form: the pure codec of a typed payload. Both
/// functions see only the buffer, never a `Host`; `decode` must leave the
/// reader just past the value and must not keep any of its input.
pub trait PayloadCodec: Sized {
    /// Writes the value. A failure is recorded on the writer.
    fn encode(&self, out: &mut PayloadWriter);
    /// Reads one value, leaving any following bytes to the caller.
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError>;
    /// The type's schema fingerprint: the shape of what `encode` writes (see
    /// [`payload_schema`](crate::payload_schema)). `visiting` holds the named types the walk is
    /// inside, by `TypeId`; a hand-written codec returns a leaf of its own name and version,
    /// `payload_schema_leaf("example:position/1")`.
    fn schema(visiting: &mut Vec<std::any::TypeId>) -> u64
    where
        Self: 'static;
}

/// Encodes `value` as a whole payload body of at most `limit` bytes.
pub fn encode_payload<T: PayloadCodec>(
    value: &T,
    limit: usize,
) -> Result<Vec<u8>, PayloadEncodeError> {
    let mut out = PayloadWriter::new(limit);
    value.encode(&mut out);
    out.finish()
}

/// Decodes the whole of `body` as one `T`: a byte left over is
/// [`PayloadDecodeError::Trailing`].
pub fn decode_payload<T: PayloadCodec>(body: &[u8]) -> Result<T, PayloadDecodeError> {
    let mut input = PayloadReader::new(body);
    let value = T::decode(&mut input)?;
    input.finish()?;
    Ok(value)
}

/// A payload body being read. It borrows the body and never reads past it.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: borrows a live decoding buffer.
#[derive(Debug)]
pub struct PayloadReader<'a> {
    data: &'a [u8],
    depth: u16,
    budget: Budget,
}

macro_rules! read_numbers {
    ($($name:ident: $ty:ty, $doc:literal;)*) => {$(
        #[doc = $doc]
        pub fn $name(&mut self) -> Result<$ty, PayloadDecodeError> {
            Ok(<$ty>::from_be_bytes(self.array()?))
        }
    )*};
}

impl<'a> PayloadReader<'a> {
    /// Reads `body` with [`PAYLOAD_DECODE_BUDGET`] bytes of allocation.
    pub fn new(body: &'a [u8]) -> Self {
        Self::with_budget(body, PAYLOAD_DECODE_BUDGET)
    }

    /// Reads `body`, letting its value allocate at most `budget` bytes.
    pub fn with_budget(body: &'a [u8], budget: usize) -> Self {
        Self {
            data: body,
            depth: 0,
            budget: Budget::new(budget as u64),
        }
    }

    /// How many bytes are not read yet.
    pub fn remaining(&self) -> usize {
        self.data.len()
    }

    /// Ends the read: [`PayloadDecodeError::Trailing`] when bytes are left.
    pub fn finish(self) -> Result<(), PayloadDecodeError> {
        match self.data.len() {
            0 => Ok(()),
            bytes => Err(PayloadDecodeError::Trailing {
                bytes: bytes as u64,
            }),
        }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PayloadDecodeError> {
        let (head, tail) = self
            .data
            .split_first_chunk::<N>()
            .ok_or(PayloadDecodeError::UnexpectedEnd)?;
        self.data = tail;
        Ok(*head)
    }

    fn slice(&mut self, len: usize) -> Result<&'a [u8], PayloadDecodeError> {
        let (head, tail) = self
            .data
            .split_at_checked(len)
            .ok_or(PayloadDecodeError::UnexpectedEnd)?;
        self.data = tail;
        Ok(head)
    }

    /// Refuses with `BudgetExhausted` when a charge does not fit.
    fn charged(fits: bool) -> Result<(), PayloadDecodeError> {
        fits.then_some(())
            .ok_or(PayloadDecodeError::BudgetExhausted)
    }

    read_numbers! {
        u8: u8, "One unsigned byte.";
        i8: i8, "One signed byte.";
        i16: i16, "A big-endian 16-bit integer.";
        u16: u16, "A big-endian unsigned 16-bit integer.";
        i32: i32, "A big-endian 32-bit integer.";
        i64: i64, "A big-endian 64-bit integer.";
        f32: f32, "A big-endian 32-bit float; NaN and the infinities pass.";
        f64: f64, "A big-endian 64-bit float; NaN and the infinities pass.";
        uuid: u128, "A UUID: 16 bytes, most significant first.";
    }

    /// A boolean: 0 or 1, anything else is refused.
    pub fn bool(&mut self) -> Result<bool, PayloadDecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(PayloadDecodeError::InvalidDiscriminant {
                value: value.into(),
            }),
        }
    }

    /// A 32-bit float that must be finite.
    pub fn finite_f32(&mut self) -> Result<f32, PayloadDecodeError> {
        let value = self.f32()?;
        match value.is_finite() {
            true => Ok(value),
            false => Err(PayloadDecodeError::NonFinite),
        }
    }

    /// A 64-bit float that must be finite.
    pub fn finite_f64(&mut self) -> Result<f64, PayloadDecodeError> {
        let value = self.f64()?;
        match value.is_finite() {
            true => Ok(value),
            false => Err(PayloadDecodeError::NonFinite),
        }
    }

    fn var(&mut self, groups: u32) -> Result<u64, PayloadDecodeError> {
        let mut value = 0u64;
        for group in 0..groups {
            let byte = self.u8()?;
            value |= u64::from(byte & 0x7F) << (7 * group);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(PayloadDecodeError::VarIntTooLong)
    }

    /// A VarInt: up to five 7-bit groups, least significant first.
    pub fn var_int(&mut self) -> Result<i32, PayloadDecodeError> {
        // Bits past the 32nd of a fifth group are dropped, as vanilla drops them.
        Ok(self.var(5)? as u32 as i32)
    }

    /// A VarLong: up to ten 7-bit groups, least significant first.
    pub fn var_long(&mut self) -> Result<i64, PayloadDecodeError> {
        Ok(self.var(10)? as i64)
    }

    /// A VarInt length or count of at most `limit`. A negative one and one
    /// past the limit are refused here, before the caller reserves anything.
    pub fn length(&mut self, limit: usize) -> Result<usize, PayloadDecodeError> {
        let length = self.var_int()?;
        let Ok(length) = u32::try_from(length) else {
            return Err(PayloadDecodeError::NegativeLength { length });
        };
        match usize::try_from(length) {
            Ok(count) if count <= limit => Ok(count),
            _ => Err(PayloadDecodeError::LengthOverLimit {
                length,
                limit: u32::try_from(limit).unwrap_or(u32::MAX),
            }),
        }
    }

    /// A byte array of at most `limit` bytes: its VarInt length, then the
    /// bytes, borrowed from the body.
    pub fn bytes(&mut self, limit: usize) -> Result<&'a [u8], PayloadDecodeError> {
        let len = self.length(limit)?;
        self.slice(len)
    }

    /// A string of at most `limit` UTF-8 bytes: its VarInt byte length, then
    /// the bytes, borrowed from the body. For a vanilla N-character bound,
    /// a schema can pass 3N UTF-8 bytes.
    pub fn str_within(&mut self, limit: usize) -> Result<&'a str, PayloadDecodeError> {
        std::str::from_utf8(self.bytes(limit)?).map_err(|_| PayloadDecodeError::Utf8)
    }

    /// An owned string of at most [`PAYLOAD_MAX_STRING_BYTES`] UTF-8 bytes.
    pub fn string(&mut self) -> Result<String, PayloadDecodeError> {
        let text = self.str_within(PAYLOAD_MAX_STRING_BYTES)?;
        Self::charged(self.budget.string(text.len()))?;
        Ok(text.to_owned())
    }

    /// Reads one level down, refusing past [`PAYLOAD_MAX_DEPTH`]. A codec of
    /// a type that can hold itself calls this around the inner value.
    pub fn nested<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, PayloadDecodeError>,
    ) -> Result<T, PayloadDecodeError> {
        if self.depth >= PAYLOAD_MAX_DEPTH {
            return Err(PayloadDecodeError::TooDeep);
        }
        self.depth += 1;
        let value = read(self);
        self.depth -= 1;
        value
    }

    /// A list of at most `limit` entries: its VarInt count, then each entry
    /// read by `read`, one level down. The count is checked against `limit`
    /// and the list's decoded size against the budget before it is reserved.
    pub fn list<T>(
        &mut self,
        limit: usize,
        mut read: impl FnMut(&mut Self) -> Result<T, PayloadDecodeError>,
    ) -> Result<Vec<T>, PayloadDecodeError> {
        let count = self.length(limit)?;
        Self::charged(self.budget.list::<T>(count))?;
        self.nested(|input| {
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read(input)?);
            }
            Ok(items)
        })
    }

    /// An optional value: a boolean, then the value one level down when it
    /// is true.
    pub fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, PayloadDecodeError>,
    ) -> Result<Option<T>, PayloadDecodeError> {
        match self.bool()? {
            true => self.nested(read).map(Some),
            false => Ok(None),
        }
    }
}

/// A payload body being written, bounded in bytes. The first failure is
/// kept and every later write is skipped, so a codec need not check each one.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: owns a live encoding buffer.
#[derive(Debug)]
pub struct PayloadWriter {
    bytes: Vec<u8>,
    limit: usize,
    depth: u16,
    budget: Budget,
    error: Option<PayloadEncodeError>,
}

macro_rules! write_numbers {
    ($($name:ident: $ty:ty, $doc:literal;)*) => {$(
        #[doc = $doc]
        pub fn $name(&mut self, value: $ty) {
            self.raw(&value.to_be_bytes());
        }
    )*};
}

impl PayloadWriter {
    /// A writer that accepts at most `limit` bytes.
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            depth: 0,
            budget: Budget::new(PAYLOAD_DECODE_BUDGET as u64),
            error: None,
        }
    }

    /// Records `BudgetExhausted` when a charge does not fit; whether it did.
    fn charged(&mut self, fits: bool) -> bool {
        if !fits {
            self.fail(PayloadEncodeError::BudgetExhausted);
        }
        fits
    }

    /// The encoded body, or the first failure.
    pub fn finish(self) -> Result<Vec<u8>, PayloadEncodeError> {
        match self.error {
            None => Ok(self.bytes),
            Some(error) => Err(error),
        }
    }

    /// Records a failure of the value's own codec; the first one is kept.
    pub fn fail(&mut self, error: PayloadEncodeError) {
        self.error.get_or_insert(error);
    }

    /// Appends bytes as they are, with no length before them. The limit is
    /// checked before the buffer grows.
    pub fn raw(&mut self, bytes: &[u8]) {
        if self.error.is_some() {
            return;
        }
        match self.bytes.len().checked_add(bytes.len()) {
            Some(total) if total <= self.limit => self.bytes.extend_from_slice(bytes),
            _ => self.fail(PayloadEncodeError::TooLarge {
                limit: self.limit as u64,
            }),
        }
    }

    write_numbers! {
        u8: u8, "One unsigned byte.";
        i8: i8, "One signed byte.";
        i16: i16, "A big-endian 16-bit integer.";
        u16: u16, "A big-endian unsigned 16-bit integer.";
        i32: i32, "A big-endian 32-bit integer.";
        i64: i64, "A big-endian 64-bit integer.";
        f32: f32, "A big-endian 32-bit float.";
        f64: f64, "A big-endian 64-bit float.";
        uuid: u128, "A UUID: 16 bytes, most significant first.";
    }

    /// A boolean: one byte, 0 or 1.
    pub fn bool(&mut self, value: bool) {
        self.u8(value.into());
    }

    /// A 32-bit float that must be finite.
    pub fn finite_f32(&mut self, value: f32) {
        match value.is_finite() {
            true => self.f32(value),
            false => self.fail(PayloadEncodeError::NonFinite),
        }
    }

    /// A 64-bit float that must be finite.
    pub fn finite_f64(&mut self, value: f64) {
        match value.is_finite() {
            true => self.f64(value),
            false => self.fail(PayloadEncodeError::NonFinite),
        }
    }

    fn var(&mut self, mut value: u64) {
        let mut groups = [0u8; 10];
        let mut len = 0;
        for group in &mut groups {
            len += 1;
            *group = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                break;
            }
            *group |= 0x80;
        }
        self.raw(&groups[..len]);
    }

    /// A VarInt.
    pub fn var_int(&mut self, value: i32) {
        self.var(u64::from(value as u32));
    }

    /// A VarLong.
    pub fn var_long(&mut self, value: i64) {
        self.var(value as u64);
    }

    /// A VarInt length or count, refused past `limit` or past what a VarInt
    /// holds.
    fn length(&mut self, length: usize, limit: usize) -> bool {
        match i32::try_from(length) {
            Ok(count) if length <= limit => {
                self.var_int(count);
                true
            }
            _ => {
                self.fail(PayloadEncodeError::LengthOverLimit {
                    length: length as u64,
                    limit: u32::try_from(limit).unwrap_or(u32::MAX),
                });
                false
            }
        }
    }

    /// A byte array of at most `limit` bytes: its VarInt length, then the bytes.
    pub fn bytes(&mut self, bytes: &[u8], limit: usize) {
        if self.length(bytes.len(), limit) {
            self.raw(bytes);
        }
    }

    /// A string of at most `limit` UTF-8 bytes: its VarInt byte length, then
    /// the bytes.
    pub fn str_within(&mut self, text: &str, limit: usize) {
        self.bytes(text.as_bytes(), limit);
    }

    /// A string of at most [`PAYLOAD_MAX_STRING_BYTES`] UTF-8 bytes.
    pub fn string(&mut self, text: &str) {
        self.str_within(text, PAYLOAD_MAX_STRING_BYTES);
        let fits = self.budget.string(text.len());
        self.charged(fits);
    }

    /// Writes one level down, refusing past [`PAYLOAD_MAX_DEPTH`].
    pub fn nested(&mut self, write: impl FnOnce(&mut Self)) {
        if self.error.is_some() {
            return;
        }
        if self.depth >= PAYLOAD_MAX_DEPTH {
            self.fail(PayloadEncodeError::TooDeep);
            return;
        }
        self.depth += 1;
        write(self);
        self.depth -= 1;
    }

    /// A list of at most `limit` entries: its VarInt count, then each entry
    /// written by `write`, one level down, charged to the budget as its reader charges it.
    pub fn list<T>(&mut self, items: &[T], limit: usize, mut write: impl FnMut(&mut Self, &T)) {
        if self.length(items.len(), limit) {
            let fits = self.budget.list::<T>(items.len());
            if self.charged(fits) {
                self.nested(|out| {
                    for item in items {
                        write(out, item);
                    }
                });
            }
        }
    }

    /// An optional value: a boolean, then the value one level down when
    /// there is one.
    pub fn option<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) {
        self.bool(value.is_some());
        if let Some(value) = value {
            self.nested(|out| write(out, value));
        }
    }
}

macro_rules! number_codecs {
    ($($name:ident: $ty:ty),*) => {$(
        impl PayloadCodec for $ty {
            fn encode(&self, out: &mut PayloadWriter) {
                out.$name(*self);
            }
            fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
                input.$name()
            }
            fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
                payload_schema_leaf(stringify!($name))
            }
        }
    )*};
}
number_codecs!(bool: bool, u8: u8, i8: i8, i16: i16, u16: u16, i32: i32, i64: i64, f32: f32, f64: f64);

/// An `i32` written as a VarInt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
// exhaustive: a newtype over its one number
pub struct VarInt(pub i32);

/// An `i64` written as a VarLong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
// exhaustive: a newtype over its one number
pub struct VarLong(pub i64);

impl PayloadCodec for VarInt {
    fn encode(&self, out: &mut PayloadWriter) {
        out.var_int(self.0);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.var_int().map(Self)
    }
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        payload_schema_leaf("var_int")
    }
}

impl PayloadCodec for VarLong {
    fn encode(&self, out: &mut PayloadWriter) {
        out.var_long(self.0);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.var_long().map(Self)
    }
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        payload_schema_leaf("var_long")
    }
}

/// At most [`PAYLOAD_MAX_STRING_BYTES`] UTF-8 bytes.
impl PayloadCodec for String {
    fn encode(&self, out: &mut PayloadWriter) {
        out.string(self);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.string()
    }
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        payload_schema_mix(
            payload_schema_leaf("string"),
            PAYLOAD_MAX_STRING_BYTES as u64,
        )
    }
}

/// At most [`PAYLOAD_MAX_LIST_ENTRIES`] entries; a schema with another bound
/// calls [`PayloadReader::list`] itself.
impl<T: PayloadCodec> PayloadCodec for Vec<T> {
    fn encode(&self, out: &mut PayloadWriter) {
        out.list(self, PAYLOAD_MAX_LIST_ENTRIES, |out, item| item.encode(out));
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.list(PAYLOAD_MAX_LIST_ENTRIES, T::decode)
    }
    fn schema(visiting: &mut Vec<std::any::TypeId>) -> u64
    where
        Self: 'static,
    {
        let list = payload_schema_mix(payload_schema_leaf("list"), PAYLOAD_MAX_LIST_ENTRIES as u64);
        payload_schema_mix(list, T::schema(visiting))
    }
}

impl<T: PayloadCodec> PayloadCodec for Option<T> {
    fn encode(&self, out: &mut PayloadWriter) {
        out.option(self.as_ref(), |out, value| value.encode(out));
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.option(T::decode)
    }
    fn schema(visiting: &mut Vec<std::any::TypeId>) -> u64
    where
        Self: 'static,
    {
        payload_schema_mix(payload_schema_leaf("option"), T::schema(visiting))
    }
}

/// The boxed value's own form: what lets a type hold itself.
impl<T: PayloadCodec> PayloadCodec for Box<T> {
    fn encode(&self, out: &mut PayloadWriter) {
        out.nested(|out| {
            let fits = out.budget.boxed::<T>();
            if out.charged(fits) {
                T::encode(self, out);
            }
        });
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.nested(|input| {
            PayloadReader::charged(input.budget.boxed::<T>())?;
            T::decode(input).map(Box::new)
        })
    }
    fn schema(visiting: &mut Vec<std::any::TypeId>) -> u64
    where
        Self: 'static,
    {
        payload_schema_mix(payload_schema_leaf("box"), T::schema(visiting))
    }
}

#[cfg(test)]
#[path = "payload_codec_tests.rs"]
mod tests;
