//! Original MD5 implementation for vanilla's deterministic name-derived seeds.
//! Integer algorithm and vectors: https://www.rfc-editor.org/rfc/rfc1321
//! This is an internal seed transform, not an authentication API.
const K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
const SHIFT: [[u32; 4]; 4] = [
    [7, 12, 17, 22],
    [5, 9, 14, 20],
    [4, 11, 16, 23],
    [6, 10, 15, 21],
];
struct Md5 {
    state: [u32; 4],
    buffer: [u8; 64],
    used: usize,
    bytes: u64,
}
impl Md5 {
    fn new() -> Self {
        Self {
            state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            buffer: [0; 64],
            used: 0,
            bytes: 0,
        }
    }
    fn block(&mut self, block: &[u8; 64]) {
        let words = std::array::from_fn::<_, 16, _>(|i| {
            u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let [mut a, mut b, mut c, mut d] = self.state;
        for (i, constant) in K.iter().enumerate() {
            let (mix, index) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(mix)
                    .wrapping_add(*constant)
                    .wrapping_add(words[index])
                    .rotate_left(SHIFT[i / 16][i % 4]),
            );
            a = d;
            d = c;
            c = b;
            b = next;
        }
        for (old, value) in self.state.iter_mut().zip([a, b, c, d]) {
            *old = old.wrapping_add(value);
        }
    }
    fn update(&mut self, mut input: &[u8]) {
        self.bytes = self.bytes.wrapping_add(input.len() as u64);
        if self.used != 0 {
            let count = (64 - self.used).min(input.len());
            self.buffer[self.used..self.used + count].copy_from_slice(&input[..count]);
            self.used += count;
            input = &input[count..];
            if self.used != 64 {
                return;
            }
            let block = self.buffer;
            self.block(&block);
            self.used = 0;
        }
        while input.len() >= 64 {
            self.block(input[..64].try_into().unwrap());
            input = &input[64..];
        }
        self.buffer[..input.len()].copy_from_slice(input);
        self.used = input.len();
    }
    fn finish(mut self) -> [u8; 16] {
        let length = self.bytes.wrapping_mul(8).to_le_bytes();
        let count = if self.used < 56 {
            56 - self.used
        } else {
            120 - self.used
        };
        let mut padding = [0; 64];
        padding[0] = 0x80;
        self.update(&padding[..count]);
        self.update(&length);
        std::array::from_fn(|i| self.state[i / 4].to_le_bytes()[i % 4])
    }
}
fn seed(digest: [u8; 16]) -> [u64; 2] {
    [
        u64::from_be_bytes(digest[..8].try_into().unwrap()),
        u64::from_be_bytes(digest[8..].try_into().unwrap()),
    ]
}
pub fn seed_from_hash(name: &str) -> [u64; 2] {
    let mut digest = Md5::new();
    digest.update(name.as_bytes());
    seed(digest.finish())
}
pub fn seed_from_hash_utf16(name: &[u16]) -> [u64; 2] {
    let mut digest = Md5::new();
    for point in char::decode_utf16(name.iter().copied()) {
        let mut bytes = [0; 4];
        // Java UTF-8 encoding replaces each malformed surrogate with '?'.
        digest.update(point.unwrap_or('?').encode_utf8(&mut bytes).as_bytes());
    }
    seed(digest.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_vectors_and_streaming_padding_boundaries() {
        for (input, expected) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            let mut digest = Md5::new();
            for chunk in input.as_bytes().chunks(3) {
                digest.update(chunk);
            }
            let actual: String = digest.finish().iter().map(|v| format!("{v:02x}")).collect();
            assert_eq!(actual, expected);
        }
        assert_eq!(
            seed_from_hash_utf16(&[0xd800, 0x61, 0xdc00]),
            seed_from_hash("?a?")
        );
        assert_eq!(
            seed_from_hash_utf16(&[0xd83d, 0xde00]),
            seed_from_hash("😀")
        );
    }
}
