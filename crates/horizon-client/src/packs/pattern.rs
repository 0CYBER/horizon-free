//! A small regular-expression subset for `pack.mcmeta` filter blocks (Java
//! `Pattern.asPredicate()` semantics: an unanchored search). Supports
//! literals, `.`, classes `[a-z_]`/`[^..]`, escapes, `* + ?` quantifiers,
//! `^`/`$` and top-level alternation with `|`.

#[derive(Clone, Debug)]
enum Atom {
    Any,
    Lit(char),
    Class(Vec<(char, char)>, bool),
    Start,
    End,
}
#[derive(Clone, Debug)]
struct Item {
    atom: Atom,
    min: usize,
    max: usize,
}
#[derive(Clone, Debug)]
pub struct Pattern(Vec<Vec<Item>>);

impl Pattern {
    pub fn new(src: &str) -> Option<Pattern> {
        let mut alternatives = vec![Vec::new()];
        let chars: Vec<char> = src.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let atom = match chars[i] {
                '|' => {
                    alternatives.push(Vec::new());
                    i += 1;
                    continue;
                }
                '.' => Atom::Any,
                '^' => Atom::Start,
                '$' => Atom::End,
                '\\' => {
                    i += 1;
                    match *chars.get(i)? {
                        'd' => Atom::Class(vec![('0', '9')], false),
                        'w' => {
                            Atom::Class(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], false)
                        }
                        c => Atom::Lit(c),
                    }
                }
                '[' => {
                    i += 1;
                    let negate = chars.get(i) == Some(&'^');
                    if negate {
                        i += 1;
                    }
                    let mut ranges = Vec::new();
                    loop {
                        let mut c = *chars.get(i)?;
                        if c == ']' && !ranges.is_empty() {
                            break;
                        }
                        if c == '\\' {
                            i += 1;
                            c = *chars.get(i)?;
                        }
                        if chars.get(i + 1) == Some(&'-')
                            && chars.get(i + 2).is_some_and(|e| *e != ']')
                        {
                            ranges.push((c, chars[i + 2]));
                            i += 3;
                        } else {
                            ranges.push((c, c));
                            i += 1;
                        }
                    }
                    Atom::Class(ranges, negate)
                }
                '(' | ')' | '{' | '}' => return None,
                c => Atom::Lit(c),
            };
            i += 1;
            let (min, max) = match chars.get(i) {
                Some('*') => (0, usize::MAX),
                Some('+') => (1, usize::MAX),
                Some('?') => (0, 1),
                _ => (1, 1),
            };
            if (min, max) != (1, 1) {
                i += 1;
            }
            alternatives.last_mut()?.push(Item { atom, min, max });
        }
        Some(Pattern(alternatives))
    }
    pub fn is_match(&self, text: &str) -> bool {
        let text: Vec<char> = text.chars().collect();
        self.0
            .iter()
            .any(|alt| (0..=text.len()).any(|start| run(alt, &text, start)))
    }
}
fn one(atom: &Atom, c: char) -> bool {
    match atom {
        Atom::Any => c != '\n',
        Atom::Lit(l) => *l == c,
        Atom::Class(r, negate) => r.iter().any(|(lo, hi)| *lo <= c && c <= *hi) != *negate,
        Atom::Start | Atom::End => false,
    }
}
fn run(items: &[Item], text: &[char], at: usize) -> bool {
    let Some((item, rest)) = items.split_first() else {
        return true;
    };
    match item.atom {
        Atom::Start => return at == 0 && run(rest, text, at),
        Atom::End => return at == text.len() && run(rest, text, at),
        _ => {}
    }
    let mut count = 0;
    while count < item.max && at + count < text.len() && one(&item.atom, text[at + count]) {
        count += 1;
    }
    (item.min..=count).rev().any(|n| run(rest, text, at + n))
}

#[cfg(test)]
mod tests {
    use super::Pattern;
    fn m(p: &str, t: &str) -> bool {
        Pattern::new(p).unwrap().is_match(t)
    }
    #[test]
    fn java_find_semantics() {
        assert!(m("stone", "minecraft/stone_bricks"));
        assert!(!m("^stone$", "stone_bricks"));
        assert!(m("^textures/.*\\.png$", "textures/block/a.png"));
        assert!(!m("^textures/.*\\.png$", "textures/block/a_png"));
        assert!(m("[a-c]+z", "xxabcz"));
        assert!(!m("^[^a]+$", "bab"));
        assert!(m("dirt|grass", "grass_block"));
        assert!(m(".*", ""));
        assert!(Pattern::new("(a)").is_none());
    }
}
