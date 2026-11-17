//! A `.hshader`'s limits hold while it is read, before anything is kept: the registry reads every
//! submitted package, so a small zip must not make it hold more than the pack's 8 MiB.
use horizon_shader_api::{MANIFEST, Pack, package::MAX_TOTAL};
use std::{collections::BTreeMap, io::Write};

/// A zip of `count` files of `size` zeros each (deflate makes it tiny).
fn zeros(path: &std::path::Path, count: usize, size: usize) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let block = vec![0u8; size];
    for i in 0..count {
        zip.start_file(format!("f{i}.png"), options).unwrap();
        zip.write_all(&block).unwrap();
    }
    zip.finish().unwrap();
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hshader-limits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "daily"]
fn a_package_over_the_total_is_refused_while_it_is_read() {
    let dir = scratch("total");
    let path = dir.join("big.hshader");
    // 64 files of 1 MiB: 64 MiB inflated, a few hundred KiB zipped; reading stops past 8 MiB.
    zeros(&path, 64, 1 << 20);
    assert!(std::fs::metadata(&path).unwrap().len() < 1 << 20);
    let refused = Pack::read(&path).unwrap_err();
    assert_eq!(refused, format!("the pack is over {} MiB", MAX_TOTAL >> 20));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn one_file_over_its_cap_is_named() {
    let dir = scratch("file");
    let path = dir.join("one.hshader");
    zeros(&path, 1, (1 << 20) + 1);
    assert_eq!(Pack::read(&path).unwrap_err(), "f0.png is over 1024 KiB");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A pack made for a line this build doesn't serve is read without its manifest being taken
/// (`Pack::declared`), under the same caps.
#[test]
fn a_pack_for_another_line_is_read_under_the_same_caps() {
    let dir = scratch("declared");
    let path = dir.join("one.hshader");
    zeros(&path, 1, (1 << 20) + 1);
    assert_eq!(
        Pack::declared(&path).unwrap_err(),
        "f0.png is over 1024 KiB"
    );
    zeros(&path, 9, 1 << 20);
    assert_eq!(
        Pack::declared(&path).unwrap_err(),
        format!("the pack is over {} MiB", MAX_TOTAL >> 20)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_over_the_total_is_refused_too() {
    let dir = scratch("folder");
    for i in 0..9 {
        std::fs::write(dir.join(format!("f{i}.png")), vec![0u8; 1 << 20]).unwrap();
    }
    assert_eq!(
        Pack::read(&dir).unwrap_err(),
        format!("the pack is over {} MiB", MAX_TOTAL >> 20)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A valid pack of exactly `count` files: the manifest, `count - 2` shared WGSL files and
/// `sky.wgsl`.
fn pack_files(count: usize) -> BTreeMap<String, Vec<u8>> {
    let shared: Vec<String> = (0..count - 2).map(|i| format!("\"f{i}.wgsl\"")).collect();
    let manifest = format!(
        "schema = 1\nid = \"limits\"\nname = \"Limits\"\nversion = \"1.0.0\"\napi = \"0.2\"\n\
         description = \"Limits\"\nauthors = [\"Horizon\"]\nshared = [{}]\n[sky]\nfile = \"sky.wgsl\"\n",
        shared.join(", ")
    );
    let mut files = BTreeMap::new();
    files.insert(MANIFEST.to_owned(), manifest.into_bytes());
    for i in 0..count - 2 {
        files.insert(format!("f{i}.wgsl"), Vec::new());
    }
    files.insert("sky.wgsl".to_owned(), Vec::new());
    files
}

fn write_pack(dir: &std::path::Path, files: &BTreeMap<String, Vec<u8>>) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

/// The 512-file bound holds for a map, a folder and the archive that map exports, at the
/// boundary and one past it.
#[test]
fn the_file_count_bound_holds_for_maps_folders_and_archives() {
    let dir = scratch("entries");
    let files = pack_files(512);
    let pack = Pack::from_files(files.clone()).expect("512 files are accepted");
    let archive = dir.join("at-the-bound.hshader");
    std::fs::write(&archive, pack.package().unwrap()).unwrap();
    assert_eq!(Pack::read(&archive).unwrap().files.len(), 512);

    let folder = dir.join("at-the-bound");
    write_pack(&folder, &files);
    assert_eq!(Pack::read(&folder).unwrap().files.len(), 512);

    let too_many = pack_files(513);
    assert_eq!(
        Pack::from_files(too_many.clone()).unwrap_err(),
        "over 512 files in the package"
    );
    let folder = dir.join("over-the-bound");
    write_pack(&folder, &too_many);
    assert_eq!(
        Pack::read(&folder).unwrap_err(),
        "over 512 files in the package"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
