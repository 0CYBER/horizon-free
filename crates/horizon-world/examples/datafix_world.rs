//! Upgrade a copy of a world saved by 1.20 to 26.2 to the 26.3 format, as
//! vanilla 26.3 does when it opens it and reads every file:
//! `datafix_world IN_WORLD OUT_WORLD` (used by tools/test_datafix_parity.py).
use horizon_world::datafix::files;
use std::{env, fs, io, path::Path, process::ExitCode};

fn copy(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy(&entry.path(), &target)?;
        } else if entry.file_name() != "session.lock" {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: datafix_world IN_WORLD OUT_WORLD");
        return ExitCode::from(2);
    };
    let output = Path::new(output);
    let result = (|| {
        if output.exists() {
            fs::remove_dir_all(output)?;
        }
        copy(Path::new(input), output)?;
        files::upgrade_world(output)
    })();
    match result {
        Ok(version) => {
            println!("upgraded {} from DataVersion {version}", output.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("datafix_world: {e}");
            ExitCode::FAILURE
        }
    }
}
