//! A download from a mock of Mojang's servers serving small fake files (never Mojang's).
use super::*;
use crate::sources::{resolve, sha1_hex, Resolved};
use std::collections::HashMap;

const CLIENT: &[u8] = b"fake client jar";
const COMMON: &[u8] = b"a library every system uses";
const LINUX: &[u8] = b"a linux native";
const SOUND: &[u8] = b"a fake ogg";
const LANG: &[u8] = b"{}";

/// The fake version file, asset index and files, by URL.
fn mojang() -> (Pins, HashMap<String, Vec<u8>>) {
    let artifact = |name: &str, data: &[u8]| {
        serde_json::json!({
            "path": format!("org/fake/{name}/1/{name}-1.jar"),
            "sha1": sha1_hex(data),
            "url": format!("https://libraries.minecraft.net/org/fake/{name}/1/{name}-1.jar"),
        })
    };
    let index = serde_json::json!({"objects": {
        "minecraft/sounds/a.ogg": {"hash": sha1_hex(SOUND), "size": SOUND.len()},
        "minecraft/sounds/same.ogg": {"hash": sha1_hex(SOUND), "size": SOUND.len()},
        "minecraft/lang/fr_fr.json": {"hash": sha1_hex(LANG), "size": LANG.len()},
    }})
    .to_string();
    let manifest = serde_json::json!({
        "id": "26.3",
        "assetIndex": {"id": "34", "sha1": sha1_hex(index.as_bytes()),
                       "url": "https://piston-meta.mojang.com/v1/packages/i/34.json"},
        "downloads": {"client": {"sha1": sha1_hex(CLIENT), "url": "https://piston-data.mojang.com/client.jar"}},
        "libraries": [
            {"name": "org.fake:common:1", "downloads": {"artifact": artifact("common", COMMON)}},
            {"name": "org.fake:linux:1", "downloads": {"artifact": artifact("linux", LINUX)},
             "rules": [{"action": "allow", "os": {"name": "linux"}}]},
            {"name": "org.fake:not-osx:1", "downloads": {"artifact": artifact("not-osx", COMMON)},
             "rules": [{"action": "allow"}, {"action": "disallow", "os": {"name": "osx"}}]},
            {"name": "org.fake:windows:1", "downloads": {"artifact": artifact("windows", b"w")},
             "rules": [{"action": "allow", "os": {"name": "windows"}}]},
        ],
    })
    .to_string();
    let pins = Pins::read(&serde_json::json!({"minecraft": {
        "version": "26.3",
        "client_sha1": sha1_hex(CLIENT),
        "manifest_sha1": sha1_hex(manifest.as_bytes()),
    }}))
    .unwrap();
    let object = |data: &[u8]| {
        let hash = sha1_hex(data);
        format!("{OBJECTS}/{}/{hash}", &hash[..2])
    };
    let lib =
        |name: &str| format!("https://libraries.minecraft.net/org/fake/{name}/1/{name}-1.jar");
    let files = HashMap::from([
        (
            format!(
                "https://piston-meta.mojang.com/v1/packages/{}/26.3.json",
                sha1_hex(manifest.as_bytes())
            ),
            manifest.into_bytes(),
        ),
        (
            "https://piston-data.mojang.com/client.jar".into(),
            CLIENT.to_vec(),
        ),
        (lib("common"), COMMON.to_vec()),
        (lib("linux"), LINUX.to_vec()),
        (lib("not-osx"), COMMON.to_vec()),
        (lib("windows"), b"w".to_vec()),
        (
            "https://piston-meta.mojang.com/v1/packages/i/34.json".into(),
            index.into_bytes(),
        ),
        (object(SOUND), SOUND.to_vec()),
        (object(LANG), LANG.to_vec()),
    ]);
    (pins, files)
}

struct Server {
    files: HashMap<String, Vec<u8>>,
    asked: Mutex<Vec<String>>,
}

impl Server {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        self.asked.lock().unwrap().push(url.to_string());
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| format!("{url} answered 404"))
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-mojang-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn an_owner_gets_the_official_layout_with_this_systems_libraries() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("layout");
    let logged = Mutex::new(Vec::new());
    download(&pins, &dir, "linux", 3, &|url| server.fetch(url), &|line| {
        logged.lock().unwrap().push(line)
    })
    .unwrap();
    assert_eq!(
        fs::read(dir.join("versions/26.3/26.3.jar")).unwrap(),
        CLIENT
    );
    let libraries = dir.join("libraries/org/fake");
    assert_eq!(
        fs::read(libraries.join("common/1/common-1.jar")).unwrap(),
        COMMON
    );
    assert_eq!(
        fs::read(libraries.join("linux/1/linux-1.jar")).unwrap(),
        LINUX
    );
    assert!(libraries.join("not-osx/1/not-osx-1.jar").is_file());
    assert!(!libraries.join("windows").exists());
    let hash = sha1_hex(SOUND);
    assert_eq!(
        fs::read(dir.join("assets/objects").join(&hash[..2]).join(&hash)).unwrap(),
        SOUND
    );
    assert!(dir.join("assets/indexes/34.json").is_file());
    let asked = server.asked();
    // The sound two names share is fetched once; nothing is fetched twice.
    assert_eq!(asked.len(), 8, "{asked:#?}");
    assert!(!asked.iter().any(|u| u.contains("windows")));
    assert_eq!(
        logged.lock().unwrap()[1],
        "Downloading Minecraft 26.3 from Mojang: the game and 3 libraries"
    );

    // Found like any launcher's folder, as it is; a second download fetches nothing.
    let found = resolve(
        &dir,
        &pins,
        &[],
        &scratch("unused"),
        &|url| server.fetch(url),
        &|_| {},
    );
    assert_eq!(found, Ok(Resolved::Folder(dir.clone())));
    download(&pins, &dir, "linux", 3, &|url| server.fetch(url), &|_| {}).unwrap();
    assert_eq!(server.asked(), Vec::<String>::new());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn macos_skips_what_its_rules_disallow() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("osx");
    download(&pins, &dir, "osx", 2, &|url| server.fetch(url), &|_| {}).unwrap();
    let libraries = dir.join("libraries/org/fake");
    assert!(libraries.join("common/1/common-1.jar").is_file());
    assert!(!libraries.join("linux").exists());
    assert!(!libraries.join("not-osx").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_is_not_the_pinned_one_stops_the_download() {
    let (pins, mut files) = mojang();
    files.insert(
        "https://piston-data.mojang.com/client.jar".into(),
        b"something else".to_vec(),
    );
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("bad-jar");
    let error = download(&pins, &dir, "linux", 2, &|url| server.fetch(url), &|_| {}).unwrap_err();
    assert!(
        error.starts_with("https://piston-data.mojang.com/client.jar is not the one Horizon pins"),
        "{error}"
    );
    assert!(!dir.join("versions/26.3/26.3.jar").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_sounds_are_a_line_in_the_log_not_an_error() {
    let (pins, mut files) = mojang();
    let hash = sha1_hex(SOUND);
    files.remove(&format!("{OBJECTS}/{}/{hash}", &hash[..2]));
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("no-sound");
    let logged = Mutex::new(Vec::new());
    download(&pins, &dir, "linux", 1, &|url| server.fetch(url), &|line| {
        logged.lock().unwrap().push(line)
    })
    .unwrap();
    assert!(dir.join("versions/26.3/26.3.jar").is_file());
    let last = logged.lock().unwrap().last().cloned().unwrap();
    assert!(
        last.starts_with("The sounds could not all be downloaded:")
            && last.ends_with("answered 404"),
        "{last}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn version_file_paths_stay_in_the_folder() {
    assert!(relative("org/fake/a.jar").is_ok());
    for bad in ["../a.jar", "/etc/a", "a/../../b", "a\\b"] {
        assert!(relative(bad).is_err(), "{bad}");
    }
}

#[test]
fn links_stay_inside_the_runtime() {
    let at = |p: &str| PathBuf::from(p);
    assert!(link_stays_inside(
        &at("legal/java.compiler/LICENSE"),
        "../java.base/LICENSE"
    ));
    assert!(link_stays_inside(&at("lib/a"), "b"));
    assert!(!link_stays_inside(&at("lib/a"), "../../b"));
    assert!(!link_stays_inside(&at("a"), "/etc/passwd"));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn java_runtime_lays_out_mojangs_runtime_with_its_executables_and_links() {
    const JAVAC: &[u8] = b"#!/bin/sh\necho javac 25.0.1\n";
    const JAVA: &[u8] = b"#!/bin/sh\necho java 25.0.1\n";
    const LICENSE: &[u8] = b"a fake license";
    let manifest = serde_json::json!({"files": {
        "bin": {"type": "directory"},
        "bin/java": {"type": "file", "executable": true, "downloads": {"raw":
            {"sha1": sha1_hex(JAVA), "size": JAVA.len(), "url": "https://piston-data.mojang.com/java"}}},
        "bin/javac": {"type": "file", "executable": true, "downloads": {"raw":
            {"sha1": sha1_hex(JAVAC), "size": JAVAC.len(), "url": "https://piston-data.mojang.com/javac"}}},
        "legal/java.base/LICENSE": {"type": "file", "executable": false, "downloads": {"raw":
            {"sha1": sha1_hex(LICENSE), "size": LICENSE.len(), "url": "https://piston-data.mojang.com/license"}}},
        "legal/java.compiler/LICENSE": {"type": "link", "target": "../java.base/LICENSE"},
    }})
    .to_string();
    let index = serde_json::json!({"linux": {"java-runtime-epsilon": [{
        "manifest": {"sha1": sha1_hex(manifest.as_bytes()), "url": "https://piston-meta.mojang.com/m.json"},
        "version": {"name": "25.0.1"},
    }]}})
    .to_string();
    let server = Server {
        files: HashMap::from([
            (JAVA_RUNTIMES.to_string(), index.into_bytes()),
            (
                "https://piston-meta.mojang.com/m.json".into(),
                manifest.into_bytes(),
            ),
            ("https://piston-data.mojang.com/java".into(), JAVA.to_vec()),
            (
                "https://piston-data.mojang.com/javac".into(),
                JAVAC.to_vec(),
            ),
            (
                "https://piston-data.mojang.com/license".into(),
                LICENSE.to_vec(),
            ),
        ]),
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("java");
    let home = java_runtime(
        "java-runtime-epsilon",
        &dir,
        2,
        &|url| server.fetch(url),
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        home,
        dir.join("java-runtime-epsilon/linux/java-runtime-epsilon")
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(home.join("bin/javac"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
    assert_eq!(
        fs::read(home.join("legal/java.compiler/LICENSE")).unwrap(),
        LICENSE
    );
    // horizon-prepare finds it where the official launcher would have put it.
    assert_eq!(horizon_prepare::java::minecraft_runtimes(&dir), [home]);
    // A second call keeps the files: only the index and manifest are asked again.
    server.asked();
    java_runtime(
        "java-runtime-epsilon",
        &dir,
        2,
        &|url| server.fetch(url),
        &|_| {},
    )
    .unwrap();
    assert_eq!(server.asked().len(), 2);
    assert!(java_runtime(
        "java-runtime-zeta",
        &dir,
        2,
        &|url| server.fetch(url),
        &|_| {}
    )
    .is_err());
    fs::remove_dir_all(&dir).unwrap();
}
