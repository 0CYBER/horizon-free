//! The "No AI" switch: one setting that hides and turns off every AI feature of Horizon (the
//! companion, agent control through the control socket, `horizon-ctl`, its MCP server and the
//! launcher's). The launcher and the game both read and write it here, so they always agree.
//!
//! It is the `noAi` line of the game's `horizon-extras.txt`, in the game's home (the folder
//! with `options.txt`). Missing means off: AI features are available. See docs/agent-control.md.
use std::{fs, io, path::Path};

/// The game's extras file, beside `options.txt`.
pub const FILE: &str = "horizon-extras.txt";
/// The key No AI is kept under there.
pub const KEY: &str = "noAi";
/// The one line every AI surface answers with while No AI is on.
pub const REFUSAL: &str = "AI features are off: turn off No AI in the launcher's settings";

/// Whether `text` (an extras file) says No AI.
pub fn parse(text: &str) -> bool {
    text.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(k, v)| k.trim() == KEY && v.trim() == "true")
    })
}

/// Whether No AI is on for the game whose home is `home`.
pub fn is_on(home: &Path) -> bool {
    fs::read_to_string(home.join(FILE)).is_ok_and(|text| parse(&text))
}

/// `text` with its `noAi` line set to `on`, every other line kept as it was.
pub fn with(text: &str, on: bool) -> String {
    let mut out: String = text
        .lines()
        .filter(|line| line.split_once(':').is_none_or(|(k, _)| k.trim() != KEY))
        .flat_map(|line| [line, "\n"])
        .collect();
    out.push_str(&format!("{KEY}:{on}\n"));
    out
}

/// Turns No AI on or off for the game in `home`, keeping the file's other settings. Turning it
/// off where there is no file writes nothing (off is the default).
pub fn set(home: &Path, on: bool) -> io::Result<()> {
    let path = home.join(FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if !on {
                return Ok(());
            }
            String::new()
        }
        Err(e) => return Err(e),
    };
    fs::create_dir_all(home)?;
    fs::write(path, with(&text, on))
}

/// `Err(REFUSAL)` while No AI is on in `home`.
pub fn check(home: &Path) -> Result<(), &'static str> {
    if is_on(home) { Err(REFUSAL) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_the_extras_line_keeping_the_others() {
        assert!(!parse(""));
        assert!(!parse("noAi:false\nzoom:true\n"));
        assert!(parse("zoom:true\nnoAi: true\n"));
        assert_eq!(
            with("zoom:true\nnoAi:false\nfps:true", true),
            "zoom:true\nfps:true\nnoAi:true\n"
        );
        assert_eq!(with("", false), "noAi:false\n");
    }

    #[test]
    fn set_writes_only_when_needed() {
        let home = std::env::temp_dir().join(format!("horizon-no-ai-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        set(&home, false).unwrap();
        assert!(!home.join(FILE).exists(), "off needs no file");
        assert!(check(&home).is_ok());
        set(&home, true).unwrap();
        assert!(is_on(&home));
        assert_eq!(check(&home), Err(REFUSAL));
        fs::write(home.join(FILE), "fps:true\nnoAi:true\n").unwrap();
        set(&home, false).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(FILE)).unwrap(),
            "fps:true\nnoAi:false\n"
        );
        let _ = fs::remove_dir_all(&home);
    }
}
