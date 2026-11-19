//! The multiplayer server list, stored like vanilla's ServerList in an
//! uncompressed `servers.dat`: `{servers: [{name, ip, icon?, acceptTextures?,
//! acceptedCodeOfConduct?, hidden}]}`. Hidden entries (servers joined by
//! Direct Connection or quick play) are not shown but keep their settings.
use horizon_core::{
    nbt,
    nbt::{JavaString, NamedTag, Tag},
};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct ServerEntry {
    pub name: String,
    pub ip: String,
    /// Base64 PNG favicon from the last status ping.
    pub icon: Option<String>,
    /// ServerPackStatus: None = prompt, Some(true) enabled, Some(false) disabled.
    pub accept_textures: Option<bool>,
    /// `String.hashCode` of the Code of Conduct accepted for good (ServerData's
    /// `acceptedCodeOfConduct`); 0 for none.
    pub accepted_code_of_conduct: i32,
}
impl ServerEntry {
    pub fn new(name: &str, ip: &str) -> Self {
        Self {
            name: name.to_owned(),
            ip: ip.to_owned(),
            icon: None,
            accept_textures: None,
            accepted_code_of_conduct: 0,
        }
    }
    fn to_tag(&self) -> Tag {
        let mut entries = vec![
            ("name", Tag::String(JavaString::from(self.name.as_str()))),
            ("ip", Tag::String(JavaString::from(self.ip.as_str()))),
        ];
        if let Some(icon) = &self.icon {
            entries.push(("icon", Tag::String(JavaString::from(icon.as_str()))));
        }
        if let Some(accept) = self.accept_textures {
            entries.push(("acceptTextures", Tag::Byte(i8::from(accept))));
        }
        if self.accepted_code_of_conduct != 0 {
            entries.push((
                "acceptedCodeOfConduct",
                Tag::Int(self.accepted_code_of_conduct),
            ));
        }
        Tag::compound(entries)
    }
    fn from_tag(tag: &Tag) -> Option<Self> {
        let string = |k: &str| match tag.get(k) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        Some(Self {
            name: string("name").unwrap_or_default(),
            ip: string("ip")?,
            icon: string("icon"),
            accept_textures: match tag.get("acceptTextures") {
                Some(Tag::Byte(b)) => Some(*b != 0),
                _ => None,
            },
            accepted_code_of_conduct: match tag.get("acceptedCodeOfConduct") {
                Some(Tag::Int(hash)) => *hash,
                _ => 0,
            },
        })
    }
}

/// Held while servers.dat is read and written back, so a ping storing an icon and a list
/// saved by the screen or at startup don't undo each other's change.
pub fn lock() -> std::sync::MutexGuard<'static, ()> {
    static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SAVING.lock().unwrap_or_else(|e| e.into_inner())
}

/// ServerList.load: the shown entries; missing or unreadable files give an empty list.
pub fn load(path: &Path) -> Vec<ServerEntry> {
    load_all(path).0
}
/// ServerList.load: the shown entries (`serverList`) and the hidden ones
/// (`hiddenServerList`, `hidden` set), each in the file's order.
pub fn load_all(path: &Path) -> (Vec<ServerEntry>, Vec<ServerEntry>) {
    let (mut shown, mut hidden) = (Vec::new(), Vec::new());
    let Ok(bytes) = std::fs::read(path) else {
        return (shown, hidden);
    };
    let Ok((root, _)) = nbt::read_named(&bytes, nbt::Limits::default()) else {
        return (shown, hidden);
    };
    if let Some(Tag::List(list)) = root.tag.get("servers") {
        for t in list {
            let Some(entry) = ServerEntry::from_tag(t) else {
                continue;
            };
            // CompoundTag.getBooleanOr("hidden", false): any number but zero.
            let is_hidden = match t.get("hidden") {
                Some(Tag::Byte(b)) => *b != 0,
                Some(Tag::Short(n)) => *n as i8 != 0,
                Some(Tag::Int(n)) => *n as i8 != 0,
                Some(Tag::Long(n)) => *n as i8 != 0,
                _ => false,
            };
            if is_hidden {
                hidden.push(entry);
            } else {
                shown.push(entry);
            }
        }
    }
    (shown, hidden)
}
/// ServerList.save for the shown entries, keeping the hidden ones the file has.
pub fn save(path: &Path, servers: &[ServerEntry]) -> std::io::Result<()> {
    let (_, hidden) = load_all(path);
    save_all(path, servers, &hidden)
}
/// ServerList.save: the shown entries with `hidden: false`, then the hidden
/// ones with `hidden: true`, written to a temporary file, then replaced.
pub fn save_all(
    path: &Path,
    servers: &[ServerEntry],
    hidden: &[ServerEntry],
) -> std::io::Result<()> {
    let flagged = |entry: &ServerEntry, flag: bool| {
        let mut tag = entry.to_tag();
        if let Tag::Compound(c) = &mut tag {
            c.insert(JavaString::from("hidden"), Tag::Byte(i8::from(flag)));
        }
        tag
    };
    let list = servers
        .iter()
        .map(|e| flagged(e, false))
        .chain(hidden.iter().map(|e| flagged(e, true)))
        .collect();
    let root = NamedTag {
        name: JavaString::from(""),
        tag: Tag::compound([("servers", Tag::List(list))]),
    };
    write(path, &root)
}
/// ServerList.add(server, true): a hidden entry goes first, and only the 16
/// newest hidden entries are kept (`MAX_HIDDEN_SERVERS`); then saved.
pub fn add_hidden(path: &Path, entry: ServerEntry) -> std::io::Result<()> {
    let (shown, mut hidden) = load_all(path);
    hidden.insert(0, entry);
    hidden.truncate(16);
    save_all(path, &shown, &hidden)
}
/// Puts `entry` first, keeping every other entry and field as the file has them (the hidden
/// ones too). A file that is there but can't be read is left as it is.
pub fn prepend(path: &Path, entry: &ServerEntry) -> std::io::Result<()> {
    insert(path, entry, |_| None)
}
/// Removes every entry `matches` accepts (given its name and address), keeping the other
/// entries and every field as the file has them (the hidden ones too). Whether any went; a
/// missing file has none, and the file is only written when something went.
pub fn remove_where(path: &Path, matches: impl Fn(&str, &str) -> bool) -> std::io::Result<bool> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut root = nbt::read_named(&bytes, nbt::Limits::default())
        .map_err(std::io::Error::other)?
        .0;
    let Some(Tag::List(list)) = root.tag.get_mut("servers") else {
        return Ok(false);
    };
    let text = |t: &Tag, key: &str| match t.get(key) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    };
    let before = list.len();
    list.retain(|t| !matches(&text(t, "name"), &text(t, "ip")));
    if list.len() == before {
        return Ok(false);
    }
    write(path, &root)?;
    Ok(true)
}
/// Inserts `entry` at the index `at` picks in the file's list (`None`: first).
fn insert(
    path: &Path,
    entry: &ServerEntry,
    at: impl Fn(&[Tag]) -> Option<usize>,
) -> std::io::Result<()> {
    let mut root = match std::fs::read(path) {
        Ok(bytes) => {
            nbt::read_named(&bytes, nbt::Limits::default())
                .map_err(std::io::Error::other)?
                .0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => NamedTag {
            name: JavaString::from(""),
            tag: Tag::Compound(Default::default()),
        },
        Err(e) => return Err(e),
    };
    let Tag::Compound(top) = &mut root.tag else {
        return Err(std::io::Error::other("servers.dat is not a compound"));
    };
    match top
        .entry(JavaString::from("servers"))
        .or_insert_with(|| Tag::List(Vec::new()))
    {
        Tag::List(list) => {
            let index = at(list).unwrap_or(0);
            list.insert(index, entry.to_tag());
        }
        _ => return Err(std::io::Error::other("servers.dat has no server list")),
    }
    write(path, &root)
}
fn write(path: &Path, root: &NamedTag) -> std::io::Result<()> {
    let bytes = nbt::write_named(root, nbt::Limits::default()).map_err(std::io::Error::other)?;
    // Its own temporary file: another game on the same folder writes at the same time.
    let tmp = path.with_extension(format!("dat_tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// ServerAddress.parseString: `host`, `host:port` or `[v6]:port`; default 25565.
pub fn parse_address(text: &str) -> Option<(String, u16)> {
    let (host, port) = parse_address_parts(text)?;
    // ServerAddress.getHost() returns the empty host when IDN conversion fails;
    // the caller then reports the invalid-address screen instead of connecting.
    Some((idn_to_ascii(&host).unwrap_or_default(), port))
}

fn parse_address_parts(text: &str) -> Option<(String, u16)> {
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest.rsplit_once(']')?;
        // HostAndPort requires a colon inside a bracketed host; it does not
        // accept a bracketed DNS name as an IPv6-style address.
        if !host.contains(':') {
            return None;
        }
        let port = match tail.strip_prefix(':') {
            Some(p) if p.bytes().all(|byte| byte.is_ascii_digit()) => parse_port(p)?,
            None if tail.is_empty() => 25565,
            _ => return None,
        };
        return Some((host.to_owned(), port));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            if host.is_empty() {
                return None;
            }
            Some((host.to_owned(), parse_port(port)?))
        }
        Some(_) => Some((text.to_owned(), 25565)),
        None => Some((text.to_owned(), 25565)),
    }
}

/// ServerAddress.isValidAddress, which the Add/Edit Server screen's Done button
/// and the Direct Connection screen's Join Server button ask: the text parses as
/// a host (and port) and `java.net.IDN.toASCII` accepts the host.
pub fn is_valid_address(text: &str) -> bool {
    parse_address_parts(text).is_some_and(|(host, _)| idn_to_ascii(&host).is_some())
}

/// Unicode 3.2 Nameprep's unassigned code points. Java's default
/// `IDN.toASCII` rejects them; newer IDNA mappings may otherwise accept them.
const UNASSIGNED_UNICODE_3_2: &[(u32, u32)] = &[
    (545, 545),
    (564, 591),
    (686, 687),
    (751, 767),
    (848, 863),
    (880, 883),
    (886, 889),
    (891, 893),
    (895, 899),
    (907, 907),
    (909, 909),
    (930, 930),
    (975, 975),
    (1015, 1023),
    (1159, 1159),
    (1231, 1231),
    (1270, 1271),
    (1274, 1279),
    (1296, 1328),
    (1367, 1368),
    (1376, 1376),
    (1416, 1416),
    (1419, 1424),
    (1442, 1442),
    (1466, 1466),
    (1477, 1487),
    (1515, 1519),
    (1525, 1547),
    (1549, 1562),
    (1564, 1566),
    (1568, 1568),
    (1595, 1599),
    (1622, 1631),
    (1774, 1775),
    (1791, 1791),
    (1806, 1806),
    (1837, 1839),
    (1867, 1919),
    (1970, 2304),
    (2308, 2308),
    (2362, 2363),
    (2382, 2383),
    (2389, 2391),
    (2417, 2432),
    (2436, 2436),
    (2445, 2446),
    (2449, 2450),
    (2473, 2473),
    (2481, 2481),
    (2483, 2485),
    (2490, 2491),
    (2493, 2493),
    (2501, 2502),
    (2505, 2506),
    (2510, 2518),
    (2520, 2523),
    (2526, 2526),
    (2532, 2533),
    (2555, 2561),
    (2563, 2564),
    (2571, 2574),
    (2577, 2578),
    (2601, 2601),
    (2609, 2609),
    (2612, 2612),
    (2615, 2615),
    (2618, 2619),
    (2621, 2621),
    (2627, 2630),
    (2633, 2634),
    (2638, 2648),
    (2653, 2653),
    (2655, 2661),
    (2677, 2688),
    (2692, 2692),
    (2700, 2700),
    (2702, 2702),
    (2706, 2706),
    (2729, 2729),
    (2737, 2737),
    (2740, 2740),
    (2746, 2747),
    (2758, 2758),
    (2762, 2762),
    (2766, 2767),
    (2769, 2783),
    (2785, 2789),
    (2800, 2816),
    (2820, 2820),
    (2829, 2830),
    (2833, 2834),
    (2857, 2857),
    (2865, 2865),
    (2868, 2869),
    (2874, 2875),
    (2884, 2886),
    (2889, 2890),
    (2894, 2901),
    (2904, 2907),
    (2910, 2910),
    (2914, 2917),
    (2929, 2945),
    (2948, 2948),
    (2955, 2957),
    (2961, 2961),
    (2966, 2968),
    (2971, 2971),
    (2973, 2973),
    (2976, 2978),
    (2981, 2983),
    (2987, 2989),
    (2998, 2998),
    (3002, 3005),
    (3011, 3013),
    (3017, 3017),
    (3022, 3030),
    (3032, 3046),
    (3059, 3072),
    (3076, 3076),
    (3085, 3085),
    (3089, 3089),
    (3113, 3113),
    (3124, 3124),
    (3130, 3133),
    (3141, 3141),
    (3145, 3145),
    (3150, 3156),
    (3159, 3167),
    (3170, 3173),
    (3184, 3201),
    (3204, 3204),
    (3213, 3213),
    (3217, 3217),
    (3241, 3241),
    (3252, 3252),
    (3258, 3261),
    (3269, 3269),
    (3273, 3273),
    (3278, 3284),
    (3287, 3293),
    (3295, 3295),
    (3298, 3301),
    (3312, 3329),
    (3332, 3332),
    (3341, 3341),
    (3345, 3345),
    (3369, 3369),
    (3386, 3389),
    (3396, 3397),
    (3401, 3401),
    (3406, 3414),
    (3416, 3423),
    (3426, 3429),
    (3440, 3457),
    (3460, 3460),
    (3479, 3481),
    (3506, 3506),
    (3516, 3516),
    (3518, 3519),
    (3527, 3529),
    (3531, 3534),
    (3541, 3541),
    (3543, 3543),
    (3552, 3569),
    (3573, 3584),
    (3643, 3646),
    (3676, 3712),
    (3715, 3715),
    (3717, 3718),
    (3721, 3721),
    (3723, 3724),
    (3726, 3731),
    (3736, 3736),
    (3744, 3744),
    (3748, 3748),
    (3750, 3750),
    (3752, 3753),
    (3756, 3756),
    (3770, 3770),
    (3774, 3775),
    (3781, 3781),
    (3783, 3783),
    (3790, 3791),
    (3802, 3803),
    (3806, 3839),
    (3912, 3912),
    (3947, 3952),
    (3980, 3983),
    (3992, 3992),
    (4029, 4029),
    (4045, 4046),
    (4048, 4095),
    (4130, 4130),
    (4136, 4136),
    (4139, 4139),
    (4147, 4149),
    (4154, 4159),
    (4186, 4255),
    (4294, 4303),
    (4345, 4346),
    (4348, 4351),
    (4442, 4446),
    (4515, 4519),
    (4602, 4607),
    (4615, 4615),
    (4679, 4679),
    (4681, 4681),
    (4686, 4687),
    (4695, 4695),
    (4697, 4697),
    (4702, 4703),
    (4743, 4743),
    (4745, 4745),
    (4750, 4751),
    (4783, 4783),
    (4785, 4785),
    (4790, 4791),
    (4799, 4799),
    (4801, 4801),
    (4806, 4807),
    (4815, 4815),
    (4823, 4823),
    (4847, 4847),
    (4879, 4879),
    (4881, 4881),
    (4886, 4887),
    (4895, 4895),
    (4935, 4935),
    (4955, 4960),
    (4989, 5023),
    (5109, 5120),
    (5751, 5759),
    (5789, 5791),
    (5873, 5887),
    (5901, 5901),
    (5909, 5919),
    (5943, 5951),
    (5972, 5983),
    (5997, 5997),
    (6001, 6001),
    (6004, 6015),
    (6109, 6111),
    (6122, 6143),
    (6159, 6159),
    (6170, 6175),
    (6264, 6271),
    (6314, 7679),
    (7836, 7839),
    (7930, 7935),
    (7958, 7959),
    (7966, 7967),
    (8006, 8007),
    (8014, 8015),
    (8024, 8024),
    (8026, 8026),
    (8028, 8028),
    (8030, 8030),
    (8062, 8063),
    (8117, 8117),
    (8133, 8133),
    (8148, 8149),
    (8156, 8156),
    (8176, 8177),
    (8181, 8181),
    (8191, 8191),
    (8275, 8278),
    (8280, 8286),
    (8292, 8297),
    (8306, 8307),
    (8335, 8351),
    (8370, 8399),
    (8427, 8447),
    (8507, 8508),
    (8524, 8530),
    (8580, 8591),
    (9167, 9215),
    (9255, 9279),
    (9291, 9311),
    (9471, 9471),
    (9748, 9749),
    (9752, 9752),
    (9854, 9855),
    (9866, 9984),
    (9989, 9989),
    (9994, 9995),
    (10024, 10024),
    (10060, 10060),
    (10062, 10062),
    (10067, 10069),
    (10071, 10071),
    (10079, 10080),
    (10133, 10135),
    (10160, 10160),
    (10175, 10191),
    (10220, 10223),
    (11008, 11903),
    (11930, 11930),
    (12020, 12031),
    (12246, 12271),
    (12284, 12287),
    (12352, 12352),
    (12439, 12440),
    (12544, 12548),
    (12589, 12592),
    (12687, 12687),
    (12728, 12783),
    (12829, 12831),
    (12868, 12880),
    (12924, 12926),
    (13004, 13007),
    (13055, 13055),
    (13175, 13178),
    (13278, 13279),
    (13311, 13311),
    (19894, 19967),
    (40870, 40959),
    (42125, 42127),
    (42183, 44031),
    (55204, 55295),
    (64046, 64047),
    (64107, 64255),
    (64263, 64274),
    (64280, 64284),
    (64311, 64311),
    (64317, 64317),
    (64319, 64319),
    (64322, 64322),
    (64325, 64325),
    (64434, 64466),
    (64832, 64847),
    (64912, 64913),
    (64968, 65007),
    (65021, 65023),
    (65040, 65055),
    (65060, 65071),
    (65095, 65096),
    (65107, 65107),
    (65127, 65127),
    (65132, 65135),
    (65141, 65141),
    (65277, 65278),
    (65280, 65280),
    (65471, 65473),
    (65480, 65481),
    (65488, 65489),
    (65496, 65497),
    (65501, 65503),
    (65511, 65511),
    (65519, 65528),
    (65534, 66303),
    (66335, 66335),
    (66340, 66351),
    (66379, 66559),
    (66598, 66599),
    (66638, 118783),
    (119030, 119039),
    (119079, 119081),
    (119262, 119807),
    (119893, 119893),
    (119965, 119965),
    (119968, 119969),
    (119971, 119972),
    (119975, 119976),
    (119981, 119981),
    (119994, 119994),
    (119996, 119996),
    (120001, 120001),
    (120004, 120004),
    (120070, 120070),
    (120075, 120076),
    (120085, 120085),
    (120093, 120093),
    (120122, 120122),
    (120127, 120127),
    (120133, 120133),
    (120135, 120137),
    (120145, 120145),
    (120484, 120487),
    (120778, 120781),
    (120832, 131071),
    (173783, 194559),
    (195102, 917504),
    (917506, 917535),
    (917632, 983039),
    (1048574, 1048575),
    (1114110, 1114111),
];

fn is_unassigned_unicode_3_2(c: char) -> bool {
    let code_point = c as u32;
    UNASSIGNED_UNICODE_3_2
        .binary_search_by(|&(start, end)| {
            if code_point < start {
                std::cmp::Ordering::Greater
            } else if code_point > end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// The ASCII half of `java.net.IDN.toASCII`: one trailing dot is allowed (a lone
/// dot too), every other label must be 1 to 63 characters long.
fn idn_accepts(host: &str) -> bool {
    let labels = host.strip_suffix('.').unwrap_or(host);
    labels.is_empty() || labels.split('.').all(|l| (1..=63).contains(&l.len()))
}

/// `java.net.IDN.toASCII` with its default (non-STD3) rules. Nameprep's
/// Unicode 3.2 mapping is applied before the UTS 46 encoder; the latter still
/// supplies the DNS length and prohibited-character checks. ASCII labels stay
/// untouched so their case and permissive default-rule characters are
/// preserved.
#[allow(deprecated)]
fn idn_to_ascii(host: &str) -> Option<String> {
    // Java's IDN implementation uses the Unicode 3.2 Nameprep table, while
    // the modern UTS #46 mapper in this crate knows about newer characters.
    if host.chars().any(is_unassigned_unicode_3_2) {
        return None;
    }
    if host.is_ascii() {
        return idn_accepts(host).then(|| host.to_owned());
    }
    let mapped: String = host
        .chars()
        .map(|c| match c {
            '\u{3002}' | '\u{ff0e}' | '\u{ff61}' => '.',
            c => c,
        })
        .collect();
    if mapped.is_ascii() {
        return idn_accepts(&mapped).then_some(mapped);
    }
    let trailing_dot = mapped.ends_with('.');
    let labels = mapped.strip_suffix('.').unwrap_or(&mapped);
    if labels.is_empty() || labels.split('.').any(str::is_empty) {
        return None;
    }
    let config = idna::Config::default().transitional_processing(true);
    let adapter = idna_adapter::Adapter::new();
    let mut ascii = String::with_capacity(mapped.len());
    for label in labels.split('.') {
        let converted = if label.is_ascii() {
            (1..=63).contains(&label.len()).then(|| label.to_owned())?
        } else {
            let mut nameprep = String::with_capacity(label.len());
            for c in label.chars() {
                match c {
                    // Nameprep's map-to-nothing step removes joiners before
                    // the label is checked and encoded.
                    '\u{200c}' | '\u{200d}' => {}
                    'ß' | 'ẞ' => nameprep.push_str("ss"),
                    'ς' => nameprep.push('σ'),
                    c => nameprep.push(c),
                }
            }
            let nameprep: String = adapter
                .map_normalize(nameprep.chars())
                .map(|c| {
                    // U+04CF was unassigned in Unicode 3.2. Modern UTS 46
                    // uses it as the lowercase mapping of U+04C0, but
                    // Nameprep leaves the then-assigned U+04C0 unchanged.
                    if c == '\u{04cf}' { '\u{04c0}' } else { c }
                })
                .collect();
            if !nameprep_bidi_is_valid(&nameprep, &adapter) {
                return None;
            }
            if nameprep.contains('\u{04c0}') || (nameprep != label && !nameprep.is_ascii()) {
                config.to_ascii(&format!("a{nameprep}")).ok()?;
                format!("xn--{}", idna::punycode::encode_str(&nameprep)?)
            } else {
                match config.to_ascii(&nameprep) {
                    Ok(converted) => converted,
                    Err(_)
                        if adapter.is_mark(nameprep.chars().next()?)
                            && config.to_ascii(&format!("a{nameprep}")).is_ok() =>
                    {
                        format!("xn--{}", idna::punycode::encode_str(&nameprep)?)
                    }
                    Err(_) => return None,
                }
            }
        };
        if converted.is_empty() || converted.len() > 63 {
            return None;
        }
        if !ascii.is_empty() {
            ascii.push('.');
        }
        ascii.push_str(&converted);
    }
    if trailing_dot {
        ascii.push('.');
    }
    Some(ascii)
}

fn nameprep_bidi_is_valid(label: &str, adapter: &idna_adapter::Adapter) -> bool {
    let Some(first) = label.chars().next() else {
        return false;
    };
    let rtl = label.chars().any(|c| {
        adapter
            .bidi_class(c)
            .to_mask()
            .intersects(idna_adapter::RTL_MASK)
    });
    if !rtl {
        return true;
    }
    if !adapter
        .bidi_class(first)
        .to_mask()
        .intersects(idna_adapter::RTL_MASK)
    {
        return false;
    }
    label.chars().last().is_some_and(|last| {
        adapter
            .bidi_class(last)
            .to_mask()
            .intersects(idna_adapter::RTL_MASK)
    })
}

/// HostAndPort uses the default for an empty port, refuses a leading plus,
/// and accepts signed zero on its non-bracketed path before checking range.
fn parse_port(text: &str) -> Option<u16> {
    if text.is_empty() {
        Some(25565)
    } else if text.starts_with('+') {
        None
    } else {
        u16::try_from(text.parse::<i32>().ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_dat_round_trips() {
        let dir = std::env::temp_dir().join(format!("horizon-servers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        let list = vec![
            ServerEntry::new("Minecraft Server", "localhost:25565"),
            ServerEntry {
                accept_textures: Some(false),
                ..ServerEntry::new("Other", "example.org")
            },
        ];
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
        // Vanilla layout: an unnamed root compound with a "servers" list.
        let (root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        assert_eq!(root.name, JavaString::from(""));
        assert!(matches!(root.tag.get("servers"), Some(Tag::List(l)) if l.len() == 2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepend_keeps_hidden_entries_and_unknown_fields() {
        let dir = std::env::temp_dir().join(format!("horizon-prepend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        prepend(&path, &ServerEntry::new("First", "a.example")).unwrap();
        assert_eq!(load(&path), [ServerEntry::new("First", "a.example")]);
        let hidden = Tag::compound([
            ("name", Tag::String(JavaString::from("Direct"))),
            ("ip", Tag::String(JavaString::from("b.example"))),
            ("hidden", Tag::Byte(1)),
            ("acceptedCodeOfConduct", Tag::String(JavaString::from("x"))),
        ]);
        let root = NamedTag {
            name: JavaString::from(""),
            tag: Tag::compound([("servers", Tag::List(vec![hidden.clone()]))]),
        };
        write(&path, &root).unwrap();
        prepend(&path, &ServerEntry::new("Top", "c.example")).unwrap();
        let (root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        let Some(Tag::List(list)) = root.tag.get("servers") else {
            panic!("a server list");
        };
        assert_eq!(
            list,
            &[ServerEntry::new("Top", "c.example").to_tag(), hidden]
        );
        // A file that can't be read stays as it is.
        std::fs::write(&path, b"not nbt").unwrap();
        assert!(prepend(&path, &ServerEntry::new("Top", "c.example")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not nbt");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_addresses_like_server_address() {
        assert_eq!(
            parse_address("localhost"),
            Some(("localhost".into(), 25565))
        );
        assert_eq!(
            parse_address("127.0.0.1:25570"),
            Some(("127.0.0.1".into(), 25570))
        );
        assert_eq!(parse_address("[::1]:1234"), Some(("::1".into(), 1234)));
        assert_eq!(parse_address("host:notaport"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn empty_explicit_ports_use_the_vanilla_default() {
        for (input, host) in [
            ("example.com:", "example.com"),
            ("127.0.0.1:", "127.0.0.1"),
            ("[::1]:", "::1"),
            ("[2001:db8::1]:", "2001:db8::1"),
        ] {
            assert_eq!(parse_address(input), Some((host.into(), 25565)), "{input}");
        }
        assert_eq!(parse_address(" host:25565"), Some((" host".into(), 25565)));
        assert_eq!(parse_address("host:25565 "), None);
    }

    #[test]
    fn ports_and_brackets_keep_vanilla_validation() {
        for input in [
            ":",
            ":25565",
            "[]",
            "[]:",
            "[example.com]",
            "[example.com]:",
            "[::1]junk",
            "example.com:+25565",
            "[::1]:+25565",
            "[::1]:-0",
            "example.com:-1",
            "example.com:65536",
            "[::1]:65536",
            "host:2147483648",
            "host:1_000",
            "host:\u{0661}\u{0662}",
        ] {
            assert_eq!(parse_address(input), None, "{input}");
        }
        assert_eq!(parse_address("host:-0"), Some(("host".into(), 0)));
        assert_eq!(parse_address("host:0"), Some(("host".into(), 0)));
        assert_eq!(parse_address("[::1]:65535"), Some(("::1".into(), 65535)));
    }

    #[test]
    fn the_server_screens_refuse_a_host_idn_refuses() {
        let label63 = "x".repeat(63);
        let label64 = "x".repeat(64);
        // Each verdict was taken from java.net.IDN.toASCII on the same host.
        for host in [
            "localhost".to_owned(),
            "a.b.c".to_owned(),
            "a.b.".to_owned(),
            ".".to_owned(),
            "-a.a-.a_b".to_owned(),
            "::1".to_owned(),
            "127.0.0.1".to_owned(),
            label63.clone(),
            format!("{label63}.{label63}.{label63}.{label63}"),
        ] {
            assert!(is_valid_address(&host), "{host}");
            assert!(is_valid_address(&format!("{host}:25570")), "{host}");
        }
        for host in [
            "a..b".to_owned(),
            ".a".to_owned(),
            "a..".to_owned(),
            "..".to_owned(),
            label64.clone(),
            format!("{label64}."),
            format!("{label63}.{label64}"),
        ] {
            assert!(!is_valid_address(&host), "{host}");
            assert!(!is_valid_address(&format!("{host}:25570")), "{host}");
            // ServerAddress.parseString does not ask IDN, so the parser still reads it.
            assert!(parse_address(&host).is_some(), "{host}");
        }
        // What parse_address refuses stays refused.
        assert!(!is_valid_address(""));
        assert!(!is_valid_address("host:notaport"));
        for (input, host) in [
            ("münchen.de", "xn--mnchen-3ya.de"),
            ("straße.de", "strasse.de"),
            ("ς.example", "xn--4xa.example"),
            ("\u{04c0}.example", "xn--d5a.example"),
            ("א1א.example", "xn--1-zhcb.example"),
            ("\u{0301}.example", "xn--lsa.example"),
            ("EXAMPLE.é", "EXAMPLE.xn--9ca"),
            ("é.EXAMPLE", "xn--9ca.EXAMPLE"),
            ("a。b", "a.b"),
        ] {
            assert!(is_valid_address(input), "{input}");
            assert_eq!(parse_address(input), Some((host.into(), 25565)), "{input}");
        }
        assert!(!is_valid_address("\u{200d}.invalid"));
        assert_eq!(
            parse_address("a\u{200d}b.example"),
            Some(("ab.example".into(), 25565))
        );
        assert!(!is_valid_address("א1.example"));
        assert!(!is_valid_address("א1א\u{0301}.example"));
        assert!(!is_valid_address("\u{1c80}.example"));
        assert!(!is_valid_address("\u{1e9e}.example"));
        let long_unicode_label = "é".repeat(64);
        assert!(!is_valid_address(&format!("{long_unicode_label}.example")));
    }
}
