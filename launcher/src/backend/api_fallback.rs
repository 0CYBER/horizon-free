//! What the launcher remembers of a graphics API it set itself, per game folder.
//!
//! After a native crash at the renderer's start the launcher moves `graphicsApi` from Auto to
//! another API (`switch_after_start_crash`). That stays as it is; this file makes the switch to
//! DirectX 12 visible and undoable. The switch is recorded in [`FILE`] in the folder it happened in, and the instance
//! shows a notice ([`notice`]) until the player answers it: [`try_previous`] writes the earlier
//! setting back, as choosing it in the game's graphics options does, and [`keep`] dismisses the
//! notice for good. A Windows folder whose settings say DirectX 12 with no record gets the same
//! question once, because launchers before the record set that without leaving a trace.
//!
//! The record never decides which API a start runs. One that is missing, unreadable, not a
//! regular file, or not exactly what this launcher writes reads as no record: nothing is
//! changed, Play is never held up, and at worst the question is asked once more. All of it is
//! Windows only: elsewhere no file is opened and nothing is shown.
use super::GRAPHICS_FILE;
use std::fs;
use std::path::Path;

/// The record's file in a game folder.
pub const FILE: &str = "horizon-launcher-graphics-api.txt";

/// An API's name for the player.
pub fn label(api: &str) -> &'static str {
    match api {
        "vulkan" => "Vulkan",
        "dx12" => "DirectX 12",
        "opengl" => "OpenGL",
        _ => "Automatic",
    }
}

/// A switch the launcher made after a start crash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    /// `graphicsApi` before and after.
    pub from: String,
    pub to: String,
    /// When, in seconds since 1970.
    pub when: u64,
    /// The game version that crashed.
    pub build: String,
}

/// What [`FILE`] says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub switch: Option<Switch>,
    /// The player answered the notice: it is not shown again until another switch.
    pub decided: bool,
}

impl Record {
    /// Only the two texts [`Record::serialize`] writes are a record: an answer alone, or a
    /// switch from Auto to DirectX 12 and whether it was answered. Anything else is none, so
    /// no other API can come out of a record.
    pub fn parse(text: &str) -> Self {
        fn decided(line: &str) -> Option<bool> {
            match line {
                "decided:true" => Some(true),
                "decided:false" => Some(false),
                _ => None,
            }
        }
        fn switched(when: &str, build: &str, answer: &str) -> Option<Record> {
            Some(Record {
                switch: Some(Switch {
                    from: "auto".to_owned(),
                    to: "dx12".to_owned(),
                    when: when.strip_prefix("when:")?.parse().ok()?,
                    build: build.strip_prefix("build:")?.to_owned(),
                }),
                decided: decided(answer)?,
            })
        }
        let Some(lines) = text.strip_suffix('\n') else {
            return Self::default();
        };
        let lines: Vec<&str> = lines.split('\n').collect();
        let parsed = match lines[..] {
            [answer] => decided(answer).map(|decided| Self {
                switch: None,
                decided,
            }),
            ["from:auto", "to:dx12", when, build, answer] => switched(when, build, answer),
            _ => None,
        };
        // And only byte for byte what the writer makes of it: no control character in the
        // build, no other spelling of the number.
        parsed
            .filter(|record| record.serialize() == text)
            .unwrap_or_default()
    }

    pub fn serialize(&self) -> String {
        let mut text = String::new();
        if let Some(switch) = &self.switch {
            let build: String = switch.build.chars().filter(|c| !c.is_control()).collect();
            text += &format!(
                "from:{}\nto:{}\nwhen:{}\nbuild:{}\n",
                switch.from, switch.to, switch.when, build
            );
        }
        text + &format!("decided:{}\n", self.decided)
    }
}

/// The text of the regular file at `path`. Anything else there (a folder, a pipe, a device) is
/// not opened: reading it could wait forever.
fn regular(path: &Path) -> Result<String, String> {
    let read = match fs::metadata(path) {
        Ok(found) if !found.is_file() => Err("not a regular file".to_owned()),
        Ok(_) => fs::read_to_string(path).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    read.map_err(|e| format!("{}: {e}", path.display()))
}

/// `home`'s record; none for one that is missing, unreadable, not a regular file or damaged.
pub fn read(home: &Path) -> Record {
    regular(&home.join(FILE))
        .map(|text| Record::parse(&text))
        .unwrap_or_default()
}

/// Writes `text` to `path` by replacing the file: a temporary file of this process beside it,
/// then a rename, so a write that stops halfway leaves the old file whole.
fn replace(path: &Path, text: &str) -> Result<(), String> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let written = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, path));
    if let Err(e) = written {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

fn write(home: &Path, record: &Record) -> Result<(), String> {
    replace(&home.join(FILE), &record.serialize())
}

/// The `graphicsApi` of settings `text`; Auto when it names none.
fn api_of(text: &str) -> String {
    text.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "graphicsApi").then(|| value.trim().to_ascii_lowercase())
        })
        .unwrap_or_else(|| "auto".to_owned())
}

/// Settings `text` with its `graphicsApi` line naming `api`: every other byte as it is, the
/// line endings and a missing last one too.
fn with_api(text: &str, api: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\r', '\n']);
            match body.split_once(':') {
                Some((key, _)) if key.trim() == "graphicsApi" => {
                    format!("{key}:{api}{}", &line[body.len()..])
                }
                _ => line.to_owned(),
            }
        })
        .collect()
}

/// The launcher moved `home` from `from` to `to` after `build` crashed while starting: the
/// record says so, and the notice is due again whatever was answered before.
pub fn record_switch(home: &Path, from: &str, to: &str, build: &str) -> Result<(), String> {
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let switch = Switch {
        from: from.to_owned(),
        to: to.to_owned(),
        when,
        build: build.to_owned(),
    };
    write(
        home,
        &Record {
            switch: Some(switch),
            decided: false,
        },
    )
}

/// What an instance asks the player about its graphics API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The launcher made this switch and the settings still name its API.
    Switched(Switch),
    /// Windows: the settings name DirectX 12 and nothing says who chose it.
    FoundDx12,
}

/// The notice `home` is due on `os`, reading only: none anywhere but Windows, where no file is
/// opened either; none once the player answered, none when the settings can't be read, and
/// none when they no longer name DirectX 12.
pub fn notice(home: &Path, os: &str) -> Option<Notice> {
    if os != "windows" {
        return None;
    }
    let record = read(home);
    if record.decided || api_of(&regular(&home.join(GRAPHICS_FILE)).ok()?) != "dx12" {
        return None;
    }
    Some(record.switch.map_or(Notice::FoundDx12, Notice::Switched))
}

/// "Try Automatic again": writes Auto over DirectX 12, as choosing it in the game's graphics
/// options does, and remembers the answer. Auto is the only setting this ever writes, whatever
/// the record says. Settings that no longer name DirectX 12 are the player's own by now and
/// stay. True when the settings changed.
pub fn try_previous(home: &Path) -> Result<bool, String> {
    let record = read(home);
    let path = home.join(GRAPHICS_FILE);
    let text = regular(&path)?;
    let changed = api_of(&text) == "dx12";
    if changed {
        replace(&path, &with_api(&text, "auto"))?;
    }
    write(
        home,
        &Record {
            switch: record.switch,
            decided: true,
        },
    )?;
    Ok(changed)
}

/// "Keep": the settings stay, and the answer is remembered.
pub fn keep(home: &Path) -> Result<(), String> {
    write(
        home,
        &Record {
            switch: read(home).switch,
            decided: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "horizon-api-fallback-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn settings(home: &Path) -> String {
        fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap()
    }

    fn files(home: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(home)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A switch is recorded with what it was, read back, and asked about; a record that is
    /// missing or damaged reads as none and changes nothing.
    #[test]
    fn a_switch_is_recorded_and_a_damaged_record_is_none() {
        let home = scratch("record");
        assert_eq!(read(&home), Record::default());
        fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:dx12\n").unwrap();
        record_switch(&home, "auto", "dx12", "nightly-2026-11-22-2b77287").unwrap();
        let record = read(&home);
        let switch = record.switch.clone().unwrap();
        assert_eq!(
            (
                switch.from.as_str(),
                switch.to.as_str(),
                switch.build.as_str()
            ),
            ("auto", "dx12", "nightly-2026-11-22-2b77287")
        );
        assert!(switch.when > 0 && !record.decided);
        assert_eq!(Record::parse(&record.serialize()), record);
        assert_eq!(notice(&home, "windows"), Some(Notice::Switched(switch)));
        // Only the record and the settings are there: no temporary file stays.
        assert_eq!(files(&home), [GRAPHICS_FILE, FILE]);

        // Anything but the exact text this launcher writes is no record. It is then asked
        // about as DirectX 12 nobody recorded, and the only setting an answer writes is Auto.
        for damaged in [
            "",
            "\u{0}\u{1}garbage",
            "from:auto\n",
            "from:auto\nto:metal\n",
            "from:vulkan\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\n",
            "from:auto\nto:opengl\nwhen:1\nbuild:v1\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:soon\nbuild:v1\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:-1\nbuild:v1\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\n",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:yes\n",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\nextra:1\n",
            "from:auto\r\nto:dx12\r\nwhen:1\r\nbuild:v1\r\ndecided:false\r\n",
            " from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\n",
            "to:dx12\nfrom:auto\nwhen:1\nbuild:v1\ndecided:false\n",
            "decided:true\ndecided:true\n",
            // Nothing the writer could have written: a control character in the build,
            // another spelling of the number, a key twice, text after the end.
            "from:auto\nto:dx12\nwhen:1\nbuild:v\u{0}1\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\tb\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:+1\nbuild:v1\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:01\nbuild:v1\ndecided:false\n",
            "from:auto\nfrom:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\ndecided:false\n",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\ngarbage",
            "from:auto\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\n\n",
            "decided:true\n\u{0}",
        ] {
            fs::write(home.join(FILE), damaged).unwrap();
            assert_eq!(read(&home), Record::default(), "{damaged:?}");
            assert_eq!(settings(&home), "sky:false\ngraphicsApi:dx12\n");
            assert_eq!(notice(&home, "windows"), Some(Notice::FoundDx12));
        }
        fs::write(
            home.join(FILE),
            "from:vulkan\nto:dx12\nwhen:1\nbuild:v1\ndecided:false\n",
        )
        .unwrap();
        assert_eq!(try_previous(&home), Ok(true));
        assert_eq!(settings(&home), "sky:false\ngraphicsApi:auto\n");
        assert_eq!(
            read(&home),
            Record {
                switch: None,
                decided: true
            }
        );
        fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:dx12\n").unwrap();
        // A folder in the record's place can't be read either.
        fs::remove_file(home.join(FILE)).unwrap();
        fs::create_dir(home.join(FILE)).unwrap();
        assert_eq!(read(&home), Record::default());
        assert_eq!(notice(&home, "windows"), Some(Notice::FoundDx12));
        assert_eq!(notice(&home, "linux"), None);
        let _ = fs::remove_dir_all(&home);
    }

    /// Linux and macOS show nothing and open nothing, a recorded switch or not: a pipe in a
    /// file's place, which would hold a read forever, is never opened. Windows' rules never
    /// open one either: it is no record, and settings that can't be read.
    #[test]
    fn other_systems_show_nothing_and_open_nothing() {
        let home = scratch("other");
        fs::write(home.join(GRAPHICS_FILE), "graphicsApi:dx12\n").unwrap();
        record_switch(&home, "auto", "dx12", "v1").unwrap();
        for os in ["linux", "macos", "freebsd", ""] {
            assert_eq!(notice(&home, os), None, "{os}");
        }
        #[cfg(unix)]
        {
            let pipe = |name: &str| {
                let path = home.join(name);
                let _ = fs::remove_file(&path);
                let made = std::process::Command::new("mkfifo").arg(&path).status();
                assert!(made.unwrap().success());
            };
            pipe(FILE);
            assert_eq!(notice(&home, "linux"), None);
            assert_eq!(read(&home), Record::default());
            assert_eq!(notice(&home, "windows"), Some(Notice::FoundDx12));
            pipe(GRAPHICS_FILE);
            assert_eq!(notice(&home, "linux"), None);
            assert_eq!(notice(&home, "windows"), None);
            assert!(try_previous(&home).is_err());
        }
        let _ = fs::remove_dir_all(&home);
    }

    /// The answer's write changes the API's name and no other byte: Windows line endings stay,
    /// and so do odd spacing, a line without an ending and lines nobody knows.
    #[test]
    fn the_settings_keep_their_line_endings() {
        let home = scratch("endings");
        for (before, after) in [
            (
                "sky:false\r\ngraphicsApi:dx12\r\nfutureKey:1\r\n",
                "sky:false\r\ngraphicsApi:auto\r\nfutureKey:1\r\n",
            ),
            (
                "sky:false\r\ngraphicsApi:dx12\nfutureKey: 1 \n\n",
                "sky:false\r\ngraphicsApi:auto\nfutureKey: 1 \n\n",
            ),
            ("graphicsApi:dx12", "graphicsApi:auto"),
            ("a:1\r\ngraphicsApi:DX12\r", "a:1\r\ngraphicsApi:auto\r"),
            (" graphicsApi : dx12 \r\n", " graphicsApi :auto\r\n"),
        ] {
            fs::write(home.join(GRAPHICS_FILE), before).unwrap();
            let _ = fs::remove_file(home.join(FILE));
            assert_eq!(try_previous(&home), Ok(true), "{before:?}");
            assert_eq!(settings(&home), after);
        }
        let _ = fs::remove_dir_all(&home);
    }

    /// "Try Automatic again" writes the earlier setting back and keeps every other line, the
    /// unknown ones too; "Keep" leaves the settings alone. Either answer ends the notice.
    #[test]
    fn the_two_answers() {
        let before = "sky:false\ngraphicsApi:dx12\nfutureKey:1\nmsaa4x:false\n";
        let home = scratch("try");
        fs::write(home.join(GRAPHICS_FILE), before).unwrap();
        record_switch(&home, "auto", "dx12", "v1").unwrap();
        assert_eq!(try_previous(&home), Ok(true));
        assert_eq!(
            settings(&home),
            "sky:false\ngraphicsApi:auto\nfutureKey:1\nmsaa4x:false\n"
        );
        assert_eq!(notice(&home, "windows"), None);
        assert!(read(&home).decided && read(&home).switch.is_some());
        assert_eq!(files(&home), [GRAPHICS_FILE, FILE]);
        // The next start crashes again and the launcher switches again: the notice is back.
        fs::write(home.join(GRAPHICS_FILE), before).unwrap();
        record_switch(&home, "auto", "dx12", "v1").unwrap();
        assert!(matches!(
            notice(&home, "windows"),
            Some(Notice::Switched(_))
        ));

        keep(&home).unwrap();
        assert_eq!(settings(&home), before);
        assert_eq!(notice(&home, "windows"), None);
        assert!(read(&home).decided);
        // An API the player chose since the switch is theirs: no notice, and no answer
        // changes it.
        record_switch(&home, "auto", "dx12", "v1").unwrap();
        fs::write(home.join(GRAPHICS_FILE), "graphicsApi:vulkan\n").unwrap();
        assert_eq!(notice(&home, "windows"), None);
        assert_eq!(try_previous(&home), Ok(false));
        assert_eq!(settings(&home), "graphicsApi:vulkan\n");
        // Without settings to write back to, the answer says why and changes nothing.
        let empty = scratch("try-empty");
        assert!(try_previous(&empty).is_err());
        assert_eq!(files(&empty), Vec::<String>::new());
        for dir in [home, empty] {
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// Windows asks once about DirectX 12 nobody recorded: until either answer, never again
    /// after it, never for another API and never on Linux or macOS.
    #[test]
    fn dx12_without_a_record_is_asked_about_once_on_windows() {
        let home = scratch("found");
        for (api, os, asked) in [
            ("dx12", "windows", true),
            ("DX12", "windows", true),
            ("dx12", "linux", false),
            ("dx12", "macos", false),
            ("auto", "windows", false),
            ("vulkan", "windows", false),
            ("opengl", "windows", false),
        ] {
            fs::write(home.join(GRAPHICS_FILE), format!("graphicsApi:{api}\n")).unwrap();
            assert_eq!(notice(&home, os).is_some(), asked, "{api} {os}");
        }
        // No settings at all: nothing to ask about.
        fs::remove_file(home.join(GRAPHICS_FILE)).unwrap();
        assert_eq!(notice(&home, "windows"), None);
        assert_eq!(files(&home), Vec::<String>::new());

        let text = "graphicsApi:dx12\nfutureKey:1\n";
        fs::write(home.join(GRAPHICS_FILE), text).unwrap();
        assert_eq!(notice(&home, "windows"), Some(Notice::FoundDx12));
        assert_eq!(notice(&home, "windows"), Some(Notice::FoundDx12));
        keep(&home).unwrap();
        assert_eq!(settings(&home), text);
        assert_eq!(notice(&home, "windows"), None);
        assert_eq!(
            read(&home),
            Record {
                switch: None,
                decided: true
            }
        );

        // The other answer: Automatic, and DirectX 12 chosen afterwards is not asked about.
        let other = scratch("found-try");
        fs::write(other.join(GRAPHICS_FILE), text).unwrap();
        assert_eq!(try_previous(&other), Ok(true));
        assert_eq!(settings(&other), "graphicsApi:auto\nfutureKey:1\n");
        fs::write(other.join(GRAPHICS_FILE), text).unwrap();
        assert_eq!(notice(&other, "windows"), None);
        for dir in [home, other] {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
