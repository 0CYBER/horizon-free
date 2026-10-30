//! `horizon-prepare`: prepares Horizon's resources from your own Minecraft Java Edition.
//! Its flags are the ones tools/prepare_native_client.py took.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// This release's version: release.yml builds it with `HORIZON_VERSION` from the tag
/// (docs/versioning.md); a build from source says Cargo.toml's.
const VERSION: &str = match option_env!("HORIZON_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

const HELP: &str = "\
horizon-prepare: prepare Horizon's resources from your own Minecraft Java Edition

Usage: horizon-prepare [--prepare] [--server-jar PATH] [--minecraft-dir PATH] [--root PATH]
       horizon-prepare compile-exporters [--root PATH]
       horizon-prepare sounds [--minecraft-dir PATH] [--root PATH]
       horizon-prepare panorama [--minecraft-dir PATH] [--root PATH]
       horizon-prepare fonts [--minecraft-dir PATH] [--root PATH]
       horizon-prepare packs [--minecraft-dir PATH] [--root PATH]
       horizon-prepare languages [--minecraft-dir PATH] [--root PATH]

  --prepare             Import the pinned game files from your own copy first
  --server-jar PATH     The pinned server jar you downloaded yourself
                        (default: HORIZON_SERVER_JAR, or the copy imported before)
  --minecraft-dir PATH  The official launcher's game directory with your installed copy
                        (default: HORIZON_MINECRAFT_DIR, or the launcher's own)
  --root PATH           The Horizon folder to prepare (default: the folder this program is
                        in, else the nearest folder above the current one with research-lock.json)

Needs Minecraft played once from the official launcher, and Java of the version
research-lock.json pins. Releases use a runtime; development or changed exporters need a JDK.
compile-exporters builds and validates a release bundle from the already imported pinned jars.
The resources go to <root>/.local/horizon-client/<version>; pass that to horizon-client --resources.

horizon-prepare panorama repairs the classic menu background in existing resources without Java.
horizon-prepare fonts adds the GNU Unifont glyphs (characters the bitmap fonts lack) the same way.
horizon-prepare packs adds the built-in Programmer Art and High Contrast resource packs the same way.
horizon-prepare languages adds the bundled languages and the language list the JAR lacks the same way.
Full preparation also imports all four after extracting the client JAR's placeholders.

horizon-prepare sounds copies the pinned version's sounds (about 380 MB) from the official
launcher's asset store into the same folder, checking every file. It is optional: without it the
game is silent. Nothing is downloaded; it needs the version played once from the official launcher.
";

/// The folder with research-lock.json: beside the executable in a release folder, else the
/// current folder or one above it (a checkout, with the binary in target/).
fn default_root() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = beside.filter(|d| d.join("research-lock.json").is_file()) {
        return Some(dir);
    }
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors()
        .find(|d| d.join("research-lock.json").is_file())
        .map(Path::to_path_buf)
}

fn main() -> ExitCode {
    let mut options = horizon_prepare::Options {
        root: PathBuf::new(),
        import: false,
        minecraft_dir: None,
        server_jar: None,
        java_home: None,
    };
    let mut root = None;
    let mut args = std::env::args_os().skip(1).peekable();
    let assets = args.next_if(|a| {
        matches!(
            a.to_str(),
            Some("sounds" | "panorama" | "fonts" | "packs" | "languages" | "compile-exporters")
        )
    });
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        let (flag, inline) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (text.clone(), None),
        };
        let mut value = |name: &str| -> Result<PathBuf, String> {
            match &inline {
                Some(v) => Ok(PathBuf::from(v)),
                None => args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{name} needs a path")),
            }
        };
        let parsed = match flag.as_str() {
            "--prepare" | "--server-jar" if assets.is_some() => {
                Err(format!("{flag} is for preparing, not for asset copying"))
            }
            "--prepare" if inline.is_none() => {
                options.import = true;
                Ok(())
            }
            "--server-jar" => value("--server-jar").map(|v| options.server_jar = Some(v)),
            "--minecraft-dir" => value("--minecraft-dir").map(|v| options.minecraft_dir = Some(v)),
            "--root" => value("--root").map(|v| root = Some(v)),
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("horizon-prepare {VERSION}");
                return ExitCode::SUCCESS;
            }
            _ => Err(format!("unrecognized argument: {text}")),
        };
        if let Err(e) = parsed {
            eprintln!("horizon-prepare: error: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    }
    let Some(root) = root.or_else(default_root) else {
        eprintln!(
            "horizon-prepare: error: no research-lock.json beside this program or above the current folder; pass --root"
        );
        return ExitCode::from(2);
    };
    options.root = root;
    let log = |line: &str| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    };
    let result = match assets.as_deref().and_then(|a| a.to_str()) {
        Some("compile-exporters") => horizon_prepare::compile_exporters(&options, &log)
            .inspect(|dir| log(&dir.display().to_string())),
        Some("sounds") => {
            horizon_prepare::sounds(&options.root, options.minecraft_dir.as_deref(), &log)
        }
        Some("panorama") => {
            horizon_prepare::panorama(&options.root, options.minecraft_dir.as_deref(), &log)
        }
        Some("fonts") => {
            horizon_prepare::fonts(&options.root, options.minecraft_dir.as_deref(), &log)
        }
        Some("packs") => {
            horizon_prepare::packs(&options.root, options.minecraft_dir.as_deref(), &log)
        }
        Some("languages") => {
            horizon_prepare::languages(&options.root, options.minecraft_dir.as_deref(), &log)
        }
        _ => horizon_prepare::prepare(&options, &log),
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
