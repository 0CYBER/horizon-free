//! `.properties` files: `shaders.properties` (spec 3.1) and the id files `block.properties`,
//! `item.properties`, `entity.properties` (4.1). `key=value` lines, `#` comments, a trailing `\`
//! continuing a value on the next line, and conditional directives evaluated with the
//! standard and option macros (1.5 item 2). `#define` and `#undef` work in every one of them:
//! a public pack defines a macro in its `shaders.properties` and tests it further down.
//! No other escape is read: values are names, numbers and expressions. A later duplicate key
//! replaces the earlier one, with a note.

use crate::{Macros, eval_condition};

/// One `key=value`, with the line it starts on (from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: String,
    pub line: u32,
}

/// Something wrong with a line, for the pack's log.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Default, PartialEq)]
pub struct Properties {
    /// In file order; a key appears once (a later duplicate replaces the earlier one).
    pub entries: Vec<Entry>,
    pub notes: Vec<Note>,
}

/// One branch level of `#if` ... `#endif`.
struct Branch {
    /// This branch's lines are read.
    on: bool,
    /// A branch of this `#if` was already taken (the rest are skipped).
    taken: bool,
    /// The enclosing level is read.
    outer: bool,
    seen_else: bool,
}

impl Properties {
    /// Reads `text`; `#define`/`#undef` change the macros for the lines after them only.
    pub fn parse(text: &str, macros: &Macros) -> Self {
        let mut out = Self::default();
        let mut macros = macros.clone();
        let mut branches: Vec<Branch> = Vec::new();
        let mut lines = text.lines().zip(1u32..);
        while let Some((raw, number)) = lines.next() {
            let line = raw.trim();
            let on = branches.last().is_none_or(|b| b.on);
            if let Some(directive) = line.strip_prefix('#') {
                out.directive(directive.trim_start(), number, &mut branches, &mut macros);
                continue;
            }
            // Continuation lines belong to their entry even inside a skipped branch.
            let mut logical = line.to_owned();
            while ends_with_continuation(&logical) {
                logical.pop();
                match lines.next() {
                    Some((next, _)) => logical.push_str(next.trim()),
                    None => break,
                }
            }
            if !on || logical.is_empty() {
                continue;
            }
            match logical.split_once('=') {
                Some((key, value)) if !key.trim().is_empty() => out.insert(Entry {
                    key: key.trim().to_owned(),
                    value: value.trim().to_owned(),
                    line: number,
                }),
                _ => out.note(number, "not a key=value line"),
            }
        }
        if !branches.is_empty() {
            out.note(text.lines().count() as u32, "`#if` without `#endif`");
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entry(key).map(|e| e.value.as_str())
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    fn insert(&mut self, entry: Entry) {
        if let Some(at) = self.entries.iter().position(|e| e.key == entry.key) {
            let first = self.entries.remove(at).line;
            self.note(
                entry.line,
                &format!(
                    "`{}` is set again (first on line {first}); this one counts",
                    entry.key
                ),
            );
        }
        self.entries.push(entry);
    }

    fn note(&mut self, line: u32, message: &str) {
        self.notes.push(Note {
            line,
            message: message.to_owned(),
        });
    }

    /// A line starting with `#`. It is a directive only when its first word is one; anything
    /// else (`# text`, `#commented.key=1`, `#ifdefault`) is a comment.
    fn directive(
        &mut self,
        line: &str,
        number: u32,
        branches: &mut Vec<Branch>,
        macros: &mut Macros,
    ) {
        let word_len = line
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(line.len());
        let (word, rest) = line.split_at(word_len);
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with("//")) {
            return;
        }
        let rest = rest.split("//").next().unwrap_or("").trim();
        let on = branches.last().is_none_or(|b| b.on);
        match word {
            "if" | "ifdef" | "ifndef" => {
                let value = on && self.condition(word, rest, number, macros);
                branches.push(Branch {
                    on: value,
                    taken: value,
                    outer: on,
                    seen_else: false,
                });
            }
            "elif" | "else" => {
                let Some(b) = branches.last_mut() else {
                    return self.note(number, &format!("`#{word}` without `#if`"));
                };
                if b.seen_else {
                    return self.note(number, &format!("`#{word}` after `#else`"));
                }
                let take = b.outer && !b.taken;
                b.seen_else = word == "else";
                b.on = take && (word == "else" || self.condition("if", rest, number, macros));
                b.taken |= b.on;
            }
            "endif" => {
                if branches.pop().is_none() {
                    self.note(number, "`#endif` without `#if`");
                }
            }
            "define" if on => {
                let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                macros.define(name, value.trim());
            }
            "undef" if on => macros.undef(rest),
            _ => {}
        }
    }

    fn condition(&mut self, word: &str, rest: &str, number: u32, macros: &Macros) -> bool {
        let name = rest.split_whitespace().next().unwrap_or("");
        let result = match word {
            "ifdef" => Ok(macros.is_defined(name) as i64),
            "ifndef" => Ok(!macros.is_defined(name) as i64),
            _ => eval_condition(rest, macros),
        };
        result.unwrap_or_else(|e| {
            self.note(number, &format!("`#{word} {rest}`: {e}; read as false"));
            0
        }) != 0
    }
}

/// An odd number of trailing backslashes: the last one joins the next line (`\\` is a literal).
fn ends_with_continuation(line: &str) -> bool {
    line.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Properties {
        let mut m = Macros::standard();
        m.define("SHADOWS", "");
        m.define("QUALITY", "2");
        Properties::parse(text, &m)
    }

    fn pairs(p: &Properties) -> Vec<(&str, &str)> {
        p.entries
            .iter()
            .map(|e| (e.key.as_str(), e.value.as_str()))
            .collect()
    }

    #[test]
    fn keys_values_and_comments() {
        let p =
            read("# a comment\n\n  sun = false\n#commented.key=1\nscreen=A B [C]\r\nbad line\n");
        assert_eq!(pairs(&p), [("sun", "false"), ("screen", "A B [C]")]);
        assert_eq!(p.entry("screen").unwrap().line, 5);
        assert_eq!(
            p.notes,
            [Note {
                line: 6,
                message: "not a key=value line".into()
            }]
        );
    }

    #[test]
    fn continuation_lines_join() {
        let p = read("screen=A B \\\n    C D\\\n  E\nsliders=X\nliteral=a\\\\\nnext=1");
        assert_eq!(
            pairs(&p),
            [
                ("screen", "A B C DE"),
                ("sliders", "X"),
                ("literal", "a\\\\"),
                ("next", "1")
            ]
        );
        assert_eq!(p.entry("sliders").unwrap().line, 4);
    }

    #[test]
    fn conditionals_pick_one_branch() {
        let p = read(
            "#if MC_VERSION >= 11300\nnew=1\n#else\nold=1\n#endif\n\
             #ifdef SHADOWS\n#if QUALITY == 1\nq=1\n#elif QUALITY == 2\nq=2\n#elif QUALITY >= 2\nq=3\n#endif\n#endif\n\
             #ifndef SHADOWS\nnone=1\n#elif 0\nnever=1\n#else\nfallback=1\n#endif\n#   ifdef SHADOWS // trailing comment\nspaced=1\n#endif",
        );
        assert_eq!(
            pairs(&p),
            [("new", "1"), ("q", "2"), ("fallback", "1"), ("spaced", "1")]
        );
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn comment_lines_are_not_directives() {
        let p = read("#ifdefault=1\n#iffy\n#endiff\n#else-where\nkey=1");
        assert_eq!(pairs(&p), [("key", "1")]);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn defines_apply_to_the_lines_after_them() {
        let text = "#ifdef MC_OS_NONE\n#define WATER 0\n#endif\n#define WATER 8\n\
                    #if WATER == 8\nblock.8=minecraft:water\n#endif\n#undef WATER\n#ifdef WATER\nno=1\n#endif";
        let p = Properties::parse(text, &Macros::default());
        assert_eq!(pairs(&p), [("block.8", "minecraft:water")]);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn duplicate_keys_keep_the_last() {
        let p = read("a=1\nb=2\na=3");
        assert_eq!(pairs(&p), [("b", "2"), ("a", "3")]);
        assert_eq!(p.notes[0].line, 3);
    }

    #[test]
    fn broken_conditionals_are_noted() {
        let p = read("#endif\n#else\n#if 1 +\nx=1\n#endif\n#if 1\n#else\n#else\n");
        let lines: Vec<u32> = p.notes.iter().map(|n| n.line).collect();
        assert_eq!(lines, [1, 2, 3, 8, 8]);
        assert!(p.entries.is_empty());
    }
}
