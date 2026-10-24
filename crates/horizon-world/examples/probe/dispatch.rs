//! How the probe binary runs a probe. main.rs lists the probes and nothing else (tools/parity.py leaves it out of
//! the harnesses' dependencies); code every probe relies on goes here.
use std::process::ExitCode;

/// `probes! { a, b, }` declares the binary's `main`, which runs the probe module named by its first argument.
macro_rules! probes {
    ($($name:ident,)*) => {
        fn run(name: &str) -> Option<std::process::ExitCode> {
            use std::process::Termination;
            Some(match name {
                $(stringify!($name) => $name::main().report(),)*
                _ => return None,
            })
        }

        fn main() -> std::process::ExitCode {
            dispatch::main(&[$(stringify!($name)),*], run)
        }
    };
}

/// The probe's command line as its own example saw it: the program, then the arguments after the probe's name.
pub fn args() -> std::vec::IntoIter<String> {
    let mut args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        args.remove(1);
    }
    args.into_iter()
}

pub fn main(names: &[&str], run: fn(&str) -> Option<ExitCode>) -> ExitCode {
    let name = std::env::args().nth(1).unwrap_or_default();
    run(&name).unwrap_or_else(|| {
        eprintln!(
            "usage: world_probe <name> [arguments...]\nprobes: {}",
            names.join(" ")
        );
        ExitCode::from(2)
    })
}
