use horizon_java_properties::Properties;

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

// Java guarantees that what `store` writes, `load` reads back as the same properties.
#[test]
fn store_and_load_round_trip() {
    let alphabet: Vec<char> = (0u32..0x300)
        .filter_map(char::from_u32)
        .chain(['\u{4e00}', '\u{1f600}', '\u{10ffff}'])
        .collect();
    let mut rng = Rng(0x243f6a8885a308d3);
    for _ in 0..2000 {
        let mut p = Properties::default();
        for _ in 0..rng.below(4) + 1 {
            let len = rng.below(6);
            let mut key = String::new();
            let mut value = String::new();
            for _ in 0..len {
                key.push(alphabet[rng.below(alphabet.len())]);
            }
            for _ in 0..rng.below(6) {
                value.push(alphabet[rng.below(alphabet.len())]);
            }
            p.put(&key, value);
        }
        // `store` with escapes off is the Writer variant: the text goes out as characters, and
        // `load` reads those same characters. With escapes on it is the OutputStream variant, whose
        // bytes are ISO-8859-1.
        for esc_unicode in [false, true] {
            let text = p.store(None, "DATE", esc_unicode);
            let back = if esc_unicode {
                Properties::load_latin1(text.as_bytes()).expect("stored text loads")
            } else {
                Properties::load(&text.chars().collect::<Vec<_>>()).expect("stored text loads")
            };
            assert_eq!(back.map, p.map, "esc_unicode={esc_unicode} text={text:?}");
        }
    }
}
