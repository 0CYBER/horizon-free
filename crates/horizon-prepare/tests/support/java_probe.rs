//! A fake Java tool for process-level JDK selection tests; no installed Java is needed.
use std::{env, fs, process};

fn main() {
    let executable = env::current_exe().unwrap();
    let bin = executable.parent().unwrap();
    let tool = executable.file_stem().unwrap().to_str().unwrap();
    let version = fs::read_to_string(bin.join(format!("{tool}.version"))).unwrap();
    if env::args().nth(1).as_deref() == Some("-version") {
        if tool == "javac" {
            println!("javac {version}");
        } else {
            eprintln!("openjdk version \"{version}\"");
        }
    } else {
        println!("{tool} {version}");
    }
    if let Ok(status) = fs::read_to_string(bin.join(format!("{tool}.exit"))) {
        process::exit(status.parse().unwrap());
    }
}
