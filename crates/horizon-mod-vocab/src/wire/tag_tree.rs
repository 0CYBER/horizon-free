//! Stack-independent traversal of the vanilla NBT tree in the shared Wire format.
use super::{Reader, WireError, Writer};
use crate::Tag;
use horizon_core::nbt::{Compound, JavaString, discard_tag};

const CONTAINERS: usize = 512;

enum WriteFrame<'a> {
    List(std::slice::Iter<'a, Tag>),
    Map(std::collections::btree_map::Iter<'a, JavaString, Tag>),
}

pub(super) fn put(tag: &Tag, out: &mut Writer) {
    // One iterator per ancestor, never one allocation per value or one slot
    // per sibling. Even a wide compound uses only its bounded ancestor stack.
    let mut frames: [Option<WriteFrame<'_>>; CONTAINERS] = [const { None }; CONTAINERS];
    let mut len = 0;
    let mut current = tag;
    loop {
        let tag = current;
        if matches!(tag, Tag::List(_) | Tag::Compound(_)) && len >= CONTAINERS {
            out.fail(WireError::Depth);
            return;
        }
        match tag {
            Tag::End => out.put(&0u32),
            Tag::Byte(v) => {
                out.put(&1u32);
                out.put(v);
            }
            Tag::Short(v) => {
                out.put(&2u32);
                out.put(v);
            }
            Tag::Int(v) => {
                out.put(&3u32);
                out.put(v);
            }
            Tag::Long(v) => {
                out.put(&4u32);
                out.put(v);
            }
            Tag::Float(v) => {
                out.put(&5u32);
                out.put(v);
            }
            Tag::Double(v) => {
                out.put(&6u32);
                out.put(v);
            }
            Tag::ByteArray(v) => {
                out.put(&7u32);
                out.put(v);
            }
            Tag::String(v) => {
                out.put(&8u32);
                out.put(v);
            }
            Tag::List(v) => {
                out.put(&9u32);
                out.put_len(v.len());
                frames[len] = Some(WriteFrame::List(v.iter()));
                len += 1;
            }
            Tag::Compound(v) => {
                out.put(&10u32);
                out.put_len(v.len());
                frames[len] = Some(WriteFrame::Map(v.iter()));
                len += 1;
            }
            Tag::IntArray(v) => {
                out.put(&11u32);
                out.put(v);
            }
            Tag::LongArray(v) => {
                out.put(&12u32);
                out.put(v);
            }
        }
        if out.error().is_some() {
            return;
        }
        loop {
            if len == 0 {
                return;
            }
            let next = match frames[len - 1].as_mut().unwrap() {
                WriteFrame::List(values) => values.next(),
                WriteFrame::Map(values) => values.next().map(|(key, value)| {
                    out.put(key);
                    value
                }),
            };
            if let Some(next) = next {
                current = next;
                break;
            }
            len -= 1;
            frames[len] = None;
        }
    }
}

enum Frame {
    List(usize, Vec<Tag>),
    Map(usize, Compound, JavaString),
}

pub(super) fn take(input: &mut Reader<'_>) -> Result<Tag, WireError> {
    let mut frames = Vec::new();
    let result = (|| {
        loop {
            let code = input.take::<u32>()?;
            if (code == 9 || code == 10) && frames.len() >= CONTAINERS {
                return Err(WireError::Depth);
            }
            let mut value = match code {
                0 => Tag::End,
                1 => Tag::Byte(input.take()?),
                2 => Tag::Short(input.take()?),
                3 => Tag::Int(input.take()?),
                4 => Tag::Long(input.take()?),
                5 => Tag::Float(input.take()?),
                6 => Tag::Double(input.take()?),
                7 => Tag::ByteArray(input.take()?),
                8 => Tag::String(input.take()?),
                9 => {
                    let len = input.take_len(4)?;
                    if len != 0 {
                        frames.push(Frame::List(len, Vec::with_capacity(len)));
                        continue;
                    }
                    Tag::List(Vec::new())
                }
                10 => {
                    let len = input.take_len(8)?;
                    if len != 0 {
                        let key = input.take()?;
                        frames.push(Frame::Map(len, Compound::new(), key));
                        continue;
                    }
                    Tag::Compound(Compound::new())
                }
                11 => Tag::IntArray(input.take()?),
                12 => Tag::LongArray(input.take()?),
                other => return Err(WireError::Tag(other)),
            };
            loop {
                match frames.last_mut() {
                    None => return Ok(value),
                    Some(Frame::List(left, values)) => {
                        values.push(value);
                        *left -= 1;
                        if *left != 0 {
                            break;
                        }
                        let Some(Frame::List(_, values)) = frames.pop() else {
                            unreachable!()
                        };
                        value = Tag::List(values);
                    }
                    Some(Frame::Map(left, values, key)) => {
                        if let Some(old) = values.insert(std::mem::take(key), value) {
                            discard_tag(old);
                        }
                        *left -= 1;
                        if *left != 0 {
                            *key = input.take()?;
                            break;
                        }
                        let Some(Frame::Map(_, values, _)) = frames.pop() else {
                            unreachable!()
                        };
                        value = Tag::Compound(values);
                    }
                }
            }
        }
    })();
    for frame in frames {
        match frame {
            Frame::List(_, values) => discard_tag(Tag::List(values)),
            Frame::Map(_, values, _) => discard_tag(Tag::Compound(values)),
        }
    }
    result
}
