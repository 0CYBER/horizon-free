use super::*;
use crate::browser::{download, FileRef, Query, GAME};
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Mutex;

/// A source in memory: projects, their versions, and the bytes each file URL serves.
#[derive(Default)]
struct Fake {
    projects: HashMap<String, Project>,
    versions: Mutex<HashMap<String, Vec<Version>>>,
    bytes: Mutex<HashMap<String, Vec<u8>>>,
}

impl Source for Fake {
    fn id(&self) -> SourceId {
        SourceId::Modrinth
    }
    fn search(&self, _: &Query) -> Result<Vec<Project>, String> {
        Ok(self.projects.values().cloned().collect())
    }
    fn project(&self, id: &str) -> Result<Project, String> {
        self.projects
            .get(id)
            .cloned()
            .ok_or_else(|| "not a pack".into())
    }
    fn versions(&self, p: &Project) -> Result<Vec<Version>, String> {
        Ok(self
            .versions
            .lock()
            .unwrap()
            .get(&p.id)
            .cloned()
            .unwrap_or_default())
    }
    fn fetch(
        &self,
        file: &FileRef,
        folder: &Path,
        progress: &dyn Fn(f32),
    ) -> Result<PathBuf, String> {
        let body = self
            .bytes
            .lock()
            .unwrap()
            .get(&file.url)
            .cloned()
            .unwrap_or_default();
        download::save(file, &body[..], folder, progress)
    }
}

fn project(id: &str, kind: Kind) -> Project {
    Project {
        source: SourceId::Modrinth,
        id: id.into(),
        slug: format!("{id}-slug"),
        title: format!("Pack {id}"),
        summary: String::new(),
        author: String::new(),
        downloads: 0,
        page: format!("https://modrinth.com/resourcepack/{id}-slug"),
        icon: None,
        license: "MIT".into(),
        kind,
        game_versions: vec![GAME.into()],
        loaders: vec![if kind == Kind::ShaderPack {
            "iris"
        } else {
            "minecraft"
        }
        .into()],
    }
}

impl Fake {
    fn add(&mut self, p: Project) {
        self.projects.insert(p.id.clone(), p);
    }
    /// A new newest version of `id`, its file serving `body`; `tamper` serves other bytes.
    fn release(&self, id: &str, number: &str, needs: &[&str], body: &[u8], tamper: bool) {
        let url = format!("https://cdn.modrinth.com/data/{id}/versions/{number}/{id}-{number}.zip");
        let file = FileRef {
            url: url.clone(),
            name: format!("{id}-{number}.zip"),
            size: body.len() as u64,
            sha512: Some(
                sha2::Sha512::digest(body)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            ),
            sha1: None,
            primary: true,
        };
        let version = Version {
            id: format!("{id}@{number}"),
            number: number.into(),
            published: number.into(),
            game_versions: vec![GAME.into()],
            loaders: self.projects[id].loaders.clone(),
            files: vec![file],
            required: needs.iter().map(|s| s.to_string()).collect(),
            blocked: None,
        };
        let mut served = body.to_vec();
        if tamper {
            served[0] ^= 1;
        }
        self.bytes.lock().unwrap().insert(url, served);
        let mut versions = self.versions.lock().unwrap();
        versions.entry(id.into()).or_default().insert(0, version);
    }
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-installed-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const NONE: &dyn Fn(&str, f32) = &|_, _| {};

#[test]
fn installs_with_dependencies_then_updates_and_removes() {
    let home = home("cycle");
    let mut fake = Fake::default();
    fake.add(project("base", Kind::ResourcePack));
    fake.add(project("addon", Kind::ResourcePack));
    fake.release("base", "1", &[], b"base pack", false);
    fake.release("addon", "1", &["base"], b"addon pack", false);
    let addon = fake.project("addon").unwrap();

    let done = install(&fake, &addon, false, &home, NONE).unwrap();
    let files: Vec<&str> = done.iter().map(|e| e.file.as_str()).collect();
    assert_eq!(
        files,
        ["resourcepacks/base-1.zip", "resourcepacks/addon-1.zip"]
    );
    assert!(done[0].dependency && !done[1].dependency);
    assert!(done[1].checksum.starts_with("SHA-512:"));
    assert_eq!(
        fs::read(home.join("resourcepacks/addon-1.zip")).unwrap(),
        b"addon pack"
    );
    assert_eq!(Installed::load(&home).entries.len(), 2);

    // Installing again with nothing new downloads nothing.
    fs::remove_file(home.join("resourcepacks/base-1.zip")).unwrap();
    install(&fake, &addon, false, &home, NONE).unwrap();
    assert!(!home.join("resourcepacks/base-1.zip").exists());

    // An update replaces the old file.
    fake.release("addon", "2", &["base"], b"addon pack v2", false);
    let entry = Installed::load(&home).lookup("addon").cloned().unwrap();
    assert_eq!(update_for(&fake, &entry).unwrap().unwrap().number, "2");
    install(&fake, &addon, false, &home, NONE).unwrap();
    assert!(!home.join("resourcepacks/addon-1.zip").exists());
    assert_eq!(
        fs::read(home.join("resourcepacks/addon-2.zip")).unwrap(),
        b"addon pack v2"
    );
    let entry = Installed::load(&home)
        .lookup("addon-slug")
        .cloned()
        .unwrap();
    assert_eq!(entry.number, "2");
    assert!(update_for(&fake, &entry).unwrap().is_none());

    let removed = remove(&home, "addon").unwrap();
    assert_eq!(removed.project, "addon");
    assert!(!home.join("resourcepacks/addon-2.zip").exists());
    assert!(remove(&home, "addon").is_err());
    assert_eq!(Installed::load(&home).entries.len(), 1);
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn nothing_is_installed_when_a_check_fails() {
    let home = home("refuse");
    let mut fake = Fake::default();
    fake.add(project("good", Kind::ResourcePack));
    fake.add(project("bad", Kind::ResourcePack));
    fake.add(project("needs-mod", Kind::ResourcePack));
    fake.release("good", "1", &[], b"good", false);
    fake.release("bad", "1", &[], b"tampered", true);
    fake.release("needs-mod", "1", &["sodium"], b"x", false);

    let e = install(&fake, &fake.project("bad").unwrap(), false, &home, NONE).unwrap_err();
    assert!(e.contains("checksum doesn't match"), "{e}");
    let e = install(
        &fake,
        &fake.project("needs-mod").unwrap(),
        false,
        &home,
        NONE,
    )
    .unwrap_err();
    assert!(e.contains("needs sodium"), "{e}");
    assert!(
        !home.join("resourcepacks").exists()
            || fs::read_dir(home.join("resourcepacks")).unwrap().count() == 0
    );
    assert!(Installed::load(&home).entries.is_empty());

    // The player's own file of the same name is never replaced.
    fs::create_dir_all(home.join("resourcepacks")).unwrap();
    fs::write(home.join("resourcepacks/good-1.zip"), b"mine").unwrap();
    let e = install(&fake, &fake.project("good").unwrap(), false, &home, NONE).unwrap_err();
    assert!(e.contains("wasn't installed by the browser"), "{e}");
    assert_eq!(
        fs::read(home.join("resourcepacks/good-1.zip")).unwrap(),
        b"mine"
    );
    assert!(remove(&home, "good").is_err());
    assert!(home.join("resourcepacks/good-1.zip").exists());

    // What doesn't run is refused before anything is asked for.
    let mut canvas = project("canvas", Kind::ShaderPack);
    canvas.loaders = vec!["canvas".into()];
    fake.add(canvas.clone());
    assert!(install(&fake, &canvas, true, &home, NONE)
        .unwrap_err()
        .contains("doesn't run"));
    let pack = project("pack", Kind::Modpack);
    assert!(plan(&fake, &pack, false)
        .unwrap_err()
        .contains("Modpacks page"));
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn shader_packs_go_to_their_folder_and_modpacks_download_checked() {
    let home = home("kinds");
    let mut fake = Fake::default();
    fake.add(project("bsl", Kind::ShaderPack));
    fake.release("bsl", "10", &[], b"shaders", false);
    install(&fake, &fake.project("bsl").unwrap(), false, &home, NONE).unwrap();
    assert!(home.join("shaderpacks/bsl-10.zip").is_file());

    fake.add(project("atm", Kind::Modpack));
    fake.release("atm", "1", &[], b"mrpack", false);
    let downloads = home.join("downloads");
    let path = fetch_modpack(&fake, &fake.project("atm").unwrap(), &downloads, &|_| {}).unwrap();
    assert_eq!(fs::read(path).unwrap(), b"mrpack");
    fake.release("atm", "2", &[], b"mrpack 2", true);
    assert!(fetch_modpack(&fake, &fake.project("atm").unwrap(), &downloads, &|_| {}).is_err());
    assert!(!downloads.join("atm-2.zip").exists());
    fs::remove_dir_all(&home).unwrap();
}
