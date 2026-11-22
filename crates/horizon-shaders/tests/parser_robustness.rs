//! Shader packs come from players, modpack authors and the shader registry, so what their config files
//! ask for is untrusted: a condition with a division by zero, an absurd number or a broken escape must
//! come back as an error or a value, never as a panic in the middle of a frame.

use horizon_shaders::{Lang, Macros, Properties, eval_condition};

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

const TOKENS: &[&str] = &[
    "0",
    "1",
    "-1",
    "2147483647",
    "-2147483648",
    "9223372036854775807",
    "18446744073709551616",
    "0.0",
    ".",
    "1e999",
    "(",
    ")",
    " ",
    "+",
    "-",
    "*",
    "/",
    "%",
    "!",
    "&&",
    "||",
    "==",
    "!=",
    "<",
    ">",
    "<=",
    ">=",
    "~",
    "&",
    "|",
    "^",
    "<<",
    ">>",
    "?",
    ":",
    "true",
    "false",
    "MC_VERSION",
    "1/0",
    "x",
    "\\",
    "\"",
    "'",
    "\n",
    "\u{0}",
];

fn random_expression(rng: &mut Rng) -> String {
    (0..1 + rng.below(10))
        .map(|_| TOKENS[rng.below(TOKENS.len())])
        .collect()
}

/// Lines that a shader pack's `*.properties` hold, whole and cut in half.
const LINES: &[&str] = &[
    "screen = A",
    "screen !A",
    "screen . A = .0",
    "block.1=minecraft:stone 0",
    "block.1 = stone:0",
    "sliders=1",
    "sliders=3",
    "profile.PROFILE=minecraft:stone",
    "/*",
    "*/",
    "=",
    " = ",
    "a=b\n\n\nc",
    "///",
    "#",
    "!",
    "\u{0}",
    "a=\u{0}",
    "a=b\\",
    "clouds=off",
    "oldLighting=fAlSe",
    "moonPhases=0 1 2 3 4 5 6 7",
    "moonPhases=0 1 2 3 x",
    "sun=fixed 0 0 0 0",
    "vignette=BIOME",
    "vignette=only",
    "fogMode=off",
    "weather=0.5",
    "particles=off",
    "separateAo=false",
    "underwaterOverlay=true",
    "frustum.culling=false",
    "dynamicHandLight=true",
    "shadowTerrain=false",
];

#[test]
fn conditions_never_panic() {
    let macros = Macros::default();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..20000 {
        let expression = random_expression(&mut rng);
        let _ = eval_condition(&expression, &macros);
    }
}

#[test]
fn pack_files_never_panic() {
    let macros = Macros::default();
    for line in LINES {
        let _ = Properties::parse(line, &macros);
        let _ = Lang::parse(line);
    }
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for _ in 0..5000 {
        let mut text = String::new();
        for _ in 0..rng.below(8) {
            let line = LINES[rng.below(LINES.len())];
            // Half a line too: a pack file read up to a limit ends mid-line.
            let cut = rng.below(line.len() + 1);
            text.push_str(&line[..cut]);
            text.push('\n');
        }
        let _ = Properties::parse(&text, &macros);
        let _ = Lang::parse(&text);
    }
}
