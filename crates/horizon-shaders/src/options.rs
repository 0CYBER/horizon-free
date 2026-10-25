//! Switch options (spec 8.1), as far as program-enable expressions (3.3) need them: which are on
//! by default. A switch is `#define NAME` (on) or `//#define NAME` (off) with nothing after the
//! name but a comment, in any stage or included file of the pack. Packs keep their options in
//! included `.glsl` files and test them elsewhere (OQ-13), so the test the spec asks for
//! (`#ifdef NAME` / `#ifndef NAME`) may be in any file of the pack.

use std::collections::BTreeSet;

use crate::Pack;

const SOURCES: [&str; 8] = ["vsh", "fsh", "gsh", "csh", "tcs", "tes", "glsl", "inc"];

/// The switch options that are on by default.
pub fn switches_on(pack: &Pack) -> BTreeSet<String> {
    let (mut on, mut off, mut tested) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for path in pack.all_files() {
        let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
        if !SOURCES.contains(&ext) {
            continue;
        }
        let Some(text) = pack.read_text(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            let (commented, rest) = match line.strip_prefix("//") {
                Some(rest) => (true, rest.trim_start()),
                None => (false, line),
            };
            let Some((directive, rest)) = rest
                .strip_prefix('#')
                .map(str::trim_start)
                .and_then(|r| r.split_once(|c: char| c.is_whitespace()))
            else {
                continue;
            };
            let mut words = rest.split_whitespace();
            let Some(name) = words.next() else { continue };
            match directive {
                "ifdef" | "ifndef" if !commented => _ = tested.insert(name.to_owned()),
                "define" if words.next().is_none_or(|w| w.starts_with("//")) => {
                    let set = if commented { &mut off } else { &mut on };
                    set.insert(name.to_owned());
                }
                _ => {}
            }
        }
    }
    // Defined both ways is ambiguous (8.1): it keeps its default only where it is on everywhere.
    on.retain(|name| tested.contains(name) && !off.contains(name));
    on
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    #[test]
    fn switches_need_a_test_and_no_value() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/settings.glsl",
                    "  #define AO // ambient occlusion\n//#define BLOOM\n#define LEVEL 2 //[1 2]\n\
                     #define SPLIT\n#define UNTESTED\n",
                ),
                (
                    "shaders/world0/deferred.fsh",
                    "#ifdef AO\n#endif\n#ifndef BLOOM\n#endif\n#ifdef SPLIT\n#endif\n#ifdef LEVEL\n#endif\n",
                ),
                ("shaders/world-1/deferred.fsh", "//#define SPLIT\n"),
                ("shaders/shaders.properties", "#define NOT_SOURCE\n#ifdef NOT_SOURCE\n"),
            ]),
        )
        .unwrap();
        assert_eq!(switches_on(&pack), BTreeSet::from(["AO".to_owned()]));
    }
}
