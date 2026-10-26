//! Player skins from profile `textures` properties: authlib's
//! `MinecraftServicesSessionService.unpackTextures`, vanilla's
//! `SkinManager` lookup (which texture a player shows, and when the default
//! one) and `SkinTextureDownloader.processLegacySkin`.

/// `Property.hasSignature` and the services key check, as authlib's
/// `SignatureState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signature {
    Unsigned,
    Signed,
    Invalid,
}

/// `MinecraftProfileTextures`: the texture URLs a profile names. Empty when
/// the payload doesn't decode or names a URL outside the allowed domains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Textures {
    pub skin: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    pub signed: bool,
}

/// The `getTexture` endpoint's valid URIs from Minecraft services discovery.
pub const TEXTURE_URIS: [&str; 2] = [
    "https://textures.minecraft.net/texture/{textureId}",
    "http://textures.minecraft.net/texture/{textureId}",
];

/// `java.util.Base64.getDecoder().decode`.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut shift) = (0u32, 18i32);
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        let b = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => {
                if shift == 6 && (i == bytes.len() || bytes[i] != '=') || shift == 18 {
                    return None;
                }
                if shift == 6 {
                    i += 1;
                }
                break;
            }
            _ => return None,
        };
        bits |= b << shift;
        shift -= 6;
        if shift < 0 {
            out.extend([(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
            shift = 18;
            bits = 0;
        }
    }
    match shift {
        6 => out.push((bits >> 16) as u8),
        0 => out.extend([(bits >> 16) as u8, (bits >> 8) as u8]),
        12 => return None,
        _ => {}
    }
    (i >= bytes.len()).then_some(out)
}

/// `unpackTextures(property)`: the textures of a `textures` property's
/// base64 JSON payload.
pub fn unpack(value: &str, signature: Signature, valid_uris: &[&str]) -> Textures {
    let empty = Textures::default();
    let Some(json) = base64_decode(value)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    else {
        return empty;
    };
    let Some(textures) = json.get("textures").and_then(|t| t.as_object()) else {
        return empty;
    };
    if textures.is_empty() {
        return empty;
    }
    let mut out = Textures {
        signed: signature == Signature::Signed,
        ..empty.clone()
    };
    for (kind, texture) in textures {
        let Some(url) = texture.get("url").and_then(|u| u.as_str()) else {
            return empty;
        };
        let allowed = valid_uris
            .iter()
            .any(|u| url.starts_with(&u.replace("{textureId}", "")));
        if !allowed {
            return empty;
        }
        let slot = match kind.as_str() {
            "SKIN" => &mut out.skin,
            "CAPE" => &mut out.cape,
            "ELYTRA" => &mut out.elytra,
            _ => continue,
        };
        *slot = Some(url.to_owned());
    }
    out
}

/// The skin URL of a `minecraft:profile` component's property list. Player
/// heads and mannequins read their profile this way, and `SkinManager` lets
/// those textures be unsigned (`getInsecureSkinInformation`).
pub fn profile_skin_url(profile: &serde_json::Value) -> Option<String> {
    let value = profile
        .get("properties")?
        .as_array()?
        .iter()
        .find_map(|p| {
            if p.get("name")?.as_str()? == "textures" {
                p.get("value")?.as_str()
            } else {
                None
            }
        })?;
    unpack(value, Signature::Unsigned, &TEXTURE_URIS).skin
}

/// `profile_skin_url` on a block entity's `profile` NBT (`SkullBlockEntity`):
/// the same `properties` list, held as a compound. A name-only profile (a
/// string tag) has no properties and no skin URL.
pub fn profile_skin_url_nbt(profile: &horizon_world::nbt::Tag) -> Option<String> {
    let properties = match profile.get("properties")? {
        horizon_world::nbt::Tag::List(list) => list,
        _ => return None,
    };
    let value = properties.iter().find_map(|p| {
        let name = nbt_text(p.get("name"))?;
        (name == "textures")
            .then(|| nbt_text(p.get("value")))
            .flatten()
    })?;
    unpack(&value, Signature::Unsigned, &TEXTURE_URIS).skin
}

/// The profile's uuid as u128: `id` as an int array (block entity NBT) or a
/// uuid string (older and component forms). Used for `DefaultPlayerSkin.get`
/// when a head's skin is not downloaded yet.
pub fn profile_uuid_nbt(profile: &horizon_world::nbt::Tag) -> Option<u128> {
    match profile.get("id")? {
        horizon_world::nbt::Tag::IntArray(v) if v.len() == 4 => Some(
            v.iter()
                .fold(0u128, |a, &w| (a << 32) | u128::from(w as u32)),
        ),
        horizon_world::nbt::Tag::String(s) => {
            let digits: String = s.to_string_lossy().chars().filter(|c| *c != '-').collect();
            u128::from_str_radix(&digits, 16).ok()
        }
        _ => None,
    }
}

fn nbt_text(tag: Option<&horizon_world::nbt::Tag>) -> Option<String> {
    match tag {
        Some(horizon_world::nbt::Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// `MinecraftProfileTexture.getHash`: the base name of the URL's path.
pub fn texture_hash(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let path = &rest[rest.find('/').unwrap_or(rest.len())..];
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let name = &path[path.rfind(['/', '\\']).map_or(0, |i| i + 1)..];
    Some(name[..name.rfind('.').unwrap_or(name.len())].to_owned())
}

/// `SkinManager.TextureCache.registerTexture`: the texture id of a skin,
/// `skins/` and the SHA-1 of its hash's UTF-16 code units.
pub fn skin_location(hash: &str) -> String {
    let units: Vec<u8> = hash.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let digest = sha1(&units);
    let mut out = String::from("skins/");
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Where a texture download stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Download {
    Ready,
    Pending,
    Failed,
}

/// `MinecraftProfileTexture.Type`: which texture cache a download is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Skin,
    Cape,
    Elytra,
}

/// `SkinManager.createLookup(profile, requireSecure).get()`: the skin
/// texture a player shows, or `None` for their default skin. `download`
/// tells how the skin, cape and elytra texture downloads stand.
pub fn lookup(
    textures: &Textures,
    require_secure: bool,
    mut download: impl FnMut(Kind, &str) -> Download,
) -> Option<String> {
    let mut skin = None;
    let mut state = Download::Ready;
    let urls = [
        (Kind::Skin, &textures.skin),
        (Kind::Cape, &textures.cape),
        (Kind::Elytra, &textures.elytra),
    ];
    for (kind, url) in urls {
        let Some(url) = url else { continue };
        // registerTexture computes the hash first; a bad URL fails the lookup.
        let hash = texture_hash(url)?;
        if kind == Kind::Skin {
            skin = Some(skin_location(&hash));
        }
        match download(kind, url) {
            Download::Failed => return None,
            Download::Pending => state = Download::Pending,
            Download::Ready => {}
        }
    }
    if state != Download::Ready || require_secure && !textures.signed {
        return None;
    }
    skin
}

/// `SkinTextureDownloader.processLegacySkin` on RGBA pixels: 64x32 skins
/// get the 64x64 layout, and the opaque parts lose their alpha. `None` for
/// any other size (the download fails).
pub fn process_legacy_skin(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width != 64 || height != 32 && height != 64 {
        return None;
    }
    let legacy = height == 32;
    let mut img = vec![0u8; 64 * 64 * 4];
    let n = rgba.len().min(img.len());
    img[..n].copy_from_slice(&rgba[..n]);
    let at = |x: i32, y: i32| ((y * 64 + x) * 4) as usize;
    if legacy {
        let rects = [
            (4, 16, 16, 32, 4, 4),
            (8, 16, 16, 32, 4, 4),
            (0, 20, 24, 32, 4, 12),
            (4, 20, 16, 32, 4, 12),
            (8, 20, 8, 32, 4, 12),
            (12, 20, 16, 32, 4, 12),
            (44, 16, -8, 32, 4, 4),
            (48, 16, -8, 32, 4, 4),
            (40, 20, 0, 32, 4, 12),
            (44, 20, -8, 32, 4, 12),
            (48, 20, -16, 32, 4, 12),
            (52, 20, -8, 32, 4, 12),
        ];
        for (sx, sy, ox, oy, w, h) in rects {
            for y in 0..h {
                for x in 0..w {
                    let (s, d) = (at(sx + x, sy + y), at(sx + ox + w - 1 - x, sy + oy + y));
                    let p: [u8; 4] = img[s..s + 4].try_into().unwrap();
                    img[d..d + 4].copy_from_slice(&p);
                }
            }
        }
    }
    let no_alpha = |img: &mut Vec<u8>, x0: i32, y0: i32, x1: i32, y1: i32| {
        for x in x0..x1 {
            for y in y0..y1 {
                img[at(x, y) + 3] = 255;
            }
        }
    };
    no_alpha(&mut img, 0, 0, 32, 16);
    if legacy {
        // doNotchTransparencyHack: a fully opaque hat layer is cleared.
        let opaque = (32..64).all(|x| (0..32).all(|y| img[at(x, y) + 3] >= 128));
        if opaque {
            for x in 32..64 {
                for y in 0..32 {
                    img[at(x, y) + 3] = 0;
                }
            }
        }
    }
    no_alpha(&mut img, 0, 16, 64, 32);
    no_alpha(&mut img, 16, 48, 48, 64);
    Some(img)
}

/// `SkinTextureDownloader`: fetches textures on background threads, from
/// the local cache (`<root>/<2 hex>/<sha1>`) or over HTTP, keeping each one
/// by kind and hash as vanilla's `TextureCache`s do. A texture that can't
/// be fetched fails, and the player keeps their default skin.
pub struct Downloader {
    root: std::path::PathBuf,
    states: std::collections::HashMap<(Kind, String), Download>,
    done: std::sync::mpsc::Receiver<((Kind, String), Option<Vec<u8>>)>,
    send: std::sync::mpsc::Sender<((Kind, String), Option<Vec<u8>>)>,
    ready: Vec<(String, Vec<u8>)>,
}
impl Downloader {
    pub fn new(root: std::path::PathBuf) -> Self {
        let (send, done) = std::sync::mpsc::channel();
        Self {
            root,
            states: Default::default(),
            done,
            send,
            ready: Vec::new(),
        }
    }
    /// How the download of `url` stands, starting it the first time.
    pub fn state(&mut self, kind: Kind, url: &str) -> Download {
        while let Ok((key, image)) = self.done.try_recv() {
            let state = match image {
                Some(rgba) => {
                    if key.0 == Kind::Skin {
                        self.ready.push((skin_location(&key.1), rgba));
                    }
                    Download::Ready
                }
                None => Download::Failed,
            };
            self.states.insert(key, state);
        }
        let Some(hash) = texture_hash(url) else {
            return Download::Failed;
        };
        let key = (kind, hash);
        if let Some(state) = self.states.get(&key) {
            return *state;
        }
        self.states.insert(key.clone(), Download::Pending);
        let digest: String = skin_location(&key.1)["skins/".len()..].to_owned();
        let file = self
            .root
            .join(if digest.len() > 2 { &digest[..2] } else { "xx" })
            .join(&digest);
        let (url, send) = (url.to_owned(), self.send.clone());
        std::thread::spawn(move || {
            let bytes = std::fs::read(&file).ok().or_else(|| {
                let body = http_get(&url)?;
                let _ = std::fs::create_dir_all(file.parent()?);
                let _ = std::fs::write(&file, &body);
                Some(body)
            });
            let image = bytes
                .and_then(|b| crate::hud::decode_bytes(&b).ok())
                .and_then(|(w, h, rgba)| match kind {
                    Kind::Skin => process_legacy_skin(w, h, &rgba),
                    _ => Some(rgba),
                });
            let _ = send.send((key, image));
        });
        Download::Pending
    }
    /// Skins that finished since the last call: texture id and 64x64 RGBA.
    pub fn take_ready(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.ready)
    }
}

/// A GET over HTTP or HTTPS (textures.minecraft.net serves both; vanilla
/// uses https): the body of a 2xx answer, 16 MiB at most, after redirects.
fn http_get(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let response = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Java")
        .build()
        .get(url)
        .call()
        .ok()?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(16 << 20)
        .read_to_end(&mut body)
        .ok()?;
    Some(body)
}

/// SHA-1 (FIPS 180-4).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend((data.len() as u64 * 8).to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn profile_components_supply_unsigned_head_skins() {
        let value = "eyJ0ZXh0dXJlcyI6IHsiU0tJTiI6IHsidXJsIjogImh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYzEyMyJ9fX0=";
        let profile = serde_json::json!({"properties": [{"name": "textures", "value": value}]});
        assert_eq!(
            super::profile_skin_url(&profile).as_deref(),
            Some("https://textures.minecraft.net/texture/abc123")
        );
        let other = serde_json::json!({"properties": [{"name": "other", "value": value}]});
        assert_eq!(super::profile_skin_url(&other), None);
        let bad = serde_json::json!({"properties": [{"name": "textures", "value": "not base64"}]});
        assert_eq!(super::profile_skin_url(&bad), None);
        assert_eq!(super::profile_skin_url(&serde_json::json!({})), None);
    }

    /// Textures come over plain HTTP too (a local server here; https is the
    /// same request through rustls), and only 2xx answers count.
    #[test]
    fn http_get_reads_2xx_bodies() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut line).unwrap();
                while reader.read_line(&mut String::new()).unwrap() > 2 {}
                let reply: &[u8] = if line.contains("/texture/abc") {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPNG!"
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                stream.write_all(reply).unwrap();
            }
        });
        assert_eq!(
            http_get(&format!("{base}/texture/abc")),
            Some(b"PNG!".to_vec())
        );
        assert_eq!(http_get(&format!("{base}/texture/missing")), None);
    }

    use super::*;

    #[test]
    fn skull_block_entity_profiles_supply_head_skins() {
        let value = "eyJ0ZXh0dXJlcyI6IHsiU0tJTiI6IHsidXJsIjogImh0dHBzOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlL2FiYzEyMyJ9fX0=";
        let profile = horizon_world::nbt::parse_snbt(&format!(
            r#"{{name:"T",id:[I;-158327916,1572358581,-2125825222,-487948715],properties:[{{name:"textures",value:"{value}"}}]}}"#
        ))
        .unwrap()
        .0;
        assert_eq!(
            super::profile_skin_url_nbt(&profile).as_deref(),
            Some("https://textures.minecraft.net/texture/abc123")
        );
        assert_eq!(
            super::profile_uuid_nbt(&profile),
            Some(
                [-158327916, 1572358581, -2125825222, -487948715]
                    .iter()
                    .fold(0u128, |a, &w: &i32| (a << 32) | u128::from(w as u32))
            )
        );
        // A name-only profile (a string tag) has no properties: no url, no id.
        let named = horizon_world::nbt::Tag::String("MHF_Blaze".into());
        assert_eq!(super::profile_skin_url_nbt(&named), None);
        assert_eq!(super::profile_uuid_nbt(&named), None);
        // A uuid in string form decodes too.
        let text_id = horizon_world::nbt::parse_snbt(
            r#"{name:"T",id:"e843a1f4-0000-0001-0000-000200000003"}"#,
        )
        .unwrap()
        .0;
        assert_eq!(
            super::profile_uuid_nbt(&text_id),
            Some(u128::from_str_radix("e843a1f4000000010000000200000003", 16).unwrap())
        );
    }

    #[test]
    fn sha1_and_base64() {
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
        assert_eq!(base64_decode("aGk").unwrap(), b"hi");
        assert!(base64_decode("a").is_none());
        assert!(base64_decode("aGk=x").is_none());
        assert_eq!(
            texture_hash("http://textures.minecraft.net/texture/abc.png?x=1").unwrap(),
            "abc"
        );
    }
}
