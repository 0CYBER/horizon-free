//! Download progress presentation for terminal and redirected CLI output.
use horizon_launcher::native_mods;
use std::io::{IsTerminal, Write};

/// Download progress on one terminal line, only when stderr is a terminal and the percentage moves.
pub(super) fn progress(label: String) -> impl Fn(f32) {
    let last = std::cell::Cell::new(-1);
    let tty = std::io::stderr().is_terminal();
    move |p| {
        let pct = (p * 100.) as i32;
        if !tty || pct == last.replace(pct) {
            return;
        }
        eprint!("\r{label} {pct:>3}%");
        let _ = std::io::stderr().flush();
        if p >= 1.0 {
            eprintln!();
        }
    }
}

/// Keep detailed terminal updates, but only the first and completed download updates in logs.
pub(super) fn native_progress(tty: bool) -> impl Fn(native_mods::Step) -> Option<String> {
    use native_mods::Step;
    let previous = std::cell::RefCell::new(None);
    move |step| {
        let show = tty
            || match (&step, previous.borrow().as_ref()) {
                (Step::Rust(done), Some(Step::Rust(last))) => *done >= 1.0 && *last < 1.0,
                (
                    Step::Tools { build, done, total },
                    Some(Step::Tools {
                        build: last_build,
                        done: last_done,
                        total: last_total,
                    }),
                ) if build == last_build => {
                    *total > 0 && done >= total && (*last_total == 0 || last_done < last_total)
                }
                _ => true,
            };
        let label = show.then(|| step.label());
        *previous.borrow_mut() = Some(step);
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_progress_limits_redirected_rust_downloads_to_start_and_end() {
        let progress = native_progress(false);
        let fractions = [
            0.0, 0.0, 0.01, 0.97, 0.5, 0.971, 0.974, 0.98, 0.9999, 1.0, 1.0,
        ];
        let labels: Vec<_> = fractions
            .into_iter()
            .filter_map(|done| progress(native_mods::Step::Rust(done)))
            .collect();
        assert_eq!(
            labels,
            [
                format!(
                    "Downloading Rust {} (once): 0%",
                    horizon_launcher::rust_toolchain::VERSION
                ),
                format!(
                    "Downloading Rust {} (once): 100%",
                    horizon_launcher::rust_toolchain::VERSION
                ),
            ]
        );
    }

    #[test]
    fn native_progress_keeps_detailed_terminal_downloads() {
        let progress = native_progress(true);
        for (done, percent) in [
            (0.0, 0),
            (0.97, 97),
            (0.971, 97),
            (0.974, 97),
            (0.98, 98),
            (1.0, 100),
        ] {
            assert_eq!(
                progress(native_mods::Step::Rust(done)),
                Some(format!(
                    "Downloading Rust {} (once): {percent}%",
                    horizon_launcher::rust_toolchain::VERSION
                ))
            );
        }
        for done in [0, 1, 97, 100] {
            let step = native_mods::Step::Tools {
                build: "sdk".into(),
                done: done * 1_000_000,
                total: 100_000_000,
            };
            assert_eq!(
                progress(step),
                Some(format!("Getting the mod tools for sdk: {done} of 100 MB"))
            );
        }
    }

    #[test]
    fn native_progress_limits_each_redirected_sdk_download() {
        let progress = native_progress(false);
        let tools = |build: &str, done, total| native_mods::Step::Tools {
            build: build.into(),
            done,
            total,
        };
        for (done, expected) in [
            (0, Some("Getting the mod tools for first: 0 of 100 MB")),
            (1, None),
            (97, None),
            (50, None),
            (99, None),
            (100, Some("Getting the mod tools for first: 100 of 100 MB")),
            (100, None),
        ] {
            assert_eq!(
                progress(tools("first", done * 1_000_000, 100_000_000)),
                expected.map(str::to_owned)
            );
        }
        assert_eq!(
            progress(tools("second", 20_000_000, 100_000_000)),
            Some("Getting the mod tools for second: 20 of 100 MB".into())
        );
        assert_eq!(progress(tools("second", 50_000_000, 100_000_000)), None);
        assert_eq!(
            progress(tools("second", 100_000_000, 100_000_000)),
            Some("Getting the mod tools for second: 100 of 100 MB".into())
        );
        assert_eq!(
            progress(tools("third", 0, 0)),
            Some("Getting the mod tools for third: 0 of 1 MB".into())
        );
        assert_eq!(progress(tools("third", 0, 0)), None);
        assert_eq!(
            progress(tools("third", 100_000_000, 100_000_000)),
            Some("Getting the mod tools for third: 100 of 100 MB".into())
        );
    }

    #[test]
    fn native_progress_keeps_phase_order_and_restarts_download_reporting() {
        use native_mods::Step;
        let progress = native_progress(false);
        assert_eq!(
            progress(Step::Rust(0.97)),
            Some(format!(
                "Downloading Rust {} (once): 97%",
                horizon_launcher::rust_toolchain::VERSION
            ))
        );
        assert_eq!(progress(Step::Rust(0.98)), None);
        let labels: Vec<_> = [
            Step::PrebuiltMods,
            Step::Compile {
                done: 1,
                total: Some(2),
                name: "first".into(),
                profile: Default::default(),
            },
            Step::Compile {
                done: 2,
                total: Some(2),
                name: "second".into(),
                profile: Default::default(),
            },
            Step::Install,
            Step::Rust(0.0),
            Step::Rust(1.0),
        ]
        .into_iter()
        .filter_map(progress)
        .collect();
        assert_eq!(
            labels,
            vec![
                "Compiling the mods against the prebuilt game".to_owned(),
                "Compiling the game (optimized): 1 of 2 crates, now first".to_owned(),
                "Compiling the game (optimized): 2 of 2 crates, now second".to_owned(),
                "Installing the build".to_owned(),
                format!(
                    "Downloading Rust {} (once): 0%",
                    horizon_launcher::rust_toolchain::VERSION
                ),
                format!(
                    "Downloading Rust {} (once): 100%",
                    horizon_launcher::rust_toolchain::VERSION
                ),
            ]
        );
    }
}
