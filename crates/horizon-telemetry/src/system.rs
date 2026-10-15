//! The machine as an item describes it ([`System`]): what the OS says, read the same way on
//! every start. The game adds the GPU, its driver and the graphics backend.
use crate::payload::System;

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The value after `key` in a `key: value` or `key=value` listing.
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?;
        let value = rest.trim_start().strip_prefix([':', '='])?;
        Some(value.trim().trim_matches('"').to_owned())
    })
}

fn os_version() -> Option<String> {
    if cfg!(target_os = "linux") {
        read("/etc/os-release").and_then(|t| field(&t, "PRETTY_NAME"))
    } else if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"))
    } else if cfg!(windows) {
        command_output("cmd", &["/C", "ver"])
    } else {
        None
    }
}

fn cpu() -> Option<String> {
    if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| field(&t, "model name"))
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    }
}

fn ram_bytes() -> Option<u64> {
    if cfg!(target_os = "linux") {
        let kib = read("/proc/meminfo").and_then(|t| field(&t, "MemTotal"))?;
        kib.split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|k| k * 1024)
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "hw.memsize"])?
            .parse()
            .ok()
    } else if cfg!(windows) {
        command_output(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ],
        )?
        .parse()
        .ok()
    } else {
        None
    }
}

impl System {
    /// This machine, without the GPU.
    pub fn detect() -> Self {
        const GIB: f64 = (1u64 << 30) as f64;
        Self {
            os: std::env::consts::OS.to_owned(),
            os_version: os_version().unwrap_or_default(),
            arch: std::env::consts::ARCH.to_owned(),
            cpu: cpu().unwrap_or_default(),
            threads: std::thread::available_parallelism().map_or(0, |n| n.get() as u16),
            ram_gb: ram_bytes().map_or(0, |b| (b as f64 / GIB).round() as u16),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_read_from_listings() {
        let os_release =
            "NAME=\"Fedora Linux\"\nPRETTY_NAME=\"Fedora Linux 43 (Workstation Edition)\"\n";
        assert_eq!(
            field(os_release, "PRETTY_NAME").as_deref(),
            Some("Fedora Linux 43 (Workstation Edition)")
        );
        let cpuinfo = "processor\t: 0\nmodel name\t: AMD Ryzen 9 7950X 16-Core Processor\n";
        assert_eq!(
            field(cpuinfo, "model name").as_deref(),
            Some("AMD Ryzen 9 7950X 16-Core Processor")
        );
        assert_eq!(
            field("MemTotal:       32788148 kB", "MemTotal").as_deref(),
            Some("32788148 kB")
        );
        assert_eq!(field("NAME=x", "PRETTY_NAME"), None);
    }

    #[test]
    fn this_machine_is_described() {
        let system = System::detect();
        assert_eq!(system.os, std::env::consts::OS);
        assert!(system.threads > 0);
        assert!(system.gpu.is_empty(), "the game fills the GPU in");
    }
}
