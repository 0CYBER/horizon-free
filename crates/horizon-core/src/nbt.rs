//! Java Edition NBT with lossless UTF-16 strings and 26.3 mixed-list wrappers.
use std::collections::BTreeMap;

/// Java strings can contain unpaired surrogates, which Rust UTF-8 strings cannot.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JavaString(pub Vec<u16>);

impl From<&str> for JavaString {
    fn from(value: &str) -> Self {
        Self(value.encode_utf16().collect())
    }
}
/// `key` in UTF-16 in a buffer kept per thread, to search a compound
/// without allocating a key every time.
fn with_key<R>(key: &str, f: impl FnOnce(&JavaString) -> R) -> R {
    thread_local! {
        static KEY: std::cell::RefCell<JavaString> = Default::default();
    }
    KEY.with_borrow_mut(|units| {
        units.0.clear();
        units.0.extend(key.encode_utf16());
        f(units)
    })
}

impl JavaString {
    pub fn to_string(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.0)
    }
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
}

pub type Compound = BTreeMap<JavaString, Tag>;

/// Float and double variants hold their IEEE bit patterns. Java's disk output
/// canonicalizes NaNs (`DataOutputStream.writeFloat`); its network output
/// keeps their bits (Netty's `ByteBuf.writeFloat`). Loading, like vanilla's
/// valueOf factories, normalizes negative zero to positive zero.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tag {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(u32),
    Double(u64),
    ByteArray(Vec<u8>),
    String(JavaString),
    List(Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Self::End => 0,
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Self::Compound(map) => with_key(key, |key| map.get(key)),
            _ => None,
        }
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        match self {
            Self::Compound(map) => with_key(key, |key| map.get_mut(key)),
            _ => None,
        }
    }
    pub fn compound(entries: impl IntoIterator<Item = (impl Into<JavaString>, Tag)>) -> Self {
        Self::Compound(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
}

/// Clone opaque NBT with an explicit work list, without recursive cloning.
/// The returned tree retains ordinary Tag ownership and drop behavior.
pub fn clone_tag(root: &Tag) -> Tag {
    match root {
        Tag::List(values) if values.is_empty() => return root.clone(),
        Tag::Compound(values) if values.is_empty() => return root.clone(),
        Tag::List(_) | Tag::Compound(_) => {}
        _ => return root.clone(),
    }
    enum Work<'a> {
        Enter(&'a Tag),
        List(usize),
        Compound(Vec<JavaString>),
    }
    let mut work = vec![Work::Enter(root)];
    let mut done = Vec::new();
    while let Some(step) = work.pop() {
        match step {
            Work::Enter(Tag::List(values)) => {
                work.push(Work::List(values.len()));
                work.extend(values.iter().rev().map(Work::Enter));
            }
            Work::Enter(Tag::Compound(values)) => {
                work.push(Work::Compound(values.keys().cloned().collect()));
                work.extend(values.values().rev().map(Work::Enter));
            }
            Work::Enter(
                value @ (Tag::End
                | Tag::Byte(_)
                | Tag::Short(_)
                | Tag::Int(_)
                | Tag::Long(_)
                | Tag::Float(_)
                | Tag::Double(_)
                | Tag::String(_)
                | Tag::ByteArray(_)
                | Tag::IntArray(_)
                | Tag::LongArray(_)),
            ) => done.push(value.clone()),
            Work::List(len) => {
                let values = done.split_off(done.len() - len);
                done.push(Tag::List(values));
            }
            Work::Compound(keys) => {
                let values = done.split_off(done.len() - keys.len());
                done.push(Tag::Compound(keys.into_iter().zip(values).collect()));
            }
        }
    }
    done.pop().expect("one opaque root")
}

/// Release a boundary-owned tag without recursive list/compound drops.
pub fn discard_tag(root: Tag) {
    let mut pending = vec![root];
    while let Some(tag) = pending.pop() {
        match tag {
            Tag::List(values) => pending.extend(values),
            Tag::Compound(values) => pending.extend(values.into_values()),
            _ => {}
        }
    }
}

/// Whether recursive lists/compounds fit the supplied NBT container bound.
/// Scalars and primitive arrays consume no recursive nesting.
pub fn within_depth(root: &Tag, limit: usize) -> bool {
    match root {
        Tag::List(values) if values.is_empty() => return limit > 0,
        Tag::Compound(values) if values.is_empty() => return limit > 0,
        Tag::List(_) | Tag::Compound(_) => {}
        _ => return true,
    }
    let mut work = vec![(root, 0)];
    while let Some((tag, depth)) = work.pop() {
        match tag {
            Tag::List(values) => {
                if depth >= limit {
                    return false;
                }
                work.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Tag::Compound(values) => {
                if depth >= limit {
                    return false;
                }
                work.extend(values.values().map(|v| (v, depth + 1)));
            }
            _ => {}
        }
    }
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedTag {
    pub name: JavaString,
    pub tag: Tag,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_nodes: 1_000_000,
            max_depth: 512,
        }
    }
}
impl Limits {
    /// Vanilla's bounded network NBT policy, including the minimum per-tag heap charge.
    pub const fn network() -> Self {
        Self {
            max_bytes: 2_097_152,
            max_nodes: 2_097_152 / 8,
            max_depth: 512,
        }
    }

    /// The prepared per-state block tables (`cached-collision.nbt`,
    /// `block-properties.nbt`, ...): trusted files with a row per block
    /// state, which content mods' states grow past the default caps
    /// (vanilla's 35,723 states fill about a tenth of these; ATM10's ported
    /// mods, 193,762 states, about half).
    pub fn tables() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_nodes: 128_000_000,
            max_depth: 512,
        }
    }

    /// Chunks, entities, POI and the world's `.dat` files: vanilla reads and
    /// writes them with `NbtAccounter.unlimitedHeap()` (`NbtIo.read`), so a
    /// chunk past the default node cap (a few thousand full containers) still
    /// loads. The byte cap stays as the only guard on the decompressed data.
    pub fn saved() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_nodes: usize::MAX,
            max_depth: 512,
        }
    }

    /// Structure templates (`structure/*.nbt`): vanilla reads them with
    /// `NbtAccounter.unlimitedHeap()` (TemplateSource.readStructure), and
    /// mods ship templates past the default node cap (YUNG's Better Ocean
    /// Monuments' start piece, 115x64x126 blocks, holds millions of tags).
    /// The byte cap stays as the only guard on the decompressed file.
    pub fn templates() -> Self {
        Self {
            max_bytes: 1 << 28,
            max_nodes: usize::MAX,
            max_depth: 512,
        }
    }
}

/// A compound entry that [`read_named_bounded`] reads under a budget of its own, without
/// changing the limits of the rest of the file: the entry keyed `key` directly inside a
/// compound reached from the root by one of the key paths in `parents`.
#[derive(Clone, Copy, Debug)]
pub struct Bounded {
    /// The key paths from the root to each compound that holds the entry, at most
    /// [`MAX_BOUNDED_PARENTS`]. `&["a", "b"]` is the root's `a`, then its `b`.
    pub parents: &'static [&'static [&'static str]],
    /// The entry's key.
    pub key: &'static str,
    /// The most bytes its payload may take.
    pub max_bytes: usize,
    /// The most compound and list levels its payload may hold, itself included.
    pub max_depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub reason: &'static str,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NBT at byte {}: {}", self.offset, self.reason)
    }
}
impl std::error::Error for Error {}

/// Hands a streamed list's elements to the caller one at a time (see
/// [`read_named_streaming`]).
type Visit<'s> = &'s mut dyn FnMut(&mut ListElements<'_, '_, '_>) -> Result<(), Error>;

struct Reader<'a, 's> {
    bytes: &'a [u8],
    offset: usize,
    limits: Limits,
    nodes: usize,
    heap: Option<(u64, u64)>,
    /// Lists with this key in the root compound or in a compound directly
    /// under it are streamed to the visitor instead of kept.
    stream: Option<(&'s str, Visit<'s>)>,
    /// Entries read under their own budget (see [`read_named_bounded`]).
    bounded: Option<Bounded>,
    /// Inside a bounded entry, where a repeated key is an error.
    strict: bool,
    /// The [`Bounded::parents`] paths (one bit each) that lead to the compound about to be
    /// read, at its depth.
    entering: u32,
    /// Bounded entries left out.
    rejected: usize,
}
impl<'a> Reader<'a, '_> {
    // Vanilla 26.3 NbtAccounter.accountBytes; disabled for existing disk readers.
    fn charge(&mut self, bytes: u64) -> Result<(), Error> {
        if let Some((used, quota)) = self.heap {
            let next = used
                .checked_add(bytes)
                .filter(|&n| n <= quota)
                .ok_or_else(|| self.error("heap quota exceeded"))?;
            self.heap = Some((next, quota));
        }
        Ok(())
    }
    fn error(&self, reason: &'static str) -> Error {
        Error {
            offset: self.offset,
            reason,
        }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| self.error("length overflow"))?;
        if end > self.limits.max_bytes {
            return Err(self.error("byte limit exceeded"));
        }
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.error("truncated input"))?;
        self.offset = end;
        Ok(value)
    }
    fn number<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn count(&mut self, element_bytes: usize) -> Result<usize, Error> {
        let count = i32::from_be_bytes(self.number()?);
        if count < 0 {
            return Err(self.error("negative collection length"));
        }
        let count = count as usize;
        let size = count
            .checked_mul(element_bytes)
            .ok_or_else(|| self.error("length overflow"))?;
        if size > self.bytes.len().saturating_sub(self.offset)
            || size > self.limits.max_bytes.saturating_sub(self.offset)
        {
            return Err(self.error("collection exceeds available bytes"));
        }
        Ok(count)
    }
    fn string(&mut self) -> Result<JavaString, Error> {
        let len = u16::from_be_bytes(self.number()?) as usize;
        let bytes = self.take(len)?;
        modified_utf8(bytes).ok_or_else(|| self.error("invalid Java modified UTF-8"))
    }
    /// `StringTag.skipString`: vanilla skips a file root's name unread, so a
    /// name that isn't modified UTF-8 is read as empty rather than refused.
    fn root_name(&mut self) -> Result<JavaString, Error> {
        let len = u16::from_be_bytes(self.number()?) as usize;
        Ok(modified_utf8(self.take(len)?).unwrap_or_default())
    }
    fn entry(&mut self, id: u8, depth: usize) -> Result<(), Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if matches!(id, 9 | 10) && depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        self.charge(match id {
            0 => 8,
            1 => 9,
            2 => 10,
            3 | 5 => 12,
            4 | 6 => 16,
            7 | 11 | 12 => 24,
            8 | 9 => 36,
            10 => 48,
            _ => 0,
        })?;
        Ok(())
    }
    fn list_header(&mut self) -> Result<(u8, usize), Error> {
        let element_id = self.byte()?;
        let len = self.count(0)?;
        if len > 0 && (element_id == 0 || element_id > 12) {
            return Err(self.error("invalid list element type"));
        }
        if len > self.limits.max_nodes.saturating_sub(self.nodes) {
            return Err(self.error("node limit exceeded"));
        }
        self.list_fits(element_id, len)?;
        Ok((element_id, len))
    }
    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag, Error> {
        self.entry(id, depth)?;
        match id {
            9 => self.list_payload(depth),
            10 => self.compound_payload(depth),
            _ => self.scalar_payload(id),
        }
    }
    // Scalar temporaries must not occupy every recursive container frame.
    fn scalar_payload(&mut self, id: u8) -> Result<Tag, Error> {
        Ok(match id {
            0 => Tag::End,
            1 => Tag::Byte(self.byte()? as i8),
            2 => Tag::Short(i16::from_be_bytes(self.number()?)),
            3 => Tag::Int(i32::from_be_bytes(self.number()?)),
            4 => Tag::Long(i64::from_be_bytes(self.number()?)),
            5 => {
                let bits = u32::from_be_bytes(self.number()?);
                Tag::Float(if f32::from_bits(bits) == 0.0 { 0 } else { bits })
            }
            6 => {
                let bits = u64::from_be_bytes(self.number()?);
                Tag::Double(if f64::from_bits(bits) == 0.0 { 0 } else { bits })
            }
            7 => {
                let len = self.count(1)?;
                self.charge(len as u64)?;
                Tag::ByteArray(self.take(len)?.to_vec())
            }
            8 => {
                let value = self.string()?;
                self.charge(2 * value.0.len() as u64)?;
                Tag::String(value)
            }

            11 => {
                let len = self.count(4)?;
                self.charge(4 * len as u64)?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(i32::from_be_bytes(self.number()?));
                }
                Tag::IntArray(values)
            }
            12 => {
                let len = self.count(8)?;
                self.charge(8 * len as u64)?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(i64::from_be_bytes(self.number()?));
                }
                Tag::LongArray(values)
            }
            _ => return Err(self.error("unknown tag type")),
        })
    }
    fn list_payload(&mut self, depth: usize) -> Result<Tag, Error> {
        let (element_id, len) = self.list_header()?;
        self.charge(4 * len as u64)?;
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            let mut value = self.payload(element_id, depth + 1)?;
            if let Tag::Compound(map) = &mut value
                && map.len() == 1
                && map.contains_key(&JavaString::default())
            {
                value = map.remove(&JavaString::default()).unwrap();
            }
            values.push(value);
        }
        Ok(Tag::List(values))
    }
    fn compound_payload(&mut self, depth: usize) -> Result<Tag, Error> {
        let paths = std::mem::take(&mut self.entering);
        let parent = self.holds_bounded(paths, depth);
        let mut map = Compound::new();
        loop {
            let id = self.byte()?;
            if id == 0 {
                break;
            }
            let name = self.string()?;
            self.charge(28 + 2 * name.0.len() as u64)?;
            if self.strict && map.contains_key(&name) {
                return Err(self.error("duplicate key"));
            }
            let value = if id == 9 && depth <= 1 && self.streams(&name) {
                self.stream_list(depth + 1)?
            } else if parent && !self.strict && self.bounds(&name) {
                match self.bounded_payload(id, depth + 1)? {
                    Some(value) => value,
                    None => continue,
                }
            } else {
                self.entering = if id == 10 && !self.strict {
                    self.descend(paths, depth, &name)
                } else {
                    0
                };
                self.payload(id, depth + 1)?
            };
            // Java maps retain the last duplicate key and charge new entries only.
            if map.insert(name, value).is_none() {
                self.charge(36)?;
            }
        }
        Ok(Tag::Compound(map))
    }
    fn bounds(&self, name: &JavaString) -> bool {
        self.bounded.is_some_and(|b| is_key(name, b.key))
    }
    /// The `paths` that end at the compound at `depth`, so its [`Bounded::key`] is bounded.
    fn holds_bounded(&self, paths: u32, depth: usize) -> bool {
        self.parent_paths(paths)
            .any(|(_, path)| path.len() == depth)
    }
    /// The `paths` that lead on from the compound at `depth` through its child `name`.
    fn descend(&self, paths: u32, depth: usize, name: &JavaString) -> u32 {
        self.parent_paths(paths)
            .filter(|(_, path)| path.get(depth).is_some_and(|key| is_key(name, key)))
            .fold(0, |next, (i, _)| next | 1 << i)
    }
    /// The [`Bounded::parents`] paths whose bits are set in `paths`, with their indices.
    fn parent_paths(&self, paths: u32) -> impl Iterator<Item = (usize, &'static [&'static str])> {
        let parents = self.bounded.map_or(&[][..], |b| b.parents);
        let parents = &parents[..parents.len().min(MAX_BOUNDED_PARENTS)];
        (0..parents.len())
            .filter(move |i| paths & (1 << i) != 0)
            .map(move |i| (i, parents[i]))
    }
    /// Whether `len` elements of type `element_id` can fit in what is left of the input and of
    /// the byte limit, each taking at least its type's smallest payload.
    fn list_fits(&self, element_id: u8, len: usize) -> Result<(), Error> {
        let least = match element_id {
            1 | 10 => 1,
            2 | 8 => 2,
            3 | 5 | 7 | 11 | 12 => 4,
            4 | 6 => 8,
            9 => 5,
            _ => 0,
        };
        let needs = len.saturating_mul(least);
        if needs > self.bytes.len().saturating_sub(self.offset) {
            return Err(self.error("truncated list"));
        }
        if needs > self.limits.max_bytes.saturating_sub(self.offset) {
            return Err(self.error("byte limit exceeded"));
        }
        Ok(())
    }
    /// A bounded entry's payload, or `None` when it passes its budget or repeats a key: it is
    /// then read past without being built, under the file's own limits, so input the plain
    /// reader refuses is still refused.
    fn bounded_payload(&mut self, id: u8, depth: usize) -> Result<Option<Tag>, Error> {
        let bounded = self.bounded.expect("bounds() checked it");
        let (start, nodes, outer) = (self.offset, self.nodes, self.limits);
        self.limits.max_bytes = outer.max_bytes.min(start.saturating_add(bounded.max_bytes));
        self.limits.max_depth = outer.max_depth.min(depth.saturating_add(bounded.max_depth));
        self.strict = true;
        let read = self.payload(id, depth);
        (self.limits, self.strict) = (outer, false);
        if let Ok(tag) = read {
            return Ok(Some(tag));
        }
        (self.offset, self.nodes) = (start, nodes);
        self.skip(id, depth)?;
        self.rejected += 1;
        Ok(None)
    }
    /// Reads past a payload without building it, with the checks [`Self::payload`] makes.
    fn skip(&mut self, id: u8, depth: usize) -> Result<(), Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if matches!(id, 9 | 10) && depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        match id {
            0 => {}
            1 => drop(self.take(1)?),
            2 => drop(self.take(2)?),
            3 | 5 => drop(self.take(4)?),
            4 | 6 => drop(self.take(8)?),
            7 | 11 | 12 => {
                let width = match id {
                    7 => 1,
                    11 => 4,
                    _ => 8,
                };
                let len = self.count(width)?;
                self.take(len * width)?;
            }
            8 => drop(self.string()?),
            9 => {
                let element_id = self.byte()?;
                let len = self.count(0)?;
                if len > 0 && (element_id == 0 || element_id > 12) {
                    return Err(self.error("invalid list element type"));
                }
                self.list_fits(element_id, len)?;
                for _ in 0..len {
                    self.skip(element_id, depth + 1)?;
                }
            }
            10 => loop {
                let id = self.byte()?;
                if id == 0 {
                    break;
                }
                self.string()?;
                self.skip(id, depth + 1)?;
            },
            _ => return Err(self.error("unknown tag type")),
        }
        Ok(())
    }
    fn streams(&self, name: &JavaString) -> bool {
        self.stream
            .as_ref()
            .is_some_and(|(key, _)| name.0.iter().copied().eq(key.encode_utf16()))
    }
    /// A list's payload handed to the visitor element by element; the tree
    /// keeps it empty. Elements the visitor leaves are read and dropped.
    fn stream_list(&mut self, depth: usize) -> Result<Tag, Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        let element_id = self.byte()?;
        let len = self.count(0)?;
        if len > 0 && (element_id == 0 || element_id > 12) {
            return Err(self.error("invalid list element type"));
        }
        if len > self.bytes.len().saturating_sub(self.offset) {
            return Err(self.error("truncated list"));
        }
        let (key, visit) = self.stream.take().expect("streams() checked the visitor");
        let mut elements = ListElements {
            reader: self,
            element_id,
            depth: depth + 1,
            len,
            left: len,
        };
        let rest = visit(&mut elements).and_then(|()| {
            while elements.next_element()?.is_some() {}
            Ok(())
        });
        self.stream = Some((key, visit));
        rest.map(|()| Tag::List(Vec::new()))
    }
}

/// The elements of a list [`read_named_streaming`] streams, read one at a
/// time.
pub struct ListElements<'r, 'a, 's> {
    reader: &'r mut Reader<'a, 's>,
    element_id: u8,
    depth: usize,
    len: usize,
    left: usize,
}
impl ListElements<'_, '_, '_> {
    /// The list's length, as its header gives it.
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// The next element, unwrapped as [`read_named`] unwraps mixed-list
    /// elements; `None` past the last.
    pub fn next_element(&mut self) -> Result<Option<Tag>, Error> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        let mut value = self.reader.payload(self.element_id, self.depth)?;
        if let Tag::Compound(map) = &mut value
            && map.len() == 1
            && map.contains_key(&JavaString::default())
        {
            value = map.remove(&JavaString::default()).unwrap();
        }
        Ok(Some(value))
    }
    /// An error at the current position in the input.
    pub fn error(&self, reason: &'static str) -> Error {
        self.reader.error(reason)
    }
}

fn is_key(name: &JavaString, key: &str) -> bool {
    name.0.iter().copied().eq(key.encode_utf16())
}

/// `DataInput.readUTF`'s decoding, accepting its overlong sequences; `None`
/// for bytes that aren't modified UTF-8.
fn modified_utf8(bytes: &[u8]) -> Option<JavaString> {
    if bytes.is_ascii() {
        return Some(JavaString(bytes.iter().map(|&b| u16::from(b)).collect()));
    }
    let len = bytes.len();
    let mut units = Vec::with_capacity(len);
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let (unit, size) = match b {
            0..=127 => (u16::from(b), 1),
            192..=223 if i + 1 < len && bytes[i + 1] & 0xc0 == 0x80 => {
                ((u16::from(b & 31) << 6) | u16::from(bytes[i + 1] & 63), 2)
            }
            224..=239
                if i + 2 < len && bytes[i + 1] & 0xc0 == 0x80 && bytes[i + 2] & 0xc0 == 0x80 =>
            {
                (
                    (u16::from(b & 15) << 12)
                        | (u16::from(bytes[i + 1] & 63) << 6)
                        | u16::from(bytes[i + 2] & 63),
                    3,
                )
            }
            _ => return None,
        };
        // Match DataInput.readUTF, including accepted overlong sequences.
        units.push(unit);
        i += size;
    }
    Some(JavaString(units))
}

/// Read a file root (type, modified-UTF name, payload), returning bytes consumed.
pub fn read_named(bytes: &[u8], limits: Limits) -> Result<(NamedTag, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        heap: None,
        stream: None,
        bounded: None,
        strict: false,
        entering: 0,
        rejected: 0,
    };
    let id = reader.byte()?;
    let name = if id == 0 {
        JavaString::default()
    } else {
        reader.root_name()?
    };
    let tag = reader.payload(id, 0)?;
    Ok((NamedTag { name, tag }, reader.offset))
}

/// [`read_named`], except that the entry keyed `bounded.key` in each of the compounds
/// `bounded.parents` names is read under that budget and may repeat no key inside it. An
/// entry that passes its budget or repeats a key is read past without being built and left
/// out of its compound; the third value counts those. The rest of the file reads as
/// [`read_named`] reads it.
/// The most [`Bounded::parents`] paths; later ones are ignored.
pub const MAX_BOUNDED_PARENTS: usize = 31;

pub fn read_named_bounded(
    bytes: &[u8],
    limits: Limits,
    bounded: Bounded,
) -> Result<(NamedTag, usize, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        heap: None,
        stream: None,
        bounded: Some(bounded),
        strict: false,
        entering: (1u32 << bounded.parents.len().min(MAX_BOUNDED_PARENTS)) - 1,
        rejected: 0,
    };
    let id = reader.byte()?;
    let name = if id == 0 {
        JavaString::default()
    } else {
        reader.root_name()?
    };
    let tag = reader.payload(id, 0)?;
    Ok((NamedTag { name, tag }, reader.offset, reader.rejected))
}

/// [`read_named`], except that lists keyed `key` in the root compound or in
/// a compound directly under it are not kept: `visit` reads each one's
/// elements as they come, and the tree holds it as an empty list. Per-state
/// tables check hundreds of thousands of state identities this way without
/// holding them as a tree (crate::registry::BlockTableFile).
pub fn read_named_streaming(
    bytes: &[u8],
    limits: Limits,
    key: &str,
    visit: &mut dyn FnMut(&mut ListElements<'_, '_, '_>) -> Result<(), Error>,
) -> Result<(NamedTag, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        heap: None,
        stream: Some((key, visit)),
        bounded: None,
        strict: false,
        entering: 0,
        rejected: 0,
    };
    let id = reader.byte()?;
    let name = if id == 0 {
        JavaString::default()
    } else {
        reader.root_name()?
    };
    let tag = reader.payload(id, 0)?;
    Ok((NamedTag { name, tag }, reader.offset))
}

/// Read a network root (type and payload, without the file name field).
pub fn read_network(bytes: &[u8], limits: Limits) -> Result<(Tag, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        heap: None,
        stream: None,
        bounded: None,
        strict: false,
        entering: 0,
        rejected: 0,
    };
    read_network_with_reader(&mut reader)
}

fn read_network_with_reader(reader: &mut Reader<'_, '_>) -> Result<(Tag, usize), Error> {
    let id = reader.byte()?;
    // NbtIo.readAnyTag returns End directly without loading its payload.
    let tag = if id == 0 {
        Tag::End
    } else {
        reader.payload(id, 0)?
    };
    Ok((tag, reader.offset))
}

struct Writer {
    bytes: Vec<u8>,
    limits: Limits,
    nodes: usize,
    /// `NbtIo.StringFallbackDataOutput`, the disk writer's: a string too long
    /// for modified UTF-8 is written empty and the rest of the tree goes on.
    /// shortcut: vanilla also logs "Failed to write NBT String"; this crate has no logger.
    empty_long_strings: bool,
    /// `DataOutputStream.writeFloat`/`writeDouble` (`floatToIntBits`): a NaN
    /// is written as the canonical one. The network writer keeps its bits.
    canonical_nan: bool,
}
impl Writer {
    fn error(&self, reason: &'static str) -> Error {
        Error {
            offset: self.bytes.len(),
            reason,
        }
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > self.limits.max_bytes.saturating_sub(self.bytes.len()) {
            return Err(self.error("byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn count(&mut self, len: usize) -> Result<(), Error> {
        let len = i32::try_from(len).map_err(|_| self.error("collection too long"))?;
        self.put(&len.to_be_bytes())
    }
    fn string(&mut self, string: &JavaString) -> Result<(), Error> {
        let len: usize = string
            .0
            .iter()
            .map(|&c| match c {
                1..=127 => 1,
                0..=2047 => 2,
                _ => 3,
            })
            .sum();
        let Ok(len) = u16::try_from(len) else {
            if self.empty_long_strings {
                return self.put(&0u16.to_be_bytes());
            }
            return Err(self.error("modified UTF-8 exceeds 65535 bytes"));
        };
        self.put(&len.to_be_bytes())?;
        // One byte per unit (most strings): written at once while they fit.
        if usize::from(len) == string.0.len()
            && usize::from(len) <= self.limits.max_bytes.saturating_sub(self.bytes.len())
        {
            self.bytes.extend(string.0.iter().map(|&c| c as u8));
            return Ok(());
        }
        for &c in &string.0 {
            match c {
                1..=127 => self.put(&[c as u8])?,
                0..=2047 => self.put(&[0xc0 | (c >> 6) as u8, 0x80 | (c & 63) as u8])?,
                _ => self.put(&[
                    0xe0 | (c >> 12) as u8,
                    0x80 | ((c >> 6) & 63) as u8,
                    0x80 | (c & 63) as u8,
                ])?,
            }
        }
        Ok(())
    }
    fn payload(&mut self, tag: &Tag, depth: usize) -> Result<(), Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if matches!(tag, Tag::List(_) | Tag::Compound(_)) && depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        match tag {
            Tag::End => {}
            Tag::Byte(v) => self.put(&[*v as u8])?,
            Tag::Short(v) => self.put(&v.to_be_bytes())?,
            Tag::Int(v) => self.put(&v.to_be_bytes())?,
            Tag::Long(v) => self.put(&v.to_be_bytes())?,
            Tag::Float(v) => self.put(
                &if self.canonical_nan && f32::from_bits(*v).is_nan() {
                    0x7fc0_0000_u32
                } else {
                    *v
                }
                .to_be_bytes(),
            )?,
            Tag::Double(v) => self.put(
                &if self.canonical_nan && f64::from_bits(*v).is_nan() {
                    0x7ff8_0000_0000_0000_u64
                } else {
                    *v
                }
                .to_be_bytes(),
            )?,
            Tag::ByteArray(v) => {
                self.count(v.len())?;
                self.put(v)?;
            }
            Tag::String(v) => self.string(v)?,
            Tag::List(values) => {
                if values.iter().any(|v| matches!(v, Tag::End)) {
                    return Err(self.error("End cannot be a list element"));
                }
                let first = values.first().map_or(0, Tag::id);
                let id = if values.iter().any(|v| v.id() != first) {
                    10
                } else {
                    first
                };
                self.put(&[id])?;
                self.count(values.len())?;
                for value in values {
                    let wrapper = matches!(value, Tag::Compound(c) if c.len() == 1 && c.contains_key(&JavaString::default()));
                    if id == 10 && (value.id() != 10 || wrapper) {
                        // Emit one synthetic compound layer without cloning the tree.
                        if depth + 1 >= self.limits.max_depth {
                            return Err(self.error("depth limit exceeded"));
                        }
                        if self.nodes >= self.limits.max_nodes {
                            return Err(self.error("node limit exceeded"));
                        }
                        self.nodes += 1;
                        self.put(&[value.id()])?;
                        self.string(&JavaString::default())?;
                        self.payload(value, depth + 2)?;
                        self.put(&[0])?;
                    } else {
                        self.payload(value, depth + 1)?;
                    }
                }
            }
            Tag::Compound(values) => {
                for (name, value) in values {
                    if matches!(value, Tag::End) {
                        return Err(self.error("End cannot be a named value"));
                    }
                    self.put(&[value.id()])?;
                    self.string(name)?;
                    self.payload(value, depth + 1)?;
                }
                self.put(&[0])?;
            }
            Tag::IntArray(v) => {
                self.count(v.len())?;
                for item in v {
                    self.put(&item.to_be_bytes())?;
                }
            }
            Tag::LongArray(v) => {
                self.count(v.len())?;
                for item in v {
                    self.put(&item.to_be_bytes())?;
                }
            }
        }
        Ok(())
    }
}

/// `NbtIo.write`: a file root. A string too long for modified UTF-8 is
/// written empty, as vanilla's disk writer does; [`write_network`] refuses it.
pub fn write_named(root: &NamedTag, limits: Limits) -> Result<Vec<u8>, Error> {
    write_named_parts(&root.name, &root.tag, limits)
}

fn write_named_parts(name: &JavaString, tag: &Tag, limits: Limits) -> Result<Vec<u8>, Error> {
    let mut writer = Writer {
        bytes: Vec::new(),
        limits,
        nodes: 0,
        empty_long_strings: true,
        canonical_nan: true,
    };
    writer.put(&[tag.id()])?;
    if tag.id() != 0 {
        writer.string(name)?;
    }
    writer.payload(tag, 0)?;
    Ok(writer.bytes)
}

pub fn write_network(tag: &Tag, limits: Limits) -> Result<Vec<u8>, Error> {
    let mut writer = Writer {
        bytes: Vec::new(),
        limits,
        nodes: 0,
        empty_long_strings: false,
        canonical_nan: false,
    };
    writer.put(&[tag.id()])?;
    writer.payload(tag, 0)?;
    Ok(writer.bytes)
}

/// Validate an opaque native component against network limits and a stable disk round trip.
pub fn validate_component_value(value: &Tag) -> Result<(), String> {
    if matches!(value, Tag::End) {
        return Err("a component value cannot be NBT End".into());
    }
    let network = Limits::network();
    let bytes = write_network(
        value,
        Limits {
            max_bytes: network.max_bytes,
            ..Limits::default()
        },
    )
    .map_err(|error| format!("NBT: {error}"))?;
    read_network(&bytes, network).map_err(|error| format!("NBT: {error}"))?;
    let limits = Limits {
        max_bytes: network.max_bytes + 2,
        ..Limits::default()
    };
    // Disk NBT canonicalizes NaNs and normalizes negative zero when read.
    let saved = write_named_parts(&JavaString::default(), value, limits)
        .map_err(|error| error.to_string())?;
    let (loaded, _) = read_named(&saved, limits).map_err(|error| error.to_string())?;
    if loaded.tag != *value {
        return Err("component value changes on save and reload".into());
    }
    Ok(())
}

/// `NbtUtils.compareNbt(expected, actual, true)`.
pub fn compare_nbt(expected: &Tag, actual: Option<&Tag>) -> bool {
    let Some(actual) = actual else { return false };
    if std::mem::discriminant(expected) != std::mem::discriminant(actual) {
        return false;
    }
    match (expected, actual) {
        (Tag::Compound(e), Tag::Compound(a)) => {
            a.len() >= e.len() && e.iter().all(|(k, v)| compare_nbt(v, a.get(k)))
        }
        (Tag::List(e), Tag::List(a)) => {
            if e.is_empty() {
                return a.is_empty();
            }
            a.len() >= e.len() && e.iter().all(|x| a.iter().any(|y| compare_nbt(x, Some(y))))
        }
        _ => expected == actual,
    }
}

/// SNBT (`TagParser`): compounds, lists, typed arrays, quoted and bare
/// strings, booleans and suffixed numbers. Returns the tag and the bytes read.
pub fn parse_snbt(text: &str) -> Result<(Tag, usize), String> {
    let mut p = Snbt {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    let tag = p.value()?;
    Ok((tag, p.i))
}
/// Nesting deeper than NBT's own limit (`NbtAccounter`'s 512) is refused
/// rather than overflowing the stack.
const SNBT_MAX_DEPTH: usize = 512;
struct Snbt<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}
impl Snbt<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }
    fn bare(c: u8) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+')
    }
    fn string(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(q @ (b'"' | b'\'')) => {
                self.i += 1;
                let mut out = Vec::new();
                while let Some(&c) = self.s.get(self.i) {
                    self.i += 1;
                    if c == b'\\' {
                        if let Some(&n) = self.s.get(self.i) {
                            self.i += 1;
                            out.push(match n {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                    } else if c == q {
                        return String::from_utf8(out).map_err(|e| e.to_string());
                    } else {
                        out.push(c);
                    }
                }
                Err("unterminated string".into())
            }
            _ => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|&c| Self::bare(c)) {
                    self.i += 1;
                }
                if start == self.i {
                    return Err(format!("expected a value at {start}"));
                }
                Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
        }
    }
    fn value(&mut self) -> Result<Tag, String> {
        match self.peek() {
            Some(b'{') => {
                if self.depth >= SNBT_MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.i));
                }
                self.depth += 1;
                let tag = self.compound();
                self.depth -= 1;
                tag
            }
            Some(b'[') => {
                if self.depth >= SNBT_MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.i));
                }
                self.depth += 1;
                let tag = self.list();
                self.depth -= 1;
                tag
            }
            Some(b'"' | b'\'') => Ok(Tag::String(JavaString::from(self.string()?.as_str()))),
            _ => {
                let word = self.string()?;
                Ok(Self::scalar(&word))
            }
        }
    }
    fn compound(&mut self) -> Result<Tag, String> {
        self.i += 1;
        let mut map = Compound::new();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Tag::Compound(map));
        }
        loop {
            let key = self.string()?;
            self.expect(b':')?;
            map.insert(JavaString::from(key.as_str()), self.value()?);
            match self.peek() {
                // SnbtGrammar: an entry may be followed by a comma before the brace.
                Some(b',') => {
                    self.i += 1;
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        return Ok(Tag::Compound(map));
                    }
                }
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Tag::Compound(map));
                }
                _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
            }
        }
    }
    fn list(&mut self) -> Result<Tag, String> {
        self.i += 1;
        let typed = match (self.s.get(self.i), self.s.get(self.i + 1)) {
            (Some(&t @ (b'B' | b'I' | b'L')), Some(b';')) => {
                self.i += 2;
                Some(t)
            }
            _ => None,
        };
        let mut items = Vec::new();
        if self.peek() != Some(b']') {
            loop {
                items.push(self.value()?);
                match self.peek() {
                    // SnbtGrammar: a list or array may end with a comma.
                    Some(b',') => {
                        self.i += 1;
                        if self.peek() == Some(b']') {
                            break;
                        }
                    }
                    Some(b']') => break,
                    _ => return Err(format!("expected ',' or ']' at {}", self.i)),
                }
            }
        }
        self.i += 1;
        let long = |t: &Tag| match *t {
            Tag::Byte(v) => i64::from(v),
            Tag::Short(v) => i64::from(v),
            Tag::Int(v) => i64::from(v),
            Tag::Long(v) => v,
            _ => 0,
        };
        Ok(match typed {
            Some(b'B') => Tag::ByteArray(items.iter().map(|t| long(t) as u8).collect()),
            Some(b'I') => Tag::IntArray(items.iter().map(|t| long(t) as i32).collect()),
            Some(_) => Tag::LongArray(items.iter().map(long).collect()),
            None => Tag::List(items),
        })
    }
    /// A bare word: a typed number, a boolean, or else a string.
    /// `FloatTag.valueOf` / `DoubleTag.valueOf` canonicalize either zero sign.
    fn scalar(word: &str) -> Tag {
        match word {
            "true" => return Tag::Byte(1),
            "false" => return Tag::Byte(0),
            _ => {}
        }
        let lower = word.to_ascii_lowercase();
        let (body, suffix) = match lower.as_bytes().last() {
            Some(c @ (b'b' | b's' | b'l' | b'f' | b'd')) => (&word[..word.len() - 1], Some(*c)),
            _ => (word, None),
        };
        // Only digits (and an exponent) make a number: a word like `nan` or
        // `infd` that Rust's float parse would take is a string here.
        if body
            .bytes()
            .any(|c| c.is_ascii_alphabetic() && c != b'e' && c != b'E')
        {
            return Tag::String(JavaString::from(word));
        }
        let parsed = match suffix {
            Some(b'b') => body.parse::<i8>().ok().map(Tag::Byte),
            Some(b's') => body.parse::<i16>().ok().map(Tag::Short),
            Some(b'l') => body.parse::<i64>().ok().map(Tag::Long),
            Some(b'f') => body
                .parse::<f32>()
                .ok()
                .map(|v| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() })),
            Some(_) => body
                .parse::<f64>()
                .ok()
                .map(|v| Tag::Double(if v == 0.0 { 0 } else { v.to_bits() })),
            None if body.contains('.') || body.contains('e') || body.contains('E') => body
                .parse::<f64>()
                .ok()
                .map(|v| Tag::Double(if v == 0.0 { 0 } else { v.to_bits() })),
            None => body.parse::<i32>().ok().map(Tag::Int),
        };
        parsed.unwrap_or_else(|| Tag::String(JavaString::from(word)))
    }
}

/// A tag as its network NBT bytes (`write_network`), the form it crosses to a wasm mod in; a
/// tag past the default limits is refused, with the reason.
impl TryFrom<&Tag> for Vec<u8> {
    type Error = Error;
    fn try_from(tag: &Tag) -> Result<Self, Error> {
        write_network(tag, Limits::default())
    }
}

/// A tag read from its network NBT bytes (`read_network`).
impl TryFrom<Vec<u8>> for Tag {
    type Error = Error;
    fn try_from(bytes: Vec<u8>) -> Result<Self, Error> {
        read_network(&bytes, Limits::default()).map(|(tag, _)| tag)
    }
}

impl From<&Compound> for Tag {
    fn from(compound: &Compound) -> Self {
        Tag::Compound(compound.clone())
    }
}

/// The compound a compound tag holds; any other tag is refused as itself.
impl TryFrom<Tag> for Compound {
    type Error = Tag;
    fn try_from(tag: Tag) -> Result<Self, Tag> {
        match tag {
            Tag::Compound(compound) => Ok(compound),
            tag => Err(tag),
        }
    }
}

/// Vanilla 26.3 NbtAccounter.defaultQuota, used by ByteBufCodecs.TAG.
pub const NETWORK_NBT_QUOTA: u64 = 2_097_152;

impl Tag {
    /// Encode a nameless network root. End (also vanilla's null marker) is `[0]`.
    /// Uses the engine's network writer, preserving NaN bits; refuses depth over
    /// 512 and strings whose modified UTF-8 exceeds 65535 bytes.
    /// Vanilla applies heap quotas when reading, not writing.
    pub fn to_network_bytes(&self) -> Result<Vec<u8>, Error> {
        write_network(
            self,
            Limits {
                max_bytes: usize::MAX,
                max_nodes: usize::MAX,
                max_depth: 512,
            },
        )
    }

    /// Decode one nameless network root and return its consumed byte count.
    /// Trailing bytes remain unread. `None` uses vanilla's 2 MiB heap quota;
    /// `Some(quota)` supplies a caller quota in NbtAccounter bytes, not encoded
    /// bytes. Container nesting is limited to 512. End represents vanilla null.
    /// Malformed input, quota exhaustion and excessive depth return an Error.
    pub fn from_network_bytes(bytes: &[u8], quota: Option<u64>) -> Result<(Self, usize), Error> {
        let mut reader = Reader {
            bytes,
            offset: 0,
            limits: Limits {
                max_bytes: usize::MAX,
                max_nodes: usize::MAX,
                max_depth: 512,
            },
            nodes: 0,
            heap: Some((0, quota.unwrap_or(NETWORK_NBT_QUOTA))),
            stream: None,
            bounded: None,
            strict: false,
            entering: 0,
            rejected: 0,
        };
        read_network_with_reader(&mut reader)
    }
}

#[cfg(test)]
#[path = "nbt_network_tests.rs"]
mod network_tests;

/// A read position in a file's bytes for readers that decode a known layout
/// without building a `Tag` tree (structure templates). Each method runs what
/// the tree reader runs for the tags it reads, so a streamed read accepts
/// and rejects the same bytes as [`read_named`]; the caller walks the root
/// compound key by key and reads each value's payload in file order.
pub struct Cursor<'a> {
    reader: Reader<'a, 'static>,
}

/// A compound key read by [`Cursor::next_key`]. ASCII names (every name a
/// template uses) are compared as the file's bytes, so no key allocates;
/// other names go through the decoded string, so an overlong-encoded name
/// matches the keys [`read_named`] would match.
pub struct Key<'a> {
    bytes: &'a [u8],
    decoded: Option<JavaString>,
}
impl Key<'_> {
    pub fn is(&self, name: &str) -> bool {
        match &self.decoded {
            None => self.bytes == name.as_bytes(),
            Some(key) => with_key(name, |wanted| key == wanted),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl<'a> Cursor<'a> {
    /// The root of a file; `None` when it is not a compound (its payload is
    /// still read and checked, as [`read_named`] does).
    pub fn file(bytes: &'a [u8], limits: Limits) -> Result<Option<Self>, Error> {
        let mut reader = Reader {
            bytes,
            offset: 0,
            limits,
            nodes: 0,
            stream: None,
            heap: None,
            bounded: None,
            strict: false,
            entering: 0,
            rejected: 0,
        };
        let id = reader.byte()?;
        if id == 0 {
            reader.payload(id, 0)?;
            return Ok(None);
        }
        reader.root_name()?;
        if id != 10 {
            reader.payload(id, 0)?;
            return Ok(None);
        }
        let mut cursor = Self { reader };
        cursor.reader.entry(10, 0)?;
        Ok(Some(cursor))
    }
    /// The next key of the open compound and its value's tag type, `None` at
    /// the compound's end; the payload that follows is the caller's to read.
    pub fn next_key(&mut self) -> Result<Option<(Key<'a>, u8)>, Error> {
        let id = self.reader.byte()?;
        if id == 0 {
            return Ok(None);
        }
        let len = u16::from_be_bytes(self.reader.number()?) as usize;
        let bytes = self.reader.take(len)?;
        let decoded = if bytes.is_ascii() {
            None
        } else {
            Some(
                modified_utf8(bytes)
                    .ok_or_else(|| self.reader.error("invalid Java modified UTF-8"))?,
            )
        };
        Ok(Some((Key { bytes, decoded }, id)))
    }
    /// A whole tag payload, using the tree reader.
    pub fn tag(&mut self, id: u8, depth: usize) -> Result<Tag, Error> {
        self.reader.payload(id, depth)
    }
    /// A list element, unwrapped as `ListTag.addAndUnwrap` unwraps
    /// it: a compound with a single empty-named member is that member.
    pub fn element(&mut self, id: u8, depth: usize) -> Result<Tag, Error> {
        let mut value = self.reader.payload(id, depth)?;
        if let Tag::Compound(map) = &mut value
            && map.len() == 1
            && map.contains_key(&JavaString::default())
        {
            value = map.remove(&JavaString::default()).unwrap();
        }
        Ok(value)
    }
    /// A compound tag's entry checks (the caller saw type 10), then its keys.
    pub fn compound(&mut self, depth: usize) -> Result<(), Error> {
        self.reader.entry(10, depth)
    }
    /// A list tag's entry and header (the caller saw type 9).
    pub fn list(&mut self, depth: usize) -> Result<(u8, usize), Error> {
        self.reader.entry(9, depth)?;
        self.reader.list_header()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// SnbtGrammar: a compound, list or typed array may end with one comma; a
    /// comma alone or two in a row are errors.
    #[test]
    fn snbt_takes_one_trailing_comma() {
        let parse = |s: &str| parse_snbt(s).map(|(t, _)| t);
        assert_eq!(parse("{a:1b,}"), parse("{a:1b}"));
        assert_eq!(parse("[1, 2 ,]"), parse("[1,2]"));
        assert_eq!(parse("[I;1,2,]"), parse("[I;1,2]"));
        assert_eq!(parse("{a:[{},],}"), parse("{a:[{}]}"));
        for bad in ["{,}", "[,]", "[I;,]", "{a:1b,,}", "[1,,2]", "[1,,]"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
    /// `NbtIo.write` (`StringFallbackDataOutput`) writes a string too long for
    /// modified UTF-8 empty and goes on (#5064); the network writer refuses
    /// it. Vanilla 26.3's bytes for `{s: 32768 x U+0000}` (65536 encoded).
    #[test]
    fn the_disk_writer_empties_a_string_too_long_for_modified_utf8() {
        let long = Tag::String(JavaString(vec![0; 32768]));
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([("s", long.clone())]),
        };
        assert_eq!(
            write_named(&root, Limits::saved()).unwrap(),
            [0x0a, 0, 0, 0x08, 0, 1, b's', 0, 0, 0]
        );
        assert!(write_network(&root.tag, Limits::saved()).is_err());
        // The rest of the tree is kept.
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("before", Tag::String("kept".into())),
                ("long", long),
                ("after", Tag::Int(7)),
            ]),
        };
        let bytes = write_named(&root, Limits::saved()).unwrap();
        let back = read_named(&bytes, Limits::saved()).unwrap().0.tag;
        assert_eq!(back.get("before"), Some(&Tag::String("kept".into())));
        assert_eq!(back.get("long"), Some(&Tag::String("".into())));
        assert_eq!(back.get("after"), Some(&Tag::Int(7)));
    }
    /// `NbtIo.readUnnamedTag` skips the root's name unread
    /// (`StringTag.skipString`), so a name that isn't modified UTF-8 still
    /// loads (#5064). Vanilla 26.3 reads these bytes the same way.
    #[test]
    fn a_root_name_that_is_not_modified_utf8_is_skipped() {
        let hex = |s: &str| -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        };
        let (root, used) = read_named(&hex("0a0002ffff00"), Limits::saved()).unwrap();
        assert_eq!((root.tag, used), (Tag::Compound(Compound::new()), 6));
        let (root, _) = read_named(&hex("0a0002ffff030001780000000700"), Limits::saved()).unwrap();
        assert_eq!(root.tag, Tag::compound([("x", Tag::Int(7))]));
        assert_eq!(
            read_named(&hex("0a00047465737400"), Limits::saved())
                .unwrap()
                .0
                .name,
            "test".into()
        );
        // A name cut short, or a key inside that isn't modified UTF-8, still fails.
        assert!(read_named(&hex("0a0009ff"), Limits::saved()).is_err());
        assert!(read_named(&hex("0a000003000278ff0000000700"), Limits::saved()).is_err());
    }
    #[test]
    fn streamed_lists_reach_the_visitor_and_stay_empty_in_the_tree() {
        let list = |n: i32| Tag::List((0..n).map(Tag::Int).collect());
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("states", list(3)),
                ("inner", Tag::compound([("states", list(2))])),
                (
                    "deep",
                    Tag::compound([("inner", Tag::compound([("states", list(1))]))]),
                ),
                ("other", list(4)),
            ]),
        };
        let bytes = write_named(&root, Limits::default()).unwrap();
        let mut seen = Vec::new();
        let (read, used) = read_named_streaming(&bytes, Limits::default(), "states", &mut |l| {
            let mut values = vec![l.len() as i32];
            // Elements left unread are skipped.
            if let Some(Tag::Int(v)) = l.next_element()? {
                values.push(v);
            }
            seen.push(values);
            Ok(())
        })
        .unwrap();
        assert_eq!(used, bytes.len());
        // Compounds are written in key order: `inner` before `states`.
        assert_eq!(seen, [vec![2, 0], vec![3, 0]]);
        assert_eq!(read.tag.get("states"), Some(&Tag::List(Vec::new())));
        assert_eq!(
            read.tag.get("inner").unwrap().get("states"),
            Some(&Tag::List(Vec::new()))
        );
        // Only the root and its direct children stream.
        assert_eq!(read.tag.get("deep"), root.tag.get("deep"));
        assert_eq!(read.tag.get("other"), root.tag.get("other"));
        // A visitor's error stops the read.
        let failed = read_named_streaming(&bytes, Limits::default(), "states", &mut |l| {
            Err(l.error("refused"))
        });
        assert_eq!(failed.unwrap_err().reason, "refused");
    }
    #[test]
    fn wire_node_budget_includes_mixed_list_wrappers() {
        let tag = Tag::List(vec![Tag::Int(1), Tag::Byte(2)]);
        let limits = Limits {
            max_nodes: 5,
            ..Limits::default()
        };
        let bytes = write_network(&tag, limits).unwrap();
        assert_eq!(read_network(&bytes, limits).unwrap().0, tag);
        let limits = Limits {
            max_nodes: 4,
            ..limits
        };
        assert!(write_network(&tag, limits).is_err());
        assert!(read_network(&bytes, limits).is_err());
    }
    #[test]
    fn templates_read_past_the_default_node_cap() {
        // A big mod template (YUNG's monument start) holds more tags than the
        // default cap; vanilla reads templates with an unlimited accounter.
        let root = NamedTag {
            name: JavaString::default(),
            tag: Tag::compound([("blocks", Tag::List(vec![Tag::Byte(1); 1_000_001]))]),
        };
        let bytes = write_named(&root, Limits::templates()).unwrap();
        assert!(read_named(&bytes, Limits::default()).is_err());
        assert_eq!(read_named(&bytes, Limits::templates()).unwrap().0, root);
    }
    #[test]
    fn depth_boundary_and_every_truncated_prefix() {
        let mut tag = Tag::Int(1);
        for _ in 0..512 {
            tag = Tag::List(vec![tag]);
        }
        let bytes = write_network(&tag, Limits::default()).unwrap();
        assert_eq!(read_network(&bytes, Limits::default()).unwrap().0, tag);
        tag = Tag::List(vec![tag]);
        assert!(write_network(&tag, Limits::default()).is_err());
        let deeper = write_network(
            &tag,
            Limits {
                max_depth: 513,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(read_network(&deeper, Limits::default()).is_err());
        let short = Tag::compound([(
            "list",
            Tag::List(vec![Tag::Long(42), Tag::ByteArray(vec![1, 2, 3])]),
        )]);
        let bytes = write_network(&short, Limits::default()).unwrap();
        for end in 0..bytes.len() {
            assert!(read_network(&bytes[..end], Limits::default()).is_err());
        }
    }
    #[test]
    fn modified_utf_and_mixed_lists_preserve_java_values() {
        let tag = Tag::List(vec![
            Tag::Int(3),
            Tag::String(JavaString(vec![0, 0xd800, 0xdc00, 0xdfff])),
            Tag::compound([("", Tag::Int(9))]),
            Tag::compound([("x", Tag::Byte(1))]),
        ]);
        let data = write_network(&tag, Limits::default()).unwrap();
        assert_eq!(data[1], 10);
        assert_eq!(
            read_network(&data, Limits::default()).unwrap(),
            (tag, data.len())
        );
    }
    /// Modified UTF-8 one unit at a time, as `DataOutputStream.writeUTF` does.
    fn utf_units(units: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        for &c in units {
            match c {
                1..=127 => out.push(c as u8),
                0..=2047 => out.extend([0xc0 | (c >> 6) as u8, 0x80 | (c & 63) as u8]),
                _ => out.extend([
                    0xe0 | (c >> 12) as u8,
                    0x80 | ((c >> 6) & 63) as u8,
                    0x80 | (c & 63) as u8,
                ]),
            }
        }
        out
    }
    /// The one-byte strings read and written whole come out as the others:
    /// ASCII, a zero unit (two bytes), units past ASCII, alone and mixed; and
    /// a string one byte over the byte limit fails where it always failed.
    #[test]
    fn ascii_strings_read_and_write_as_every_other_string() {
        let strings: Vec<Vec<u16>> = vec![
            vec![],
            "minecraft:stone".encode_utf16().collect(),
            vec![0],
            vec![b'a'.into(), 0, b'b'.into()],
            vec![127, 128, 0x7ff, 0x800, 0xffff],
            "caf\u{e9}".encode_utf16().collect(),
            vec![0xd800, b'x'.into()],
            (0..3000).map(|i| 1 + (i % 127) as u16).collect(),
        ];
        for units in strings {
            let string = JavaString(units.clone());
            let encoded = utf_units(&units);
            assert_eq!(modified_utf8(&encoded), Some(string.clone()));
            let mut expected = vec![8, 0, 0];
            expected.extend((encoded.len() as u16).to_be_bytes());
            expected.extend(&encoded);
            let root = NamedTag {
                name: JavaString::default(),
                tag: Tag::String(string),
            };
            assert_eq!(write_named(&root, Limits::default()).unwrap(), expected);
            assert_eq!(read_named(&expected, Limits::default()).unwrap().0, root);
            // One byte short: the error is at the last write, a unit (or the
            // length of an empty string) that no longer fits.
            let last = units.last().map_or(2, |&c| utf_units(&[c]).len());
            let limits = Limits {
                max_bytes: expected.len() - 1,
                ..Limits::default()
            };
            let e = write_named(&root, limits).unwrap_err();
            assert_eq!(e.offset, expected.len() - last, "{units:?}");
        }
    }
    #[test]
    fn malformed_lengths_truncation_limits_and_utf_are_rejected() {
        for data in [
            &[7, 0xff, 0xff, 0xff, 0xff][..],
            &[9, 0, 0, 0, 0, 1],
            &[8, 0, 1, 0xc0],
            &[13],
            &[4, 0],
        ] {
            assert!(read_network(data, Limits::default()).is_err());
        }
        let tag = Tag::List(vec![Tag::List(vec![Tag::Int(1)])]);
        let bytes = write_network(&tag, Limits::default()).unwrap();
        assert!(
            read_network(
                &bytes,
                Limits {
                    max_depth: 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            read_network(
                &bytes,
                Limits {
                    max_nodes: 2,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            read_network(
                &bytes,
                Limits {
                    max_bytes: bytes.len() - 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            write_network(&Tag::String(JavaString(vec![0; 32768])), Limits::default()).is_err()
        );
    }
    #[test]
    fn duplicate_keys_last_wins_and_nan_output_is_canonical_on_disk() {
        let bytes = [10, 1, 0, 1, b'x', 1, 1, 0, 1, b'x', 2, 0];
        assert_eq!(
            read_network(&bytes, Limits::default()).unwrap().0.get("x"),
            Some(&Tag::Byte(2))
        );
        // Vanilla 26.3 (#5064): `FriendlyByteBuf.writeNbt` keeps a NaN's bits,
        // `NbtIo.write` writes the canonical NaN.
        let nan = |bits: u32| NamedTag {
            name: "".into(),
            tag: Tag::compound([("v", Tag::Float(bits))]),
        };
        assert_eq!(
            write_network(&Tag::Float(0xff80_0001), Limits::default()).unwrap(),
            [5, 0xff, 0x80, 0, 1]
        );
        assert_eq!(
            write_network(&Tag::Double(0x7ff0_0000_0000_0001), Limits::default()).unwrap(),
            [6, 0x7f, 0xf0, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(
            write_named(&nan(0x7f80_0001), Limits::default()).unwrap(),
            [10, 0, 0, 5, 0, 1, b'v', 0x7f, 0xc0, 0, 0, 0]
        );
        assert_eq!(
            write_network(&Tag::Double((-0.0_f64).to_bits()), Limits::default()).unwrap()[1],
            0x80
        );
    }
    #[test]
    fn snbt_parses_typed_values() {
        let text = "{a:1b,b:[I;1,2],c:'x y',d:[{e:2.5f}],f:true,g:id}";
        let (tag, used) = parse_snbt(&format!("{text}rest")).unwrap();
        assert_eq!(used, text.len());
        assert_eq!(tag.get("a"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("b"), Some(&Tag::IntArray(vec![1, 2])));
        assert_eq!(tag.get("c"), Some(&Tag::String("x y".into())));
        assert_eq!(tag.get("f"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("g"), Some(&Tag::String("id".into())));
        let Some(Tag::List(d)) = tag.get("d") else {
            panic!()
        };
        assert_eq!(d[0].get("e"), Some(&Tag::Float(2.5f32.to_bits())));
    }
    #[test]
    fn snbt_nesting_is_bounded() {
        let nest = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
        assert!(parse_snbt(&nest(512)).is_ok());
        assert!(parse_snbt(&nest(513)).is_err());
        // Far past any limit it errs instead of overflowing the stack.
        assert!(parse_snbt(&nest(100_000)).is_err());
    }
    #[test]
    fn snbt_bare_words_stay_strings() {
        for word in ["nan", "NaN", "inf", "infd", "NaNf", "Infinityd", "nand"] {
            assert_eq!(
                parse_snbt(word).unwrap().0,
                Tag::String(JavaString::from(word)),
                "{word}"
            );
        }
        assert_eq!(
            parse_snbt("1e5").unwrap().0,
            Tag::Double(100000f64.to_bits())
        );
        assert_eq!(parse_snbt("1.5f").unwrap().0, Tag::Float(1.5f32.to_bits()));
        assert_eq!(parse_snbt("3b").unwrap().0, Tag::Byte(3));
        assert_eq!(parse_snbt("-2L").unwrap().0, Tag::Long(-2));
    }

    /// A bounded entry with a key twice or past its budget is left out unbuilt; the rest of
    /// the file, the same key under another parent or a parent of the same name deeper down,
    /// and the input read plainly are as before.
    #[test]
    fn a_bounded_entry_over_its_budget_or_with_a_repeated_key_is_left_out() {
        let budget = Bounded {
            parents: &[&["p"]],
            key: "b",
            max_bytes: 16,
            max_depth: 1,
        };
        let key = |out: &mut Vec<u8>, id: u8, name: &str| {
            out.push(id);
            out.extend((name.len() as u16).to_be_bytes());
            out.extend(name.as_bytes());
        };
        // {<parent>:{a:1b, b:<b>, c:3b}}
        let file = |parent: &str, b: &dyn Fn(&mut Vec<u8>)| {
            let mut out = vec![10, 0, 0];
            key(&mut out, 10, parent);
            key(&mut out, 1, "a");
            out.push(1);
            b(&mut out);
            key(&mut out, 1, "c");
            out.extend([3, 0, 0]);
            out
        };
        let twice = |out: &mut Vec<u8>| {
            key(out, 10, "b");
            out.extend([1, 0, 1, b'x', 1, 1, 0, 1, b'x', 2, 0]);
        };
        let big = |out: &mut Vec<u8>| {
            key(out, 7, "b");
            out.extend(20i32.to_be_bytes());
            out.extend([0; 20]);
        };
        // A list whose count the whole file could hold, but its budget cannot.
        let long_list = |out: &mut Vec<u8>| {
            key(out, 9, "b");
            out.push(1);
            out.extend(64i32.to_be_bytes());
            out.extend([0; 64]);
        };
        for b in [&twice as &dyn Fn(&mut Vec<u8>), &big, &long_list] {
            let raw = file("p", b);
            let (named, used, rejected) =
                read_named_bounded(&raw, Limits::default(), budget).unwrap();
            assert_eq!((used, rejected), (raw.len(), 1));
            let p = named.tag.get("p").unwrap();
            assert_eq!(p.get("b"), None);
            assert_eq!(
                (p.get("a"), p.get("c")),
                (Some(&Tag::Byte(1)), Some(&Tag::Byte(3)))
            );
            assert!(read_named_bounded(&raw[..raw.len() - 3], Limits::default(), budget).is_err());
        }
        let elsewhere = file("q", &twice);
        let (named, _, rejected) =
            read_named_bounded(&elsewhere, Limits::default(), budget).unwrap();
        let (plain, _) = read_named(&elsewhere, Limits::default()).unwrap();
        assert_eq!(
            (named, rejected),
            (plain.clone(), 0),
            "another parent's `b` reads as before"
        );
        let x = plain
            .tag
            .get("q")
            .and_then(|q| q.get("b"))
            .and_then(|b| b.get("x"));
        assert_eq!(x, Some(&Tag::Byte(2)), "Java keeps the last");
        // {q:{p:{b:{x:1b, x:2b}}}}: a `p` below the root is not the path's `p`.
        let mut nested = vec![10, 0, 0];
        key(&mut nested, 10, "q");
        nested.extend(&file("p", &twice)[3..]);
        nested.push(0);
        let (named, _, rejected) = read_named_bounded(&nested, Limits::default(), budget).unwrap();
        let (plain, _) = read_named(&nested, Limits::default()).unwrap();
        assert_eq!(
            (named, rejected),
            (plain, 0),
            "only the root's `p` is bounded"
        );
    }
}
