//! `horizon-server`: host a Horizon server (docs/server.md). It keeps the server's settings in `server.toml`,
//! runs the game headless as the engine, and manages the vanilla files around it.

mod config;
mod engine;
mod init;
mod lists;
mod signals;
mod supervisor;

use std::{path::PathBuf, process::ExitCode};

const HELP: &str = "\
horizon-server: host a Horizon (Minecraft Java Edition 26.3) server

Usage: horizon-server [--dir DIR] <command>

  init        Set up a server here: the EULA, the world, the port, who may join (asks, or takes flags:
              --eula --world NAME --seed SEED --port N --whitelist/--no-whitelist --player NAME... --op NAME
              --yes)
  run         Run the server in the foreground; type console commands, Ctrl-C stops it cleanly. It
              restarts the game if it crashes.
  import FILE Write server.toml from a vanilla server.properties
  op NAME, deop NAME
  whitelist on|off|list, whitelist add|remove NAME
              Change who may join while the server is stopped (while it runs, type the same command
              in its console)

  --dir DIR   The server folder (default: the current folder)

Settings live in server.toml; the server writes server.properties from it before every start.
";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = PathBuf::from(".");
    if let Some(i) = args.iter().position(|a| a == "--dir") {
        if i + 1 >= args.len() {
            eprintln!("horizon-server: --dir needs a folder");
            return ExitCode::from(2);
        }
        dir = PathBuf::from(args.remove(i + 1));
        args.remove(i);
    }
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            Ok(())
        }
        Some("version" | "-V" | "--version") => {
            println!("horizon-server {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("init") => init_answers(&args[1..]).and_then(|a| init::run(&dir, a)),
        Some("run") => supervisor::run(&dir),
        Some("import") => import(&dir, &args[1..]),
        Some(command @ ("op" | "deop" | "whitelist")) => lists::run(&dir, command, &args[1..]),
        Some(other) => Err(format!(
            "unknown command {other}; `horizon-server help` lists them"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("horizon-server: {e}");
            ExitCode::FAILURE
        }
    }
}

fn init_answers(args: &[String]) -> Result<init::Answers, String> {
    let mut answers = init::Answers::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--eula" => answers.eula = true,
            "--yes" | "-y" => answers.yes = true,
            "--world" => answers.world = Some(value(arg)?),
            "--seed" => answers.seed = Some(value(arg)?),
            "--port" => {
                answers.port = Some(
                    value(arg)?
                        .parse()
                        .map_err(|_| "--port needs a number from 1 to 65535")?,
                );
            }
            "--whitelist" => answers.whitelist = Some(true),
            "--no-whitelist" => answers.whitelist = Some(false),
            "--player" => answers.players.push(value(arg)?),
            "--op" => answers.op = Some(value(arg)?),
            other => return Err(format!("init: unknown option {other}")),
        }
    }
    Ok(answers)
}

fn import(dir: &std::path::Path, args: &[String]) -> Result<(), String> {
    let [source] = args else {
        return Err("import needs the server.properties to read".into());
    };
    let target = dir.join(config::FILE);
    if target.exists() {
        return Err(format!(
            "{} already exists; move it away first",
            target.display()
        ));
    }
    let cfg = config::Config::import(&config::read_properties(std::path::Path::new(source))?)?;
    std::fs::write(&target, config::render(&cfg)).map_err(|e| e.to_string())?;
    println!("Wrote {}", target.display());
    Ok(())
}
