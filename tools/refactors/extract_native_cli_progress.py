#!/usr/bin/env python3
"""Move CLI progress presentation and its existing tests out of the CLI parser."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
cli = root / "launcher/src/cli.rs"
source = cli.read_text()
start = source.index("/// Download progress on one terminal line")
end = source.index("fn json(", start)
presentation = source[start:end]
test_start = source.index("    #[test]\n    fn native_progress_limits_redirected_rust_downloads")
test_end = source.index("    #[test]\n    fn attached_preserves_real_process_codes", test_start)
tests = source[test_start:test_end]
source = source[:test_start] + source[test_end:]
source = source[:start] + source[end:]
source = source.replace("use clap::{Parser, Subcommand};", "mod progress;\nuse progress::{native_progress, progress};\n\nuse clap::{Parser, Subcommand};", 1)
presentation = presentation.replace("fn progress(", "pub(super) fn progress(", 1).replace("fn native_progress(", "pub(super) fn native_progress(", 1)
module = "//! Download progress presentation for terminal and redirected CLI output.\nuse horizon_launcher::native_mods;\nuse std::io::{IsTerminal, Write};\n\n" + presentation + "#[cfg(test)]\nmod tests {\n    use super::*;\n\n" + tests + "}\n"
(root / "launcher/src/cli/progress.rs").write_text(module)
cli.write_text(source)
