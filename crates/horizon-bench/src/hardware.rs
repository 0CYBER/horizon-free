//! The machine a report was measured on. CPU, RAM and OS are detected here; the GPU and the display
//! come from the program that rendered (the game knows its adapter, the launcher its window).

use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hardware {
    /// `macos`, `linux`, `windows`.
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_threads: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<Gpu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    pub driver: String,
    /// `metal`, `vulkan`, `dx12`, `gl`.
    pub backend: String,
    /// `integrated`, `discrete`, `virtual`, `cpu`, `other`.
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Display {
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_hz: Option<u32>,
}

impl Hardware {
    pub fn detect() -> Hardware {
        Hardware {
            os: std::env::consts::OS.to_owned(),
            os_version: os_version(),
            arch: std::env::consts::ARCH.to_owned(),
            cpu: cpu(),
            cpu_threads: std::thread::available_parallelism().ok().map(|n| n.get()),
            ram_mb: ram_bytes().map(|b| b >> 20),
            gpu: None,
            display: None,
        }
    }
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    output("sw_vers", &["-productVersion"])
}

#[cfg(target_os = "macos")]
fn cpu() -> Option<String> {
    output("sysctl", &["-n", "machdep.cpu.brand_string"])
}

#[cfg(target_os = "macos")]
fn ram_bytes() -> Option<u64> {
    output("sysctl", &["-n", "hw.memsize"])?.parse().ok()
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    let release = std::fs::read_to_string("/etc/os-release").ok()?;
    let name = key_value(&release, "PRETTY_NAME", '=')?
        .trim_matches('"')
        .to_owned();
    Some(match output("uname", &["-r"]) {
        Some(kernel) => format!("{name} (kernel {kernel})"),
        None => name,
    })
}

#[cfg(target_os = "linux")]
fn cpu() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    // x86 names the CPU per core; ARM boards name the machine once.
    key_value(&info, "model name", ':')
        .or_else(|| key_value(&info, "Model", ':'))
        .map(str::to_owned)
}

#[cfg(target_os = "linux")]
fn ram_bytes() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = key_value(&info, "MemTotal", ':')?
        .trim_end_matches("kB")
        .trim();
    kb.parse::<u64>().ok().map(|kb| kb * 1024)
}

#[cfg(target_os = "linux")]
fn key_value<'a>(text: &'a str, key: &str, sep: char) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(sep)?;
        (k.trim() == key).then(|| v.trim())
    })
}

#[cfg(target_os = "windows")]
fn os_version() -> Option<String> {
    output("cmd", &["/C", "ver"])
}

#[cfg(target_os = "windows")]
fn cpu() -> Option<String> {
    std::env::var("PROCESSOR_IDENTIFIER").ok()
}

#[cfg(target_os = "windows")]
fn ram_bytes() -> Option<u64> {
    let out = output(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ],
    )?;
    out.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn os_version() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn cpu() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn ram_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_this_machine() {
        let hw = Hardware::detect();
        assert_eq!(hw.os, std::env::consts::OS);
        assert!(hw.cpu_threads.unwrap() >= 1);
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(hw.cpu.is_some(), "{hw:?}");
            assert!(hw.ram_mb.unwrap() > 256, "{hw:?}");
            assert!(hw.os_version.is_some(), "{hw:?}");
        }
    }
}
