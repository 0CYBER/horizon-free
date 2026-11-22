//! The filter's normaliser turns any message into runs and word ranges. Whatever a player types, the
//! ranges must stay inside the runs: a message is not a place to panic, and a wrong range would match
//! against another word's letters.

use horizon_chat_filter::Text;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The characters the normaliser folds: letters, digits, leetspeak, separators, invisibles, and
/// whatever else a keyboard can produce.
const POOL: &[char] = &[
    'a',
    'b',
    'g',
    'i',
    'n',
    'z',
    'A',
    'Z',
    '0',
    '1',
    '3',
    '@',
    '!',
    '-',
    '.',
    '\'',
    ' ',
    '\t',
    '\n',
    '\u{0}',
    '\u{200b}',
    '\u{feff}',
    '\u{130}',
    '\u{3b1}',
    '\u{410}',
    '\u{ff41}',
    '\u{1d400}',
    '\u{24d0}',
    '\u{ff10}',
    'é',
    'ß',
    '\u{1f600}',
    '\u{10ffff}',
    '\u{1d552}',
];

fn random_text(rng: &mut Rng) -> String {
    (0..rng.below(24))
        .map(|_| {
            if rng.below(4) == 0 {
                char::from_u32(rng.next() as u32 % 0x11_0000).unwrap_or('a')
            } else {
                POOL[rng.below(POOL.len())]
            }
        })
        .collect()
}

#[test]
fn every_range_stays_inside_the_runs() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for _ in 0..5000 {
        let text = random_text(&mut rng);
        let t = Text::new(&text);
        for &(start, end) in &t.words {
            assert!(
                start < end && end <= t.runs.len(),
                "{start}..{end} outside {} runs for {text:?}",
                t.runs.len()
            );
        }
        assert!(
            t.words.windows(2).all(|w| w[0].1 <= w[1].0),
            "overlapping words for {text:?}"
        );
        for &(start, end) in &t.spaced {
            assert!(
                start < end && end <= t.runs.len(),
                "spaced {start}..{end} outside {} runs for {text:?}",
                t.runs.len()
            );
        }
        // One start per word, in the joined text and in order: a matcher over `joined` walks them.
        assert_eq!(t.joined_at.len(), t.words.len(), "joined_at for {text:?}");
        assert!(
            t.joined_at.windows(2).all(|w| w[0] <= w[1]),
            "joined_at not in order for {text:?}"
        );
        assert!(
            t.joined_at
                .last()
                .is_none_or(|&last| last <= t.joined.len()),
            "joined_at past the joined text for {text:?}"
        );
    }
}
