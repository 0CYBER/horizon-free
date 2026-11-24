use super::*;
use std::path::{Path, PathBuf};

thread_local! {
    static STRIP_DECODED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub(super) fn record_strip_decoded(bytes: usize) {
    STRIP_DECODED_BYTES.with(|total| total.set(total.get().saturating_add(bytes)));
}
thread_local! {
    static FONT_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub(super) fn record_font_read() {
    FONT_READS.with(|reads| reads.set(reads.get() + 1));
}
/// A pack's fonts as files: `(id, providers)`.
fn font_pack(fonts: &[(String, serde_json::Value)]) -> ResourceSource {
    use horizon_assets::resource_source::Resource;
    ResourceSource::indexed(
        fonts
            .iter()
            .map(|(id, providers)| {
                let file = serde_json::json!({ "providers": providers });
                (
                    resource_path(id, "font", ".json"),
                    Resource::bytes(file.to_string().into_bytes()),
                )
            })
            .collect(),
    )
}
/// The providers `id` expands to and how many font files that read.
fn expand_counting(source: &ResourceSource, id: &str) -> (Vec<serde_json::Value>, usize) {
    FONT_READS.with(|reads| reads.set(0));
    let out = providers(source, id);
    (out, FONT_READS.with(std::cell::Cell::get))
}
fn space(c: &str) -> serde_json::Value {
    serde_json::json!({ "type": "space", "advances": { c: 4 } })
}
fn reference(id: &str) -> serde_json::Value {
    serde_json::json!({ "type": "reference", "id": id })
}
/// The first advance character of each space provider.
fn advances(out: &[serde_json::Value]) -> Vec<&str> {
    out.iter()
        .map(|p| {
            p["advances"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .as_str()
        })
        .collect()
}
/// `l0` to `l{levels}`: each level names the next `fan` times, the last holds one space provider.
fn fan_out(fan: usize, levels: usize) -> Vec<(String, serde_json::Value)> {
    let mut fonts: Vec<(String, serde_json::Value)> = (0..levels)
        .map(|k| {
            let next = reference(&format!("minecraft:l{}", k + 1));
            (
                format!("minecraft:l{k}"),
                serde_json::Value::Array(vec![next; fan]),
            )
        })
        .collect();
    fonts.push((
        format!("minecraft:l{levels}"),
        serde_json::json!([space(" ")]),
    ));
    fonts
}

#[test]
fn each_referenced_font_is_read_once_however_often_it_is_named() {
    // Every copy is still listed (vanilla lists a font once per reference), but each file is
    // read once: one read per level, not one per path to it (40 and 121 before).
    let (three, reads) = expand_counting(&font_pack(&fan_out(3, 3)), "minecraft:l0");
    assert_eq!((three.len(), reads), (27, 4));
    let (four, reads) = expand_counting(&font_pack(&fan_out(3, 4)), "minecraft:l0");
    assert_eq!((four.len(), reads), (81, 5));
}

#[test]
fn spellings_of_one_font_are_one_file() {
    let source = font_pack(&[
        (
            "minecraft:default".into(),
            serde_json::json!([
                reference("x"),
                reference("minecraft:x"),
                reference("x.json")
            ]),
        ),
        ("minecraft:x".into(), serde_json::json!([space("a")])),
        (
            "minecraft:loop".into(),
            serde_json::json!([reference("loop.json"), space("b")]),
        ),
    ]);
    let (out, reads) = expand_counting(&source, "default");
    assert_eq!((advances(&out), reads), (vec!["a", "a", "a"], 2));
    // A reference to itself under another spelling is a cycle, not nine nested copies.
    let (out, reads) = expand_counting(&source, "minecraft:loop");
    assert_eq!((advances(&out), reads), (vec!["b"], 1));
}

#[test]
fn a_font_keeps_all_its_own_providers_past_the_reference_budget() {
    let mut own: Vec<serde_json::Value> =
        (0..=MAX_REFERENCED_PROVIDERS).map(|_| space("a")).collect();
    own.push(reference("minecraft:z"));
    let source = font_pack(&[
        ("minecraft:default".into(), serde_json::Value::Array(own)),
        ("minecraft:z".into(), serde_json::json!([space("z")])),
    ]);
    let (out, _) = expand_counting(&source, "minecraft:default");
    assert_eq!(out.len(), MAX_REFERENCED_PROVIDERS + 2);
    assert_eq!(advances(&out).last(), Some(&"z"));
}

#[test]
fn a_deep_fan_out_stops_at_the_reference_budget() {
    // 10^8 copies of the last level's provider without a bound.
    let mut fonts = fan_out(10, 8);
    fonts[0].1.as_array_mut().unwrap().push(space("end"));
    let (out, reads) = expand_counting(&font_pack(&fonts), "minecraft:l0");
    assert_eq!(reads, 9);
    assert!(out.len() <= MAX_REFERENCED_PROVIDERS + 1, "{}", out.len());
    // The font's own provider after its references stays.
    assert_eq!(advances(&out).last(), Some(&"end"));
}

#[test]
fn a_font_that_references_itself_keeps_its_other_providers() {
    let source = font_pack(&[
        (
            "minecraft:default".into(),
            serde_json::json!([
                reference("minecraft:default"),
                reference("minecraft:loop"),
                { "type": "reference", "id": "minecraft:space", "filter": { "uniform": false } },
                space("a"),
            ]),
        ),
        (
            "minecraft:loop".into(),
            serde_json::json!([reference("default"), space("b")]),
        ),
        ("minecraft:space".into(), serde_json::json!([space("c")])),
    ]);
    let (out, _) = expand_counting(&source, "minecraft:default");
    // The cycles add nothing; `loop`'s own provider and the filtered reference stay, in order.
    assert_eq!(advances(&out), ["b", "c", "a"]);
    assert_eq!(out[1]["filter"], serde_json::json!({ "uniform": false }));
}

#[test]
fn a_filtered_reference_to_a_font_with_a_provider_that_is_not_an_object_loads() {
    let source = font_pack(&[
        (
            "minecraft:default".into(),
            serde_json::json!([{ "type": "reference", "id": "x", "filter": { "jp": true } }]),
        ),
        (
            "minecraft:x".into(),
            serde_json::json!(["not a provider", space("a")]),
        ),
    ]);
    let (out, _) = expand_counting(&source, "minecraft:default");
    assert_eq!(out[1]["filter"], serde_json::json!({ "jp": true }));
}

fn measure_decoded_bytes<R>(f: impl FnOnce() -> R) -> (R, usize) {
    STRIP_DECODED_BYTES.with(|total| total.set(0));
    let out = f();
    let decoded = STRIP_DECODED_BYTES.with(std::cell::Cell::get);
    (out, decoded)
}
fn parse_hex_table(chunks: &[&[u8]]) -> HexTable {
    let mut entries = Vec::new();
    let mut rows = Vec::new();
    let mut by_cp = crate::fast_hash::FxHashMap::default();
    let mut has_dead_rows = false;
    for chunk in chunks {
        parse_hex(
            chunk,
            &mut entries,
            &mut rows,
            &mut by_cp,
            &mut has_dead_rows,
        );
    }
    entries.sort_unstable_by_key(|e| e.codepoint);
    if has_dead_rows {
        let total: usize = entries
            .iter()
            .map(|e| 16 * usize::from(e.bytes_per_row))
            .sum();
        let mut compacted = Vec::with_capacity(total);
        for e in &mut entries {
            let len = 16 * usize::from(e.bytes_per_row);
            let start = e.offset as usize;
            e.offset = compacted.len() as u32;
            compacted.extend_from_slice(&rows[start..start + len]);
        }
        rows = compacted;
    }
    entries.shrink_to_fit();
    rows.shrink_to_fit();
    HexTable {
        entries,
        rows,
        wide_bounds: Vec::new(),
    }
}

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn zip_with_corrupt_member(entries: &[(&str, &[u8])], corrupt_name: &str) -> Vec<u8> {
    use std::io::{Cursor, Write};
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(*name, deflated).unwrap();
        writer.write_all(bytes).unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    let (start, len) = {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let index = archive.index_for_name(corrupt_name).unwrap();
        let entry = archive.by_index_raw(index).unwrap();
        (
            entry.data_start().unwrap() as usize,
            entry.compressed_size() as usize,
        )
    };
    assert!(len > 0);
    bytes[start..start + len].fill(0xff);
    bytes
}

#[test]
fn unihex_rows_and_widths() {
    // 'A' from unifont (8 wide), a 16-wide glyph with an empty left column,
    // 24-wide, 32-wide, and an all-zero 8-wide blank glyph.
    let wide16 = format!("4E00:{}7FFE{}\n", "0000".repeat(4), "0000".repeat(11));
    let wide24 = format!(
        "1F600:{}00FF00{}\n",
        "000000".repeat(2),
        "000000".repeat(13)
    );
    let wide32 = format!("1F601:0000000000000001{}\n", "00000000".repeat(14));
    let blank8 = format!("0020:{}\n", "00".repeat(16));
    let table = parse_hex_table(&[
        b"0041:0000000018242442427E424242420000\n",
        wide16.as_bytes(),
        wide24.as_bytes(),
        wide32.as_bytes(),
        blank8.as_bytes(),
        // Unparseable lines are skipped.
        b"zz:00\n",
    ]);
    assert_eq!(table.entries.len(), 5);
    let a = table.get(0x41).unwrap();
    assert_eq!((a.left, a.right), (1, 6));
    assert_eq!(a.rows[4], 0x18 << 24);
    assert_eq!(table.advance(0x41), Some(hex_advance(&a)));
    let yi = table.get(0x4E00).unwrap();
    assert_eq!((yi.left, yi.right), (1, 14));
    assert_eq!(yi.rows[4], 0x7FFE << 16);
    assert_eq!(table.advance(0x4E00), Some(hex_advance(&yi)));
    let g24 = table.get(0x1F600).unwrap();
    assert_eq!((g24.left, g24.right), (8, 15));
    assert_eq!(g24.rows[2], 0x00FF00 << 8);
    assert_eq!(table.advance(0x1F600), Some(hex_advance(&g24)));
    let g32 = table.get(0x1F601).unwrap();
    assert_eq!((g32.left, g32.right), (31, 31));
    assert_eq!(g32.rows[1], 1);
    assert_eq!(table.advance(0x1F601), Some(hex_advance(&g32)));
    let sp = table.get(0x20).unwrap();
    assert_eq!((sp.left, sp.right), (0, 8));
    assert_eq!(table.advance(0x20), Some(hex_advance(&sp)));
    assert!(table.get(0x9999).is_none());
    assert!(table.advance(0x9999).is_none());
}

#[test]
fn glyph_uvs_have_the_native_texel_inset() {
    for (x, y, w, h) in [(120, 48, 8, 8), (0, 0, 1, 16), (4000, 2000, 32, 16)] {
        let r = glyph_rect(x, y, w, h, (4096, 4096));
        assert_eq!((r.w, r.h), (w, h));
        assert!(r.u0 > x as f32 / 4096.0 && r.v0 > y as f32 / 4096.0);
        assert!(r.u1 < (x + w) as f32 / 4096.0 && r.v1 < (y + h) as f32 / 4096.0);
        assert_eq!(r.u0, (x as f32 + 0.01) / 4096.0);
        assert_eq!(r.v1, ((y + h) as f32 - 0.01) / 4096.0);
    }
}

/// A resource root with these font definitions and files.
fn root(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-font-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    root
}
fn white() -> Rect {
    Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 0.0,
        v1: 0.0,
        w: 1,
        h: 1,
    }
}
fn load(root: &Path) -> Font {
    Font::load(
        &ResourceSource::directory(root.to_owned()),
        &HashMap::new(),
        &[],
        (4096, 1024),
        (512, 512),
        white(),
    )
}
fn png(width: u32, height: u32, opaque: impl Fn(u32, u32) -> bool) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend(if opaque(x, y) { [255; 4] } else { [0; 4] });
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    out
}

#[test]
fn archive_fonts_keep_lazy_unihex_on_the_open_reload_snapshot() {
    use std::io::{Cursor, Write};
    let root = std::env::temp_dir().join(format!("horizon-font-archive-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let write = |path: &Path, bits: &str| {
        let mut unifont = zip::ZipWriter::new(Cursor::new(Vec::new()));
        unifont
            .start_file("glyphs.hex", zip::write::SimpleFileOptions::default())
            .unwrap();
        unifont
            .write_all(format!("0041:{}\n", bits.repeat(16)).as_bytes())
            .unwrap();
        let unifont = unifont.finish().unwrap().into_inner();
        let image = png(4, 8, |_, _| true);
        let mut pack = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in [
            ("assets/minecraft/font/default.json", br#"{"providers":[{"type":"reference","id":"demo:nested/bitmap"}]}"#.as_slice()),
            ("assets/demo/font/nested/bitmap.json", br#"{"providers":[{"type":"bitmap","file":"demo:font/sheet.png","ascent":7,"chars":["a"]}]}"#),
            ("assets/demo/font/hex.json", br#"{"providers":[{"type":"unihex","hex_file":"demo:font/unifont.zip"}]}"#),
            ("assets/demo/font/type.json", br#"{"providers":[{"type":"ttf","file":"demo:mono.ttf","size":11}]}"#),
            ("assets/demo/font/mono.ttf", horizon_theme::fonts::GEIST_MONO),
            ("assets/demo/font/unifont.zip", unifont.as_slice()),
            ("assets/demo/textures/font/sheet.png", image.as_slice()),
        ] {
            pack.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            pack.write_all(bytes).unwrap();
        }
        pack.finish().unwrap();
    };
    let jar = root.join("fonts.jar");
    write(&jar, "FF");
    let source = ResourceSource::archive(&jar).unwrap();
    let font = Font::load(
        &source,
        &HashMap::new(),
        &[],
        (4096, 1024),
        (512, 512),
        white(),
    );
    let metrics = Font::metrics(&source);
    let replacement = root.join("replacement.jar");
    write(&replacement, "F0");
    std::fs::rename(&jar, root.join("old.jar")).unwrap();
    std::fs::rename(replacement, &jar).unwrap();
    assert_eq!(
        font.glyph('a').advance,
        5.0,
        "bitmap font reference in another namespace"
    );
    assert!(
        font.glyph_in("demo:type", 'A').rect.is_some(),
        "archive TrueType data"
    );
    assert_eq!(
        font.glyph_in("demo:hex", 'A').advance,
        5.0,
        "first lazy read uses the old nested archive"
    );
    assert_eq!(metrics.glyph_in("demo:hex", 'A').advance, 5.0);
    assert!(metrics.glyph_in("demo:hex", 'A').rect.is_none());
    let reloaded_source = ResourceSource::archive(&jar).unwrap();
    let reloaded = Font::metrics(&reloaded_source);
    assert_eq!(reloaded.glyph_in("demo:hex", 'A').advance, 3.0);
    assert!(!root.join("assets").exists());
    drop((font, metrics, reloaded, source, reloaded_source));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn server_metrics_preserve_bitmap_true_type_and_unihex_advances() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let def = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let ttf =
        br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2}]}"#;
    let root = root(
        "metrics",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/font/alt.json", ttf),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let rendered = load(&root);
    let metrics = Font::metrics(&ResourceSource::directory(root.clone()));
    assert_eq!(metrics.width("a b"), 14.0);
    assert_eq!(metrics.width("§lab"), 12.0);
    let text = Text::literal("ab ab");
    assert_eq!(
        metrics.split_formatted(&text, 10.0),
        rendered.split_formatted(&text, 10.0)
    );
    let alt = Style {
        font: Some("minecraft:alt".into()),
        bold: Some(true),
        ..Style::default()
    };
    for c in ['A', 'i', ' ', '\u{fffd}'] {
        assert_eq!(metrics.advance(c, &alt), rendered.advance(c, &alt));
        assert!(metrics.glyph_styled(c, &alt).rect.is_none());
    }
    let hex = parse_hex_table(&[b"0041:0000000018242442427E424242420000\n"]);
    let h = hex.get(0x41).unwrap();
    let glyph = metrics
        .rasterize_hex(&mut metrics.cache.borrow_mut().region, h)
        .unwrap();
    assert_eq!(glyph.advance, 4.0);
    assert_eq!(glyph.offset, 0.5);
    assert!(glyph.rect.is_none());
    assert!(metrics.take_patches().is_empty());
    assert!(!rendered.take_patches().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn true_type_glyphs_are_rasterized_at_the_provider_size() {
    let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2,"shift":[0.5,1],"skip":["B"]}]}"#;
    let root = root(
        "ttf",
        &[
            ("assets/minecraft/font/default.json", def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let a = font.glyph('A');
    let rect = a.rect.expect("A is drawn");
    // Oversampled 2x: the bitmap is twice the glyph's GUI size.
    assert_eq!(rect.w as f32 / 2.0, a.right - a.left);
    assert_eq!(rect.h as f32 / 2.0, a.bottom - a.top);
    // A monospaced font: every advance is the same, in half pixels.
    assert_eq!(a.advance, font.glyph('i').advance);
    assert_eq!((a.advance * 2.0).fract(), 0.0);
    assert!(a.advance > 4.0 && a.advance < 9.0, "{}", a.advance);
    // Capitals sit on the baseline at y = 7, shifted down a pixel.
    assert!((a.bottom - 8.0).abs() <= 0.5, "{}", a.bottom);
    assert_eq!(a.offset, 1.0);
    // The space has an advance and nothing to draw.
    let space = font.glyph(' ');
    assert!(space.rect.is_none() && space.advance > 0.0);
    // Skipped characters fall through to the missing glyph.
    assert_eq!(font.glyph('B').advance, 6.0);
    // Coverage as `rrrr` texels, faint coverage cut.
    let patches = font.take_patches();
    assert_eq!(patches.len(), 2);
    let texels: Vec<&[u8]> = patches.iter().flat_map(|p| p.rgba.chunks(4)).collect();
    assert!(texels.iter().all(|t| t.iter().all(|v| *v == t[0])));
    assert!(texels.iter().all(|t| t[0] == 0 || t[0] >= 25));
    assert!(texels.iter().any(|t| t[0] == 255));
}

/// A pack's `size` and `oversample` are unbounded: a glyph larger than the atlas can hold is
/// refused before its bitmap is allocated, and falls to the missing glyph.
#[test]
fn a_true_type_glyph_too_big_for_the_atlas_is_refused_before_it_is_drawn() {
    let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":100000}]}"#;
    let root = root(
        "ttf-huge",
        &[
            ("assets/minecraft/font/default.json", def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let a = font.glyph('A');
    assert!(a.rect.is_none());
    assert_eq!(a.advance, 6.0, "the missing glyph");
    assert!(font.take_patches().is_empty());
}

/// A `space` advance too big for an `f32` would be infinite: the glyph is left out.
#[test]
fn a_space_advance_too_big_for_a_float_is_ignored() {
    let def = br#"{"providers":[{"type":"space","advances":{"a":1e300,"b":3}}]}"#;
    let root = root("space-huge", &[("assets/minecraft/font/default.json", def)]);
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(font.glyph('a').advance, 6.0, "the missing glyph");
    assert_eq!(font.glyph('b').advance, 3.0);
}

/// A bitmap provider's `file` can name any texture. When the atlas has that
/// texture (HUD packs keep their glyph sheets outside `textures/font`), the
/// glyphs are read from it instead of being copied into the small run-time
/// region one by one; the region then cannot fill up and turn later glyphs
/// into the missing one, whose advance shifts the rest of a HUD line.
#[test]
fn a_bitmap_sheet_outside_font_is_read_from_the_atlas() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let (_, _, pixels) = crate::hud::decode_bytes(&sheet).unwrap();
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 8,
    };
    for (ns, file, key) in [
        ("minecraft", "minecraft:hud/sheet.png", "hud/sheet"),
        (
            "hudengine",
            "hudengine:font/sheet.png",
            "hudengine:font/sheet",
        ),
    ] {
        let def = format!(
            r#"{{"providers":[{{"type":"bitmap","file":"{file}","ascent":7,"chars":["ab"]}}]}}"#
        );
        // The image is only in the atlas: the pack's copy is not on disk.
        let root = root(
            ns,
            &[("assets/minecraft/font/default.json", def.as_bytes())],
        );
        let sprites = HashMap::from([(key.to_string(), rect)]);
        let font = Font::load(
            &ResourceSource::directory(root.clone()),
            &sprites,
            &pixels,
            (16, 8),
            (0, 8),
            white(),
        );
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 3.0, "{ns}");
        assert_eq!(font.glyph('b').advance, 7.0, "{ns}");
        assert!(font.glyph('a').rect.is_some(), "{ns}");
        assert!(
            font.take_patches().is_empty(),
            "{ns}: the atlas sheet is not copied into the run-time region"
        );
    }
}

#[test]
fn styles_pick_their_font_and_unknown_fonts_draw_missing_glyphs() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let alt = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ba"]}]}"#;
    let root = root(
        "styles",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/font/alt.json", alt),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    // The sheet is not in this atlas: glyphs are copied in when first drawn.
    assert_eq!(font.glyph('a').advance, 3.0);
    assert_eq!(font.glyph('b').advance, 7.0);
    let alt = Style {
        font: Some("minecraft:alt".into()),
        ..Style::default()
    };
    assert_eq!(font.glyph_styled('a', &alt).advance, 7.0);
    assert_eq!(font.glyph_in("alt", 'b').advance, 3.0);
    assert!(font.glyph_styled('a', &alt).rect.is_some());
    // Not in alt (no space provider there), or no such font at all.
    assert_eq!(font.glyph_styled(' ', &alt).advance, 6.0);
    let missing = Style {
        font: Some("minecraft:nope".into()),
        ..Style::default()
    };
    assert_eq!(font.glyph_styled(' ', &missing).advance, 6.0);
    assert_eq!(font.advance('a', &alt), 7.0);
    let text = Text::literal("ab").with_style(&alt);
    assert_eq!(font.width_text(&text), 10.0);
    assert_eq!(font.random_glyph(&alt, 7.0).advance, 7.0);
    assert!(!font.take_patches().is_empty());
}

/// `FontSet.computeGlyphInfo`: `SelectedGlyphs.any` is the first provider's glyph, fishy
/// advance or not, and is what is drawn and measured; `nonFishy`, what obfuscated text draws,
/// is the first provider whose advance is within 0..32, else the missing glyph.
#[test]
fn a_fishy_glyph_is_drawn_and_only_obfuscated_text_falls_through() {
    // 64 opaque columns: the first provider's 'a' advances 65, past the 32 vanilla
    // calls fishy, and the narrow provider's 'a' advances 3.
    let wide = png(64, 8, |_, _| true);
    let narrow = png(16, 8, |x, _| x < 2);
    let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/wide.png","ascent":7,"chars":["a"]},{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}]}"#;
    let root = root(
        "fishy",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/textures/font/wide.png", &wide),
            ("assets/minecraft/textures/font/narrow.png", &narrow),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let drawn = font.glyph('a');
    assert_eq!(drawn.advance, 65.0, "the first provider's glyph is drawn");
    assert!(drawn.rect.is_some());
    assert_eq!(font.advance('a', &Style::default()), 65.0);
    // The by-width table files 'a' under its first glyph's 65; the glyph it then draws is
    // the narrow provider's (`nonFishy`).
    assert_eq!(font.random_glyph(&Style::default(), 65.0).advance, 3.0);
    // Nothing else is 3 wide in the table, and the missing glyph is 6.
    assert_eq!(font.random_glyph(&Style::default(), 3.0).advance, 6.0);
}

/// A `space` provider's shift glyphs (kerning in server packs: -512 to 512) are drawn and
/// measured as they are, and `nonFishy` has nothing else to draw for them.
#[test]
fn space_glyphs_with_any_advance_are_kept() {
    let shifts: Vec<i32> = (0..10)
        .map(|i| -(1 << i))
        .chain((0..10).map(|i| 1 << i))
        .collect();
    let advances: Vec<String> = shifts
        .iter()
        .enumerate()
        .map(|(i, a)| format!("\"\\ue{:03x}\": {a}", i))
        .collect();
    let def = format!(
        r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
        advances.join(",")
    );
    let root = root(
        "space-shifts",
        &[("assets/minecraft/font/default.json", def.as_bytes())],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    for (i, a) in shifts.iter().enumerate() {
        let c = char::from_u32(0xe000 + i as u32).unwrap();
        assert_eq!(font.glyph(c).advance, *a as f32, "{c:?}");
        assert_eq!(font.advance(c, &Style::default()), *a as f32);
    }
    // Obfuscated text asks for a glyph of a given width: a fishy space has none to give.
    let c = char::from_u32(0xe000 + 19).unwrap();
    assert_eq!(font.glyph(c).advance, 512.0);
    assert_eq!(
        font.glyph_non_fishy(DEFAULT, c).advance,
        6.0,
        "the missing glyph"
    );
    assert_eq!(font.random_glyph(&Style::default(), 512.0).advance, 6.0);
}

/// Nexo's GUI glyphs: a bitmap `height` of 287 with no advance, so every glyph wider than 31
/// pixels has a fishy advance, and is still drawn.
#[test]
fn a_wide_bitmap_glyph_is_drawn() {
    let sheet = png(40, 287, |_, _| true);
    let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/gui.png","height":287,"ascent":65,"chars":["a"]}]}"#;
    let root = root(
        "wide-bitmap",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/textures/font/gui.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let g = font.glyph('a');
    assert_eq!(g.advance, 41.0, "the glyph's 40 columns and one");
    assert!(g.rect.is_some(), "drawn, not the missing glyph");
    assert!(fishy_advance(g.advance));
    assert_eq!(
        font.glyph_non_fishy(DEFAULT, 'a').advance,
        6.0,
        "excluded from what obfuscated text draws"
    );
}

/// Server packs pair a cell with its copy at a negative `height` so the two advance a fixed
/// amount whatever the cell's width: `(w + 1) + ((int) (0.5 - w) + 1)` is 3 in vanilla.
#[test]
fn a_negative_height_cell_advances_as_vanilla_truncates() {
    let sheet = png(8, 8, |x, _| x < 4);
    let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/cell.png","height":8,"ascent":7,"chars":["a"]},{"type":"bitmap","file":"minecraft:font/cell.png","height":-8,"ascent":-32768,"chars":["b"]}]}"#;
    let root = root(
        "negative-height",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/textures/font/cell.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(font.glyph('a').advance, 5.0);
    assert_eq!(font.glyph('b').advance, -2.0);
}

/// A font whose first provider is `first` (its sheet `sheet`), then a
/// valid provider drawing `a` from a sheet with two columns used.
fn font_after(name: &str, first: &str, sheet: &[u8]) -> Font {
    let narrow = png(8, 8, |x, _| x < 2);
    let def = format!(
        r#"{{"providers":[{first},{{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}}]}}"#
    );
    let root = root(
        name,
        &[
            ("assets/minecraft/font/default.json", def.as_bytes()),
            ("assets/minecraft/textures/font/first.png", sheet),
            ("assets/minecraft/textures/font/narrow.png", &narrow),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    font
}

/// `BitmapProvider.Definition` refuses these while the file decodes, and
/// the whole file goes. Rows of unequal length were sized from the first
/// row, so the longer row's last cell reached past the sheet and panicked
/// when drawn.
#[test]
fn a_font_file_with_a_refused_bitmap_provider_is_skipped() {
    let sheet = png(16, 16, |_, _| true);
    for (name, first) in [
        ("uneven", r#""ascent":7,"chars":["ab","cdef"]"#),
        (
            "ascent-above-height",
            r#""height":8,"ascent":9,"chars":["af"]"#,
        ),
        ("text-ascent", r#""ascent":"7","chars":["af"]"#),
        ("no-ascent", r#""chars":["af"]"#),
        ("empty-row", r#""ascent":7,"chars":["","af"]"#),
    ] {
        let first = format!(r#"{{"type":"bitmap","file":"minecraft:font/first.png",{first}}}"#);
        let font = font_after(name, &first, &sheet);
        let missing = font.glyph('z');
        for c in ['a', 'f'] {
            let g = font.glyph(c);
            assert_eq!(
                g.advance, missing.advance,
                "{name}: {c} is the missing glyph"
            );
            assert_eq!(g.rect.map(|r| r.u0), missing.rect.map(|r| r.u0), "{name}");
        }
    }
}

/// Vanilla reads `height` and `ascent` as integers, a fraction truncated
/// toward zero: 8.9 and 6.5 load as 8 and 6.
#[test]
fn fractional_height_and_ascent_are_truncated() {
    let sheet = png(16, 16, |_, _| true);
    let first = r#"{"type":"bitmap","file":"minecraft:font/first.png","height":8.9,"ascent":6.5,"chars":["af"]}"#;
    let g = font_after("fractional", first, &sheet).glyph('f');
    // Cells 8x16 at height 8: half scale; top 7 - 6.
    assert_eq!((g.top, g.bottom), (1.0, 9.0));
    assert_eq!(g.advance, 5.0, "8 columns used, at half scale, and one");
}

/// `BitmapProvider.Definition.load` puts each cell in a map: a character
/// listed twice keeps its last cell. Character 0 is padding, and an earlier
/// provider still wins over a later one.
#[test]
fn a_repeated_character_keeps_its_last_cell() {
    // Cells 8 wide using 2, 6 and 4 columns: advances 3, 7 and 5.
    let sheet = png(24, 8, |x, _| {
        x < 2 || (8..14).contains(&x) || (16..20).contains(&x)
    });
    let first =
        r#"{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":["a\u0000a"]}"#;
    let font = font_after("repeated", first, &sheet);
    assert_eq!(
        font.glyph('a').advance,
        5.0,
        "the last cell, not the first (3) or the next provider's (3)"
    );
    assert_eq!(
        font.glyph('\0').advance,
        font.glyph('z').advance,
        "character 0 is not the middle cell (7)"
    );
}

/// More rows or columns than the sheet has pixels leave cells of no size.
/// Vanilla keeps the provider: each character is an empty glyph of advance
/// 1, and a later provider's glyph for it isn't used.
#[test]
fn a_grid_larger_than_its_sheet_draws_empty_glyphs() {
    for (name, sheet, chars) in [
        ("too-many-rows", png(16, 1, |_, _| true), r#"["a","b"]"#),
        ("too-many-columns", png(4, 8, |_, _| true), r#"["abcde"]"#),
    ] {
        let first = format!(
            r#"{{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":{chars}}}"#
        );
        let g = font_after(name, &first, &sheet).glyph('a');
        assert_eq!(g.advance, 1.0, "{name}: not the next provider's 3");
        assert!(g.rect.is_none(), "{name}: nothing drawn");
        assert!(g.top.is_finite() && g.bottom.is_finite(), "{name}");
    }
}

/// StringDecomposer.iterateFormatted: a `§` that ends a part stops that
/// part only; the next part is still measured and can break.
#[test]
fn a_section_sign_ending_a_part_still_breaks_the_next() {
    let sheet = png(16, 8, |x, _| x < 2);
    let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let root = root(
        "section-sign",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let text = Text::from_spans(
        ["a§", "a a"]
            .map(|part| Span::new(part, Style::default()))
            .into(),
    );
    // 3 + 3 + 4 + 3 is past 12: the line breaks at the space.
    let lines: Vec<String> = font
        .split_formatted(&text, 12.0)
        .iter()
        .map(Text::plain)
        .collect();
    assert_eq!(lines, ["a§a", "a"]);
    let mixed = Text::literal("aa")
        .append(Text::untrusted("aa"))
        .with_style(&Style {
            click: Some(crate::text::ClickEvent::OpenFile("/tmp/local".into())),
            ..Style::default()
        });
    let lines = font.split_formatted(&mixed, 6.0);
    assert_eq!(
        lines.iter().map(Text::plain).collect::<Vec<_>>(),
        ["aa", "aa"]
    );
    assert_eq!(lines[0].spans[0].origin(), crate::text::Origin::Local);
    assert_eq!(lines[1].spans[0].origin(), crate::text::Origin::Remote);
    assert!(lines[0].spans[0].click_event().is_some());
    assert!(lines[1].spans[0].click_event().is_none());
}

/// A renderer rebuilt over the same font (the loading screen's handoff)
/// makes its atlas texture from the textures alone, so the glyphs it had
/// rasterized into the previous texture must be placed into the new one.
#[test]
fn a_rebuilt_atlas_gets_its_glyphs_again() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let default = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let root = root(
        "rebuild",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let first = font.glyph('a');
    assert_eq!(font.take_patches().len(), 1);
    font.reset_cache();
    // Nothing is left over: drawing the character again places its pixels.
    assert!(font.take_patches().is_empty());
    assert_eq!(font.glyph('a').advance, first.advance);
    assert_eq!(font.take_patches().len(), 1);
}

#[test]
fn provider_filters_follow_the_font_options() {
    let sheet = png(8, 8, |x, _| x < 4);
    let default = br#"{"providers":[{"type":"reference","id":"minecraft:include/sheet","filter":{"uniform":false}},{"type":"space","advances":{"a":2}}]}"#;
    let sheet_def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/s.png","ascent":7,"chars":["a"]}]}"#;
    let root = root(
        "filters",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/font/include/sheet.json", sheet_def),
            ("assets/minecraft/textures/font/s.png", &sheet),
        ],
    );
    let mut font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(font.glyph('a').advance, 5.0);
    font.uniform = true;
    font.reset_cache();
    assert_eq!(font.glyph('a').advance, 2.0);
}

#[test]
fn a_full_glyph_region_recovers_at_the_next_frame() {
    // Two 8x8 glyphs from a sheet outside the atlas; the region holds one row.
    let sheet = png(16, 8, |x, _| x % 8 < 4);
    let default =
        br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["ab"]}]}"#;
    let root = root(
        "full",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/pack/textures/font/s.png", &sheet),
        ],
    );
    // A 10 wide atlas fits one 8 pixel glyph per row; the region two rows.
    let font = Font::load(
        &ResourceSource::directory(root.clone()),
        &HashMap::new(),
        &[],
        (10, 64),
        (0, 8),
        white(),
    );
    let _ = std::fs::remove_dir_all(&root);
    let epoch = font.epoch();
    let a = font.glyph('a');
    assert!(a.rect.is_some());
    // No room for 'b': missing for now, and asked again next time.
    assert!(font.glyph('b').rect.is_none());
    assert!(font.glyph('b').rect.is_none());
    assert_eq!(font.epoch(), epoch);
    font.begin_frame();
    assert_ne!(font.epoch(), epoch);
    let b = font.glyph('b');
    assert!(b.rect.is_some());
    assert_eq!(b.advance, a.advance);
    // Nothing overflowed since: the next frame keeps the glyphs.
    let epoch = font.epoch();
    font.begin_frame();
    assert_eq!(font.epoch(), epoch);
}

#[test]
fn a_character_without_a_provider_does_not_reset_the_region() {
    let sheet = png(8, 8, |x, _| x < 4);
    let default =
        br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["a"]}]}"#;
    let root = root(
        "nofill",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/pack/textures/font/s.png", &sheet),
        ],
    );
    let font = Font::load(
        &ResourceSource::directory(root.clone()),
        &HashMap::new(),
        &[],
        (10, 64),
        (0, 8),
        white(),
    );
    let _ = std::fs::remove_dir_all(&root);
    let epoch = font.epoch();
    assert!(font.glyph('z').rect.is_none());
    font.begin_frame();
    assert_eq!(font.epoch(), epoch);
}

fn rgba_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
    out
}

fn cell_rgba(pixels: &[u8], stride: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for dy in 0..h {
        let start = (((y + dy) * stride + x) * 4) as usize;
        out.extend_from_slice(&pixels[start..start + (w * 4) as usize]);
    }
    out
}

fn off_atlas_strip_bytes(font: &Font) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut total = 0;
    for (_, set) in &font.cache.borrow().sets {
        for provider in &set.providers {
            if let Provider::Bitmap(_, glyphs) = provider {
                for glyph in glyphs.values() {
                    if let BitmapGlyph::Sheet(cell) = glyph
                        && seen.insert(cell.sheet.id)
                    {
                        total += cell.sheet.stored_bytes();
                    }
                }
            }
        }
    }
    total
}

#[test]
fn off_atlas_bitmap_strips_upload_exact_pixels_and_bound_decoded_work() {
    // Large-cell sheet: 8x2 grid of 64x64 cells (512x128 = 256 KiB raw).
    let (large_w, large_h) = (512u32, 128u32);
    let mut large_pixels = vec![0u8; (large_w * large_h * 4) as usize];
    for y in 0..large_h {
        for x in 0..large_w {
            let col = x / 64;
            let row = y / 64;
            let lx = x % 64;
            let ly = y % 64;
            let px = if (4..60).contains(&lx) && (4..60).contains(&ly) {
                [
                    (col * 29 + 17) as u8,
                    (row * 71 + 43) as u8,
                    ((row * 8 + col) * 13 + 9) as u8,
                    255,
                ]
            } else if (lx, ly) == (0, 0) {
                [row as u8, col as u8, 1, 255]
            } else if (lx, ly) == (63, 63) {
                [row as u8, col as u8, 2, 255]
            } else {
                [0, 0, 0, 0]
            };
            let at = ((y * large_w + x) * 4) as usize;
            large_pixels[at..at + 4].copy_from_slice(&px);
        }
    }
    let large_png = rgba_png(large_w, large_h, &large_pixels);
    let large_def = br#"{"providers":[{"type":"bitmap","file":"pack:font/large.png","height":64,"ascent":7,"chars":["abcdefgh","ijklmnop"]}]}"#;
    let large_root = root(
        "strips-large",
        &[
            ("assets/minecraft/font/default.json", large_def),
            ("assets/pack/textures/font/large.png", &large_png),
        ],
    );
    let font = load(&large_root);
    let _ = std::fs::remove_dir_all(&large_root);

    let raw_large_bytes = (large_w * large_h * 4) as usize;
    let stored_large = off_atlas_strip_bytes(&font);
    assert!(
        stored_large < raw_large_bytes / 2,
        "expected LZ4-compressed strips to shrink large sheet ({stored_large} vs {raw_large_bytes})"
    );

    let (patches, decoded_pass1) = measure_decoded_bytes(|| {
        for ch in "abcdefghijklmnop".chars() {
            assert!(font.glyph(ch).rect.is_some(), "glyph {ch} missing");
        }
        font.take_patches()
    });
    assert_eq!(patches.len(), 16);
    for (idx, patch) in patches.iter().enumerate() {
        let row = (idx / 8) as u32;
        let col = (idx % 8) as u32;
        assert_eq!((patch.width, patch.height), (64, 64));
        assert_eq!(
            patch.rgba,
            cell_rgba(&large_pixels, large_w, col * 64, row * 64, 64, 64),
            "large cell ({row}, {col}) mismatch on pass 1"
        );
    }
    assert!(
        decoded_pass1 <= raw_large_bytes,
        "resolving all 16 large cells decoded {decoded_pass1} bytes > {raw_large_bytes}"
    );

    // Cache reset re-uploads identical pixels with bounded decoded work.
    font.reset_cache();
    let (patches2, decoded_pass2) = measure_decoded_bytes(|| {
        for ch in "abcdefghijklmnop".chars() {
            assert!(font.glyph(ch).rect.is_some(), "glyph {ch} missing");
        }
        font.take_patches()
    });
    assert_eq!(patches2.len(), 16);
    for (idx, patch) in patches2.iter().enumerate() {
        let row = (idx / 8) as u32;
        let col = (idx % 8) as u32;
        assert_eq!((patch.width, patch.height), (64, 64));
        assert_eq!(
            patch.rgba,
            cell_rgba(&large_pixels, large_w, col * 64, row * 64, 64, 64),
            "large cell ({row}, {col}) mismatch on pass 2"
        );
    }
    assert!(
        decoded_pass2 <= raw_large_bytes,
        "re-resolving all 16 large cells decoded {decoded_pass2} bytes > {raw_large_bytes}"
    );

    // A single cold large-cell lookup must not decompress the whole row.
    font.reset_cache();
    let (single_patches, decoded_single) = measure_decoded_bytes(|| {
        assert!(font.glyph('d').rect.is_some());
        font.take_patches()
    });
    assert_eq!(single_patches.len(), 1);
    assert_eq!(
        single_patches[0].rgba,
        cell_rgba(&large_pixels, large_w, 3 * 64, 0, 64, 64)
    );
    assert!(
        decoded_single <= 64 * 64 * 4,
        "single large cell decoded {decoded_single} bytes > {}",
        64 * 64 * 4
    );

    // Small-cell sheet: 8x10 grid of 16x16 cells (128x160 = 80 KiB raw).
    // Interleaving two rows exercises MRU promotion; walking 10 rows exercises eviction.
    let (small_w, small_h) = (128u32, 160u32);
    let mut small_pixels = vec![0u8; (small_w * small_h * 4) as usize];
    for y in 0..small_h {
        for x in 0..small_w {
            let col = x / 16;
            let row = y / 16;
            let lx = x % 16;
            let ly = y % 16;
            let px = if (2..14).contains(&lx) && (2..14).contains(&ly) {
                [
                    (row * 23 + 11) as u8,
                    (col * 31 + 5) as u8,
                    ((row * 8 + col) * 7 + 3) as u8,
                    255,
                ]
            } else if (lx, ly) == (0, 0) {
                [row as u8, col as u8, 3, 255]
            } else if (lx, ly) == (15, 15) {
                [row as u8, col as u8, 4, 255]
            } else {
                [0, 0, 0, 0]
            };
            let at = ((y * small_w + x) * 4) as usize;
            small_pixels[at..at + 4].copy_from_slice(&px);
        }
    }
    let small_png = rgba_png(small_w, small_h, &small_pixels);
    let small_rows: Vec<String> = (0..10u32)
        .map(|row| {
            (0..8u32)
                .map(|col| char::from_u32(0xE000 + row * 8 + col).unwrap())
                .collect()
        })
        .collect();
    let small_def = serde_json::json!({
        "providers": [{
            "type": "bitmap",
            "file": "pack:font/small.png",
            "height": 16,
            "ascent": 7,
            "chars": small_rows,
        }]
    })
    .to_string();
    let small_root = root(
        "strips-small",
        &[
            ("assets/minecraft/font/default.json", small_def.as_bytes()),
            ("assets/pack/textures/font/small.png", &small_png),
        ],
    );
    let small_font = load(&small_root);
    let _ = std::fs::remove_dir_all(&small_root);

    let mut order: Vec<(u32, u32)> = Vec::with_capacity(80);
    for col in 0..8 {
        order.push((0, col));
        order.push((1, col));
    }
    for row in 2..10 {
        for col in 0..8 {
            order.push((row, col));
        }
    }
    let (small_patches, decoded_small) = measure_decoded_bytes(|| {
        for &(row, col) in &order {
            let ch = char::from_u32(0xE000 + row * 8 + col).unwrap();
            assert!(
                small_font.glyph(ch).rect.is_some(),
                "small glyph ({row}, {col}) missing"
            );
        }
        small_font.take_patches()
    });
    assert_eq!(small_patches.len(), 80);
    for (&(row, col), patch) in order.iter().zip(&small_patches) {
        assert_eq!((patch.width, patch.height), (16, 16));
        assert_eq!(
            patch.rgba,
            cell_rgba(&small_pixels, small_w, col * 16, row * 16, 16, 16),
            "small cell ({row}, {col}) mismatch"
        );
    }
    let raw_small_bytes = (small_w * small_h * 4) as usize;
    assert!(
        decoded_small <= raw_small_bytes,
        "resolving all 80 small cells decoded {decoded_small} bytes > {raw_small_bytes}"
    );
}

#[test]
fn incompressible_off_atlas_strip_stays_raw_without_decompressing() {
    let (w, h) = (64u32, 64u32);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut noise = vec![0u8; (w * h * 4) as usize];
    for chunk in noise.as_chunks_mut::<8>().0 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    let sheet = rgba_png(w, h, &noise);
    let def = br#"{"providers":[{"type":"bitmap","file":"pack:font/noise.png","height":64,"ascent":7,"chars":["a"]}]}"#;
    let noise_root = root(
        "strips-incompressible",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/pack/textures/font/noise.png", &sheet),
        ],
    );
    let font = load(&noise_root);
    let _ = std::fs::remove_dir_all(&noise_root);

    assert_eq!(
        off_atlas_strip_bytes(&font),
        noise.len(),
        "incompressible strip should store exact raw bytes with no LZ4 bloat"
    );
    for pass in 0..2 {
        let (patches, decoded) = measure_decoded_bytes(|| {
            assert!(font.glyph('a').rect.is_some());
            font.take_patches()
        });
        assert_eq!(patches.len(), 1, "pass {pass}");
        assert_eq!((patches[0].width, patches[0].height), (w, h));
        assert_eq!(patches[0].rgba, noise, "pass {pass}");
        assert_eq!(
            decoded, 0,
            "pass {pass}: incompressible strip should not waste CPU decompressing"
        );
        font.reset_cache();
    }
}

#[test]
fn corrupted_compressed_strip_warns_once_with_file_name_and_draws_empty_cell() {
    let info = Arc::new(SheetInfo::new("pack:font/broken.png"));
    let bad_stream = Arc::new(SheetStrip {
        id: next_strip_id(),
        info: Arc::clone(&info),
        stride: 4,
        cacheable: false,
        pixels: StripPixels::Lz4 {
            len: 64,
            compressed: vec![0xFF; 16],
        },
    });
    let sibling_short = Arc::new(SheetStrip {
        id: next_strip_id(),
        info,
        stride: 4,
        cacheable: false,
        pixels: StripPixels::Lz4 {
            len: 64,
            compressed: lz4_flex::block::compress(&[0xAA; 16]),
        },
    });
    let make_cell = |sheet: Arc<SheetStrip>| SheetCell {
        sheet,
        x: 0,
        y: 0,
        w: 4,
        h: 4,
        glyph: Glyph::default(),
    };
    let cell_bad = make_cell(bad_stream);
    let cell_sibling = make_cell(sibling_short);
    let mut strips = StripCache::default();

    let (rgba1, logs1) = crate::test_log::capture(|| cell_bad.pixels(&mut strips));
    assert_eq!(rgba1, Some(vec![0; 64]));
    assert_eq!(logs1.len(), 1, "expected a single warning, got {logs1:?}");
    assert!(
        logs1[0].contains("pack:font/broken.png"),
        "warning must name the sheet file: {}",
        logs1[0]
    );

    let (rgba2, logs2) = crate::test_log::capture(|| cell_bad.pixels(&mut strips));
    assert_eq!(rgba2, Some(vec![0; 64]));
    assert!(
        logs2.is_empty(),
        "same strip must not warn twice: {logs2:?}"
    );

    let (rgba3, logs3) = crate::test_log::capture(|| cell_sibling.pixels(&mut strips));
    assert_eq!(rgba3, Some(vec![0; 64]));
    assert!(
        logs3.is_empty(),
        "sibling strip of the same sheet must not warn again: {logs3:?}"
    );

    // Wrong decoded length on another sheet warns once and clears any partial bytes.
    let short_strip = Arc::new(SheetStrip {
        id: next_strip_id(),
        info: Arc::new(SheetInfo::new("pack:font/short_decode.png")),
        stride: 4,
        cacheable: false,
        pixels: StripPixels::Lz4 {
            len: 64,
            compressed: lz4_flex::block::compress(&[0xAA; 16]),
        },
    });
    let cell_short = make_cell(short_strip);
    let (rgba4, logs4) = crate::test_log::capture(|| cell_short.pixels(&mut strips));
    assert_eq!(rgba4, Some(vec![0; 64]));
    assert_eq!(logs4.len(), 1, "expected a single warning, got {logs4:?}");
    assert!(
        logs4[0].contains("pack:font/short_decode.png"),
        "warning must name the sheet file: {}",
        logs4[0]
    );

    let (rgba5, logs5) = crate::test_log::capture(|| cell_short.pixels(&mut strips));
    assert_eq!(rgba5, Some(vec![0; 64]));
    assert!(logs5.is_empty(), "must not warn twice: {logs5:?}");
}

#[test]
fn off_atlas_sheet_with_non_multiple_width_and_padding_extracts_exact_cells() {
    // 19x8 sheet with 2 columns -> cell_w = 9, chunk_stride = 18 != sheet stride 19.
    let (odd_w, odd_h) = (19u32, 8u32);
    let mut odd_pixels = vec![0u8; (odd_w * odd_h * 4) as usize];
    for y in 0..odd_h {
        for x in 0..odd_w {
            let at = ((y * odd_w + x) * 4) as usize;
            odd_pixels[at..at + 4].copy_from_slice(&[
                (x * 13 + 7) as u8,
                (y * 29 + 3) as u8,
                ((x + y) * 11) as u8,
                255,
            ]);
        }
    }
    let odd_png = rgba_png(odd_w, odd_h, &odd_pixels);
    let odd_def = br#"{"providers":[{"type":"bitmap","file":"pack:font/odd.png","height":8,"ascent":7,"chars":["ab"]}]}"#;
    let odd_root = root(
        "strips-odd-width",
        &[
            ("assets/minecraft/font/default.json", odd_def),
            ("assets/pack/textures/font/odd.png", &odd_png),
        ],
    );
    let odd_font = load(&odd_root);
    let _ = std::fs::remove_dir_all(&odd_root);
    assert!(odd_font.glyph('a').rect.is_some());
    assert!(odd_font.glyph('b').rect.is_some());
    let odd_patches = odd_font.take_patches();
    assert_eq!(odd_patches.len(), 2);
    assert_eq!((odd_patches[0].width, odd_patches[0].height), (9, 8));
    assert_eq!(
        odd_patches[0].rgba,
        cell_rgba(&odd_pixels, odd_w, 0, 0, 9, 8)
    );
    assert_eq!((odd_patches[1].width, odd_patches[1].height), (9, 8));
    assert_eq!(
        odd_patches[1].rgba,
        cell_rgba(&odd_pixels, odd_w, 9, 0, 9, 8)
    );

    // 10 cols of 32x32 with all-zero padding chunks, single-active chunks, and multi-active chunks.
    let (pad_w, pad_h) = (320u32, 32u32);
    let mut pad_pixels = vec![0u8; (pad_w * pad_h * 4) as usize];
    for y in 0..pad_h {
        for x in 0..pad_w {
            let col = x / 32;
            let lx = x % 32;
            let ly = y % 32;
            let px = if (4..28).contains(&lx) && (4..28).contains(&ly) {
                [(col * 25 + 10) as u8, (lx * 7) as u8, (ly * 7) as u8, 255]
            } else {
                [0, 0, 0, 0]
            };
            let at = ((y * pad_w + x) * 4) as usize;
            pad_pixels[at..at + 4].copy_from_slice(&px);
        }
    }
    let pad_png = rgba_png(pad_w, pad_h, &pad_pixels);
    let pad_def = br#"{"providers":[{"type":"bitmap","file":"pack:font/pad.png","height":32,"ascent":7,"chars":["\u0000\u0000\u0000ab\u0000cd\u0000\u0000"]}]}"#;
    let pad_root = root(
        "strips-padding",
        &[
            ("assets/minecraft/font/default.json", pad_def),
            ("assets/pack/textures/font/pad.png", &pad_png),
        ],
    );
    let pad_font = load(&pad_root);
    let _ = std::fs::remove_dir_all(&pad_root);
    let expected_cols = [('a', 3u32), ('b', 4), ('c', 6), ('d', 7)];
    for &(ch, _) in &expected_cols {
        assert!(pad_font.glyph(ch).rect.is_some(), "glyph {ch} missing");
    }
    let pad_patches = pad_font.take_patches();
    assert_eq!(pad_patches.len(), expected_cols.len());
    for (&(ch, col), patch) in expected_cols.iter().zip(&pad_patches) {
        assert_eq!((patch.width, patch.height), (32, 32));
        assert_eq!(
            patch.rgba,
            cell_rgba(&pad_pixels, pad_w, col * 32, 0, 32, 32),
            "padded glyph {ch} at col {col} mismatch"
        );
    }
}

#[test]
fn true_type_shared_glyph_keeps_first_codepoint_only() {
    let raw = ab_glyph::FontVec::try_from_vec(horizon_theme::fonts::GEIST_MONO.to_vec()).unwrap();
    // In GeistMono, U+00B5 ('µ') and U+03BC ('μ') share one GlyphId; `codepoint_ids()`
    // yields only the first codepoint per GlyphId (U+00B5).
    let shared = raw.glyph_id('\u{00b5}');
    assert_ne!(shared.0, 0);
    assert_eq!(raw.glyph_id('\u{03bc}'), shared);

    let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2},{"type":"space","advances":{"\u03bc":11}}]}"#;
    let root_dir = root(
        "ttf-shared-gid",
        &[
            ("assets/minecraft/font/default.json", def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
        ],
    );
    let font = load(&root_dir);
    let _ = std::fs::remove_dir_all(&root_dir);

    let micro = font.glyph('\u{00b5}');
    assert!(
        micro.rect.is_some(),
        "canonical codepoint resolves from TTF"
    );
    assert_ne!(micro.advance, 11.0);
    let mu = font.glyph('\u{03bc}');
    assert!(mu.rect.is_none(), "shared-GlyphId alias falls through");
    assert_eq!(mu.advance, 11.0);

    // Skipping the canonical codepoint drops it without promoting the alias.
    let skipped_def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2,"skip":"\u00b5"},{"type":"space","advances":{"\u00b5":9,"\u03bc":11}}]}"#;
    let skipped_root = root(
        "ttf-shared-gid-skip",
        &[
            ("assets/minecraft/font/default.json", skipped_def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
        ],
    );
    let skipped = load(&skipped_root);
    let _ = std::fs::remove_dir_all(&skipped_root);
    assert_eq!(skipped.glyph('\u{00b5}').advance, 9.0);
    assert_eq!(skipped.glyph('\u{03bc}').advance, 11.0);
}

#[test]
fn oversized_true_type_glyph_falls_through_to_following_bitmap_provider() {
    let style = Style::default();
    let sheet = png(16, 8, |x, _| x < 2 || (8..12).contains(&x));
    let oversized_def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":100000},{"type":"bitmap","file":"minecraft:font/fallback.png","ascent":7,"chars":["AB"]}]}"#;
    let oversized_root = root(
        "ttf-oversized-fallback",
        &[
            ("assets/minecraft/font/default.json", oversized_def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
            ("assets/minecraft/textures/font/fallback.png", &sheet),
        ],
    );
    let font = load(&oversized_root);
    let _ = std::fs::remove_dir_all(&oversized_root);

    // Advance lookup applies the same outline-size eligibility check as `resolve` without
    // allocating/rasterizing atlas patches: 'A' falls through to bitmap advance 3.0, while
    // empty-outline spaces still fit the TrueType provider.
    {
        let mut cache = font.cache.borrow_mut();
        let Cache { sets, unihex, .. } = &mut *cache;
        let by_width = font.glyphs_by_width(&mut sets[0].1, unihex);
        assert_eq!(
            by_width.get(&3).map(Vec::as_slice),
            Some(&['A'][..]),
            "oversized TTF glyph is bucketed under fallback bitmap advance"
        );
        assert!(
            by_width
                .iter()
                .all(|(&w, chars)| w == 3 || !chars.contains(&'A')),
            "oversized TTF glyph is not indexed under rejected TTF advance"
        );
        assert!(
            font.provider_advance(&mut sets[0].1, unihex, ' ')
                .is_some_and(|adv| adv > 32.0),
            "empty-outline space still fits the TrueType provider"
        );
    }
    assert!(
        font.take_patches().is_empty(),
        "advance lookup does not rasterize glyphs"
    );

    let obf = font.random_glyph(&style, 3.0);
    assert!(
        obf.rect.is_some(),
        "obfuscated lookup draws bitmap fallback"
    );
    assert_eq!(
        obf.advance, 3.0,
        "obfuscated lookup selects bitmap fallback under displayed width"
    );
    let a = font.glyph('A');
    assert!(a.rect.is_some(), "bitmap fallback is drawn");
    assert_eq!(a.advance, 3.0, "falls through to following bitmap provider");
    assert_eq!(font.advance('A', &style), 3.0, "advance uses fallback");
    assert_eq!(font.width("A"), 3.0, "width uses fallback");
    let space = font.glyph(' ');
    assert!(
        space.rect.is_none() && space.advance > 32.0,
        "empty-outline space resolves from TrueType"
    );

    // A fitting TrueType glyph still takes precedence over the following bitmap provider
    // for drawing, measurement, and obfuscated width bucketing.
    let fitting_def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2},{"type":"bitmap","file":"minecraft:font/fallback.png","ascent":7,"chars":["AB"]}]}"#;
    let fitting_root = root(
        "ttf-fitting-precedence",
        &[
            ("assets/minecraft/font/default.json", fitting_def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
            ("assets/minecraft/textures/font/fallback.png", &sheet),
        ],
    );
    let fitting = load(&fitting_root);
    let metrics = Font::metrics(&ResourceSource::directory(fitting_root.clone()));
    let _ = std::fs::remove_dir_all(&fitting_root);
    let a_fit = fitting.glyph('A');
    assert!(a_fit.rect.is_some());
    assert_ne!(
        a_fit.advance, 3.0,
        "fitting TrueType glyph takes precedence"
    );
    assert_eq!(fitting.advance('A', &style), a_fit.advance);
    assert_eq!(fitting.width("A"), a_fit.advance.ceil());
    let fit_bucket = a_fit.advance.ceil() as i32;
    {
        let mut cache = fitting.cache.borrow_mut();
        let Cache { sets, unihex, .. } = &mut *cache;
        let by_width = fitting.glyphs_by_width(&mut sets[0].1, unihex);
        assert!(
            by_width
                .get(&fit_bucket)
                .is_some_and(|chars| chars.contains(&'A')),
            "fitting TTF glyph is bucketed under TTF advance"
        );
        assert!(
            by_width.get(&3).is_none_or(|chars| !chars.contains(&'A')),
            "fitting TTF glyph is not bucketed under fallback bitmap advance"
        );
    }
    assert_eq!(
        fitting.random_glyph(&style, a_fit.advance).advance,
        a_fit.advance
    );
    assert_eq!(
        fitting.random_glyph(&style, 3.0).advance,
        6.0,
        "no width-3 glyph when fitting TTF shadows bitmap"
    );
    {
        let mut cache = metrics.cache.borrow_mut();
        let Cache { sets, unihex, .. } = &mut *cache;
        assert_eq!(
            metrics.provider_advance(&mut sets[0].1, unihex, 'A'),
            Some(a_fit.advance),
            "metrics-only font keeps TTF advance without an atlas region"
        );
    }

    // A fitting TrueType glyph with a fishy advance (> 32) preserves existing fishy-advance
    // behavior: drawn and bucketed under the TTF advance, while obfuscated `nonFishy`
    // falls through to the following bitmap provider.
    let fishy_def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":64},{"type":"bitmap","file":"minecraft:font/fallback.png","ascent":7,"chars":["A"]}]}"#;
    let fishy_root = root(
        "ttf-fishy-fitting",
        &[
            ("assets/minecraft/font/default.json", fishy_def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
        ],
    );
    let mut atlas_pixels = vec![0u8; 128 * 128 * 4];
    for y in 0..8usize {
        for x in 0..2usize {
            let i = (y * 128 + x) * 4;
            atlas_pixels[i..i + 4].copy_from_slice(&[255; 4]);
        }
    }
    let sprites = HashMap::from([(
        "font/fallback".to_string(),
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 8.0 / 128.0,
            v1: 8.0 / 128.0,
            w: 8,
            h: 8,
        },
    )]);
    let fishy = Font::load(
        &ResourceSource::directory(fishy_root.clone()),
        &sprites,
        &atlas_pixels,
        (128, 128),
        (8, 120),
        white(),
    );
    let _ = std::fs::remove_dir_all(&fishy_root);
    let a_fishy = fishy.glyph('A');
    assert!(a_fishy.rect.is_some(), "fitting fishy TTF glyph is drawn");
    assert!(
        fishy_advance(a_fishy.advance),
        "size-64 GeistMono advance ({}) is fishy",
        a_fishy.advance
    );
    assert_eq!(fishy.advance('A', &style), a_fishy.advance);
    assert_eq!(fishy.width("A"), a_fishy.advance.ceil());
    assert_eq!(
        fishy.random_glyph(&style, a_fishy.advance).advance,
        3.0,
        "fishy TTF stays indexed under TTF advance and draws nonFishy bitmap fallback"
    );
    assert_eq!(
        fishy.random_glyph(&style, 3.0).advance,
        6.0,
        "fitting fishy TTF is not bucketed under fallback advance 3"
    );
}

#[test]
fn true_type_canonical_table_matches_codepoint_ids_and_is_compact() {
    // Option<char> uses char's niche, so each canonical slot is 4 bytes.
    assert_eq!(
        std::mem::size_of::<Option<char>>(),
        std::mem::size_of::<char>()
    );
    let cases: [(&str, &[u8], &str, &[char]); 2] = [
        (
            "ttf-compact-mono",
            horizon_theme::fonts::GEIST_MONO,
            r#"{"type":"ttf","file":"minecraft:font.ttf","size":11}"#,
            &[],
        ),
        (
            "ttf-compact-bricolage",
            horizon_theme::fonts::BRICOLAGE_REGULAR,
            r#"{"type":"ttf","file":"minecraft:font.ttf","size":11,"skip":["A","z","\u00b5"]}"#,
            &['A', 'z', '\u{00b5}'],
        ),
    ];
    for (name, bytes, def_json, skip_chars) in cases {
        let root_dir = root(name, &[("assets/minecraft/font/font.ttf", bytes)]);
        let source = ResourceSource::directory(root_dir.clone());
        let def: serde_json::Value = serde_json::from_str(def_json).unwrap();
        let mut ttf = load_true_type(&source, &def).expect("TTF loads");
        let _ = std::fs::remove_dir_all(&root_dir);

        let raw = ab_glyph::FontVec::try_from_vec(bytes.to_vec()).unwrap();
        let expected: HashMap<char, ab_glyph::GlyphId> = raw
            .codepoint_ids()
            .filter(|(id, c)| id.0 != 0 && !skip_chars.contains(c))
            .map(|(id, c)| (c, id))
            .collect();

        // Exhaustive lookup equivalence across the entire BMP, every codepoint yielded
        // by `codepoint_ids()`, and sample supplementary codepoints.
        for c in (0..=0xFFFFu32)
            .filter_map(char::from_u32)
            .chain(['\u{1f600}', '\u{10ffff}'])
            .chain(raw.codepoint_ids().map(|(_, c)| c))
        {
            assert_eq!(
                ttf.glyph_id(c),
                expected.get(&c).copied(),
                "{name}: glyph_id mismatch for U+{:04X}",
                u32::from(c)
            );
        }

        assert!(ttf.overrides.is_empty(), "{name}: single-mapping cmap");
        assert!(ttf.canonical.len() <= raw.glyph_count());
        assert!(ttf.canonical.last().is_none_or(Option::is_some));

        let table_bytes = ttf.canonical.len() * std::mem::size_of::<Option<char>>()
            + ttf.overrides.len() * std::mem::size_of::<(char, ab_glyph::GlyphId)>();
        let map_min_bytes =
            expected.capacity() * (std::mem::size_of::<(char, ab_glyph::GlyphId)>() + 1);
        assert!(
            table_bytes < map_min_bytes,
            "{name}: expected compact table ({table_bytes} B) < HashMap ({map_min_bytes} B)"
        );

        // Verify the sorted override lookup path takes precedence over canonical entries
        // and handles characters unmapped by `font.glyph_id()`.
        if skip_chars.is_empty() {
            let orig_a = ttf.glyph_id('A').unwrap();
            let orig_b = ttf.glyph_id('B').unwrap();
            assert_ne!(orig_a, orig_b);
            ttf.overrides = vec![('A', orig_b), ('\u{e000}', orig_a)].into_boxed_slice();
            assert_eq!(ttf.glyph_id('A'), Some(orig_b));
            assert_eq!(ttf.glyph_id('\u{e000}'), Some(orig_a));
            assert_eq!(ttf.glyph_id('B'), Some(orig_b));
            assert_eq!(ttf.glyph_id('\u{e001}'), None);
        }
    }
}

#[test]
fn compact_unihex_table_lookup_overrides_and_memory_savings() {
    assert_eq!(std::mem::size_of::<HexEntry>(), 12);
    assert_eq!(std::mem::size_of::<HexGlyph>(), 72);

    use std::fmt::Write as _;

    // Include duplicate codepoints (same width and changed width to exercise
    // dead-row compaction) plus standard and >255 size_overrides.
    let mut hex = String::new();
    writeln!(hex, "0041:{}", "FF".repeat(16)).unwrap();
    // Overwrite 0041 with a 16-wide glyph so the old 8-wide row slice becomes dead.
    writeln!(hex, "0041:{}7FFE{}", "0000".repeat(4), "0000".repeat(11)).unwrap();
    // Overwrite 0042 in place with the same width.
    writeln!(hex, "0042:{}", "0F".repeat(16)).unwrap();
    writeln!(hex, "0042:{}", "F0".repeat(16)).unwrap();
    writeln!(hex, "0043:{}", "FF".repeat(16)).unwrap();
    writeln!(hex, "0044:{}", "FF".repeat(16)).unwrap();
    let archive = zip_with(&[("glyphs.hex", hex.as_bytes())]);
    let def: serde_json::Value = serde_json::from_slice(
        br#"{
            "type":"unihex",
            "hex_file":"pack:font/unihex.zip",
            "size_overrides":[
                {"from":"A","to":"B","left":2,"right":11},
                {"from":"C","to":"D","left":10,"right":300},
                {"from":"D","to":"D","left":1,"right":9}
            ]
        }"#,
    )
    .unwrap();
    let root_dir = root(
        "unihex-compact-overrides",
        &[("assets/pack/font/unihex.zip", &archive)],
    );
    let source = ResourceSource::directory(root_dir.clone());
    let HexLoad::Complete(table) = load_hex(&source, &UnihexKey::of(&def)) else {
        panic!("expected complete Unihex table load");
    };
    let _ = std::fs::remove_dir_all(&root_dir);

    // Dead 8-wide slice for 0041 was compacted out: 1x32 + 3x16 = 80 bytes.
    assert_eq!(table.entries.len(), 4);
    assert_eq!(table.rows.len(), 32 + 16 * 3);

    // 'A' kept the 16-wide rows and received the standard override (2, 11).
    let a = table.get(0x41).unwrap();
    assert_eq!(a.rows[4], 0x7FFE << 16);
    assert_eq!((a.left, a.right), (2, 11));
    assert_eq!(table.advance(0x41), Some(hex_advance(&a)));

    // 'B' kept the in-place overwritten 0xF0 rows and standard override (2, 11).
    let b = table.get(0x42).unwrap();
    assert_eq!(b.rows[0], 0xF0 << 24);
    assert_eq!((b.left, b.right), (2, 11));
    assert_eq!(table.advance(0x42), Some(hex_advance(&b)));

    // 'C' preserves exact u32 size_overrides > 255 without truncating to 255.
    let c = table.get(0x43).unwrap();
    assert_eq!((c.left, c.right), (10, 300));
    assert_eq!(table.advance(0x43), Some(146.0));
    assert_eq!(table.advance(0x43), Some(hex_advance(&c)));

    // 'D' had a >255 override replaced by a normal override (1, 9), clearing wide_bounds.
    let d = table.get(0x44).unwrap();
    assert_eq!((d.left, d.right), (1, 9));
    assert_eq!(table.advance(0x44), Some(hex_advance(&d)));
    assert_eq!(table.wide_bounds, [(0x43, 10, 300)]);

    // Memory savings on a 2,000-glyph mixed 8-wide / 16-wide table vs HashMap<u32, HexGlyph>.
    let mut bulk = String::new();
    let mut old_map: HashMap<u32, HexGlyph> = HashMap::new();
    for i in 0u32..2_000 {
        let cp = 0x0100 + i;
        let row_hex = if i.is_multiple_of(2) {
            "18".repeat(16)
        } else {
            "7FFE".repeat(16)
        };
        writeln!(bulk, "{cp:04X}:{row_hex}").unwrap();
        let mut rows = [0u32; 16];
        let (left, right) = if i.is_multiple_of(2) {
            rows.fill(0x18 << 24);
            (3, 4)
        } else {
            rows.fill(0x7FFE << 16);
            (1, 14)
        };
        old_map.insert(cp, HexGlyph { rows, left, right });
    }
    let compact = parse_hex_table(&[bulk.as_bytes()]);
    assert_eq!(compact.entries.len(), 2_000);
    assert!(compact.wide_bounds.is_empty());
    for (&cp, expected) in &old_map {
        let got = compact.get(cp).unwrap();
        assert_eq!(got.rows, expected.rows);
        assert_eq!((got.left, got.right), (expected.left, expected.right));
        assert_eq!(compact.advance(cp), Some(hex_advance(expected)));
    }
    let compact_bytes = compact.entries.capacity() * std::mem::size_of::<HexEntry>()
        + compact.rows.capacity()
        + compact.wide_bounds.capacity() * std::mem::size_of::<(u32, u32, u32)>();
    let old_bytes =
        old_map.capacity() * (std::mem::size_of::<u32>() + std::mem::size_of::<HexGlyph>());
    assert!(
        compact_bytes * 2 < old_bytes,
        "expected compact HexTable ({compact_bytes} B) to be <50% of HashMap<u32, HexGlyph> ({old_bytes} B)"
    );
}

#[test]
fn unihex_table_is_shared_across_filter_variants_and_font_sets() {
    let archive = zip_with(&[(
        "glyphs.hex",
        format!("0041:{}\n", "FF".repeat(16)).as_bytes(),
    )]);
    let default = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip","filter":{"jp":false}},
        {"type":"unihex","hex_file":"pack:font/unihex.zip","filter":{"jp":true}}
    ]}"#;
    let alt = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip"}
    ]}"#;
    let wide_a = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip","size_overrides":[{"from":"A","to":"A","left":0,"right":300}]}
    ]}"#;
    let wide_b = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip","size_overrides":[{"from":"A","to":"A","left":0,"right":400}]}
    ]}"#;
    let root_dir = root(
        "unihex-shared-variants",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/font/alt.json", alt),
            ("assets/pack/font/unihex.zip", &archive),
        ],
    );
    let mut font = load(&root_dir);

    // First filter variant lazily loads the archive from disk.
    assert_eq!(font.glyph('A').advance, 5.0);
    assert_eq!(font.cache.borrow().unihex.len(), 1);
    let _ = std::fs::remove_dir_all(&root_dir);

    // Switching filter variants and resetting the glyph cache reuses the same Arc<HexTable>
    // even though the archive directory has already been removed from disk.
    font.japanese = true;
    font.reset_cache();
    assert_eq!(font.glyph('A').advance, 5.0);

    // Another font set referencing the same archive shares the same Arc<HexTable>,
    // including through the provider_advance / random_glyph path.
    let alt_style = Style {
        font: Some("minecraft:alt".into()),
        ..Style::default()
    };
    assert_eq!(font.random_glyph(&alt_style, 5.0).advance, 5.0);
    assert_eq!(font.cache.borrow().unihex.len(), 1);

    // Distinct >255 size_overrides do not collide in the shared Unihex cache.
    // Reload from a live root for the two wide override sets.
    let root_dir = root(
        "unihex-wide-distinct",
        &[
            ("assets/minecraft/font/wide_a.json", wide_a),
            ("assets/minecraft/font/wide_b.json", wide_b),
            ("assets/pack/font/unihex.zip", &archive),
        ],
    );
    let wide_font = load(&root_dir);
    assert_eq!(wide_font.glyph_in("minecraft:wide_a", 'A').advance, 151.0);
    assert_eq!(wide_font.glyph_in("minecraft:wide_b", 'A').advance, 201.0);
    assert_eq!(wide_font.cache.borrow().unihex.len(), 2);
    let _ = std::fs::remove_dir_all(&root_dir);

    let cache = font.cache.borrow();
    let (_, default_set) = cache.sets.iter().find(|(k, _)| k == DEFAULT).unwrap();
    let (_, alt_set) = cache
        .sets
        .iter()
        .find(|(k, _)| k == "minecraft:alt")
        .unwrap();
    let Provider::Unihex(_, _, Some(jp_false)) = &default_set.providers[0] else {
        panic!("jp=false unihex provider should be loaded");
    };
    let Provider::Unihex(_, _, Some(jp_true)) = &default_set.providers[1] else {
        panic!("jp=true unihex provider should be loaded");
    };
    let Provider::Unihex(_, _, Some(alt_table)) = &alt_set.providers[0] else {
        panic!("alt unihex provider should be loaded");
    };
    assert!(Arc::ptr_eq(jp_false, jp_true));
    assert!(Arc::ptr_eq(jp_false, alt_table));
}

#[test]
fn failed_unihex_load_is_not_cached_as_empty() {
    let def = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip","filter":{"jp":false}},
        {"type":"unihex","hex_file":"pack:font/unihex.zip","filter":{"jp":true}}
    ]}"#;
    let valid_archive = zip_with(&[(
        "glyphs.hex",
        format!("0041:{}\n", "FF".repeat(16)).as_bytes(),
    )]);

    // 1. Missing archive: first filter variant fails without poisoning cache.unihex;
    // once the archive appears, the second filter variant retries and loads it.
    let root_missing = root(
        "unihex-retry-missing",
        &[("assets/minecraft/font/default.json", def)],
    );
    let mut font = load(&root_missing);
    assert_eq!(font.glyph('A').advance, 6.0);
    assert!(font.cache.borrow().unihex.is_empty());

    let archive_path = root_missing.join("assets/pack/font/unihex.zip");
    std::fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    std::fs::write(&archive_path, &valid_archive).unwrap();
    font.japanese = true;
    font.reset_cache();
    assert_eq!(font.glyph('A').advance, 5.0);
    assert_eq!(font.cache.borrow().unihex.len(), 1);
    let _ = std::fs::remove_dir_all(&root_missing);

    // 2. Corrupt archive bytes: also treated as a read failure, never cached as empty.
    let root_corrupt = root(
        "unihex-retry-corrupt",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/pack/font/unihex.zip", b"not a zip archive"),
        ],
    );
    let mut font = load(&root_corrupt);
    assert_eq!(font.glyph('A').advance, 6.0);
    assert!(font.cache.borrow().unihex.is_empty());

    std::fs::write(
        root_corrupt.join("assets/pack/font/unihex.zip"),
        &valid_archive,
    )
    .unwrap();
    font.japanese = true;
    font.reset_cache();
    assert_eq!(font.glyph('A').advance, 5.0);
    assert_eq!(font.cache.borrow().unihex.len(), 1);
    let _ = std::fs::remove_dir_all(&root_corrupt);

    // 3. Valid archive containing an empty .hex file IS cached and shared across variants.
    let empty_archive = zip_with(&[("empty.hex", b"")]);
    let root_empty = root(
        "unihex-valid-empty",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/pack/font/unihex.zip", &empty_archive),
        ],
    );
    let mut font = load(&root_empty);
    assert_eq!(font.glyph('A').advance, 6.0);
    assert_eq!(font.cache.borrow().unihex.len(), 1);
    let _ = std::fs::remove_dir_all(&root_empty);

    font.japanese = true;
    font.reset_cache();
    assert_eq!(font.glyph('A').advance, 6.0);
    let cache = font.cache.borrow();
    let (_, default_set) = cache.sets.iter().find(|(k, _)| k == DEFAULT).unwrap();
    let Provider::Unihex(_, _, Some(first)) = &default_set.providers[0] else {
        panic!("first empty unihex provider should be loaded");
    };
    let Provider::Unihex(_, _, Some(second)) = &default_set.providers[1] else {
        panic!("second empty unihex provider should be loaded");
    };
    assert!(first.entries.is_empty());
    assert!(Arc::ptr_eq(first, second));
}

#[test]
fn unreadable_unihex_member_keeps_readable_glyphs_without_caching_partial() {
    let a_hex = format!("0041:{}\n", "FF".repeat(16));
    let b_hex = format!("0042:{}\n", "7FFE".repeat(16));
    let c_hex = format!("0043:{}\n0044:{}\n", "18".repeat(16), "FF".repeat(16));
    let partial_archive = zip_with_corrupt_member(
        &[
            ("a_good.hex", a_hex.as_bytes()),
            ("b_corrupt.hex", b_hex.as_bytes()),
            ("c_good.hex", c_hex.as_bytes()),
        ],
        "b_corrupt.hex",
    );
    let repaired_archive = zip_with(&[
        ("a_good.hex", a_hex.as_bytes()),
        ("b_corrupt.hex", b_hex.as_bytes()),
        ("c_good.hex", c_hex.as_bytes()),
    ]);

    // Verify the fixture opens cleanly as a ZIP while only the middle .hex member fails.
    let probe = Zip::from_bytes(partial_archive.clone()).unwrap();
    assert!(probe.read("a_good.hex").is_ok());
    assert!(probe.read("b_corrupt.hex").is_err());
    assert!(probe.read("c_good.hex").is_ok());

    let def = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip","size_overrides":[{"from":"D","to":"D","left":0,"right":11}],"filter":{"jp":false}},
        {"type":"unihex","hex_file":"pack:font/unihex.zip","size_overrides":[{"from":"D","to":"D","left":0,"right":11}],"filter":{"jp":true}}
    ]}"#;
    let alt = br#"{"providers":[
        {"type":"unihex","hex_file":"pack:font/unihex.zip","size_overrides":[{"from":"D","to":"D","left":0,"right":11}]}
    ]}"#;
    let root_dir = root(
        "unihex-partial-member",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/font/alt.json", alt),
            ("assets/pack/font/unihex.zip", &partial_archive),
        ],
    );

    let source = ResourceSource::directory(root_dir.clone());
    let parsed_def: serde_json::Value = serde_json::from_slice(def).unwrap();
    let key = UnihexKey::of(&parsed_def["providers"][0]);
    match load_hex(&source, &key) {
        HexLoad::Partial(table) => {
            assert_eq!(table.advance(0x41), Some(5.0));
            assert_eq!(table.advance(0x42), None);
            assert_eq!(table.advance(0x43), Some(2.0));
            assert_eq!(table.advance(0x44), Some(7.0));
        }
        HexLoad::Complete(_) => {
            panic!("archive with an unreadable .hex member must return HexLoad::Partial");
        }
        HexLoad::Failed => {
            panic!("archive with readable .hex members must not drop the whole table");
        }
    }

    // First provider keeps glyphs from both readable members (before and after the
    // corrupt one) with size_overrides applied, skips the corrupt member's glyph,
    // and leaves the shared cache unpopulated.
    let mut font = load(&root_dir);
    let a = font.glyph('A');
    assert_eq!(a.advance, 5.0);
    assert_eq!(a.offset, 0.5);
    assert!(a.rect.is_some());
    let c = font.glyph('C');
    assert_eq!(c.advance, 2.0);
    assert_eq!(c.offset, 0.5);
    assert!(c.rect.is_some());
    let d = font.glyph('D');
    assert_eq!(d.advance, 7.0);
    assert_eq!(d.offset, 0.5);
    assert!(d.rect.is_some());
    let b_missing = font.glyph('B');
    assert_eq!(b_missing.advance, 6.0);
    assert!(b_missing.rect.is_none());
    assert!(font.cache.borrow().unihex.is_empty());

    // Repair the archive: a later provider retries, loads all glyphs (including 'B'),
    // and populates the shared cache for subsequent providers.
    std::fs::write(
        root_dir.join("assets/pack/font/unihex.zip"),
        &repaired_archive,
    )
    .unwrap();
    font.japanese = true;
    font.reset_cache();
    assert_eq!(font.glyph('A').advance, 5.0);
    let b = font.glyph('B');
    assert_eq!(b.advance, 8.0);
    assert_eq!(b.offset, 0.5);
    assert!(b.rect.is_some());
    assert_eq!(font.glyph('C').advance, 2.0);
    assert_eq!(font.glyph('D').advance, 7.0);
    assert_eq!(font.cache.borrow().unihex.len(), 1);
    let _ = std::fs::remove_dir_all(&root_dir);

    // Another font set reuses the cached complete table without touching disk.
    assert_eq!(font.glyph_in("minecraft:alt", 'B').advance, 8.0);
    let cache = font.cache.borrow();
    let (_, default_set) = cache.sets.iter().find(|(k, _)| k == DEFAULT).unwrap();
    let (_, alt_set) = cache
        .sets
        .iter()
        .find(|(k, _)| k == "minecraft:alt")
        .unwrap();
    let Provider::Unihex(_, _, Some(partial_table)) = &default_set.providers[0] else {
        panic!("first unihex provider should hold its partial table");
    };
    let Provider::Unihex(_, _, Some(full_table)) = &default_set.providers[1] else {
        panic!("second unihex provider should hold the complete table");
    };
    let Provider::Unihex(_, _, Some(alt_table)) = &alt_set.providers[0] else {
        panic!("alt unihex provider should share the complete table");
    };
    assert!(!Arc::ptr_eq(partial_table, full_table));
    assert!(Arc::ptr_eq(full_table, alt_table));
}
