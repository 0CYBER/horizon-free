use super::*;
use std::cell::Cell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use zip::write::SimpleFileOptions;

fn pack(entries: u64, with_manifest: bool) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    if with_manifest {
        writer.start_file("manifest.json", options).unwrap();
        writer.write_all(br#"{"name":"Example Pack"}"#).unwrap();
    }
    for i in 0..entries {
        writer
            .start_file(
                format!("overrides/config/folder-{}/entry-{i}.cfg", i / 100),
                options,
            )
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

// Both shape and prefix tests read the same large archive without modifying it.
fn modpack(entries: u64) -> std::borrow::Cow<'static, [u8]> {
    if entries == 65_536 {
        static ZIP64: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        std::borrow::Cow::Borrowed(ZIP64.get_or_init(|| pack(65_536, true)))
    } else {
        std::borrow::Cow::Owned(pack(entries, true))
    }
}

fn oversized_entry_count() -> [u8; END64_LEN + LOCATOR64_LEN + END_LEN] {
    // A ZIP64 footer can declare a large entry table in a tiny file.
    let mut archive = [0; END64_LEN + LOCATOR64_LEN + END_LEN];
    archive[..4].copy_from_slice(&END64);
    archive[4..12].copy_from_slice(&44u64.to_le_bytes());
    let declared = (MAX_ENTRIES + 1).to_le_bytes();
    archive[24..32].copy_from_slice(&declared);
    archive[32..40].copy_from_slice(&declared);
    let locator = &mut archive[END64_LEN..END64_LEN + LOCATOR64_LEN];
    locator[..4].copy_from_slice(&LOCATOR64);
    locator[16..20].copy_from_slice(&1u32.to_le_bytes());
    let end = &mut archive[END64_LEN + LOCATOR64_LEN..];
    end[..4].copy_from_slice(&END);
    end[8..12].fill(0xff);
    archive
}

#[test]
fn refuses_a_declared_entry_count_before_reading_a_directory() {
    let error = open(Cursor::new(oversized_entry_count()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("ZIP entry count"), "{error}");
}

#[test]
fn refuses_a_declared_oversized_central_directory() {
    let mut archive = pack(0, true);
    let end = archive.len() - END_LEN;
    archive[end + 12..end + 16].copy_from_slice(&((MAX_DIRECTORY_BYTES + 1) as u32).to_le_bytes());

    let error = open(Cursor::new(archive)).unwrap_err().to_string();
    assert!(error.contains("ZIP central directory size"), "{error}");
}

#[test]
fn zero_length_entries_consume_the_entry_budget() {
    let archive = pack(MAX_ENTRIES + 1, false);
    let error = open(Cursor::new(archive)).unwrap_err().to_string();
    assert!(error.contains("ZIP entry count"), "{error}");
}

#[test]
fn accepts_large_zip32_and_zip64_modpack_shapes() {
    for entries in [20_000, 65_536] {
        let archive = modpack(entries);
        let mut zip = open(Cursor::new(archive.as_ref())).unwrap();
        assert_eq!(zip.len(), entries as usize + 1);
        let mut text = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, r#"{"name":"Example Pack"}"#);
    }
}

#[test]
fn accepts_redundant_zip64_metadata_with_matching_classic_fields() {
    let mut archive = pack(0, true);
    let end_at = archive.len() - END_LEN;
    let end = archive.split_off(end_at);
    let mut footer = [0; END64_LEN + LOCATOR64_LEN];
    footer[..4].copy_from_slice(&END64);
    footer[4..12].copy_from_slice(&44u64.to_le_bytes());
    footer[12..16].copy_from_slice(&[45, 0, 45, 0]);
    let count = u64::from(u16_at(&end, 10)).to_le_bytes();
    footer[24..32].copy_from_slice(&count);
    footer[32..40].copy_from_slice(&count);
    footer[40..48].copy_from_slice(&u64::from(u32_at(&end, 12)).to_le_bytes());
    footer[48..56].copy_from_slice(&u64::from(u32_at(&end, 16)).to_le_bytes());
    let locator = &mut footer[END64_LEN..];
    locator[..4].copy_from_slice(&LOCATOR64);
    locator[8..16].copy_from_slice(&(end_at as u64).to_le_bytes());
    locator[16..20].copy_from_slice(&1u32.to_le_bytes());
    archive.extend(footer);
    archive.extend(end);

    let mut zip = open(Cursor::new(archive)).unwrap();
    assert_eq!(zip.len(), 1);
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, r#"{"name":"Example Pack"}"#);
}

#[test]
fn refuses_records_past_the_declared_count_or_directory() {
    let mut archive = pack(1, true);
    let end = archive.len() - END_LEN;
    archive[end + 8..end + 12].copy_from_slice(&[1, 0, 1, 0]);
    let error = open(Cursor::new(archive)).unwrap_err().to_string();
    assert!(error.contains("ZIP central directory size"), "{error}");
}

#[test]
fn keeps_offsets_for_prefixed_archives() {
    for entries in [0, 65_536] {
        let mut prefixed = b"prefix bytes".to_vec();
        prefixed.extend_from_slice(&modpack(entries));
        let mut zip = open(Cursor::new(prefixed)).unwrap();
        assert_eq!(zip.offset(), 12);
        assert!(zip.by_name("manifest.json").is_ok());
    }
}

#[test]
fn opens_the_directory_that_was_checked() {
    struct ChangingArchive<'a> {
        contents: Cursor<Vec<u8>>,
        changed: &'a Cell<bool>,
    }

    impl Read for ChangingArchive<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = self.contents.read(buf)?;
            let bytes = self.contents.get_mut();
            if !self.changed.get() {
                // Overwrite the original footer just after preflight copies it.
                let end = bytes.len() - END_LEN;
                bytes[end + 12..end + 16]
                    .copy_from_slice(&((MAX_DIRECTORY_BYTES + 1) as u32).to_le_bytes());
                self.changed.set(true);
            }
            Ok(count)
        }
    }

    impl Seek for ChangingArchive<'_> {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.contents.seek(from)
        }
    }

    let changed = Cell::new(false);
    let mut zip = open(ChangingArchive {
        contents: Cursor::new(pack(0, true)),
        changed: &changed,
    })
    .unwrap();
    assert_eq!(zip.len(), 1);
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, r#"{"name":"Example Pack"}"#);
    assert!(changed.get());
}

#[test]
fn skips_an_end_record_in_a_zip_comment_when_its_disk_fields_are_invalid() {
    let mut archive = pack(0, true);
    let end = archive.len() - END_LEN;
    archive[end + 20..end + 22].copy_from_slice(&(END_LEN as u16).to_le_bytes());
    let mut comment = [0; END_LEN];
    comment[..4].copy_from_slice(&END);
    comment[4] = 1;
    archive.extend(comment);

    let zip = open(Cursor::new(archive)).unwrap();
    assert_eq!(zip.comment(), comment);
    assert_eq!(zip.len(), 1);
}
