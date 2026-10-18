**pr_ready** (`tools/pr_ready.py --quick` on archlinux, against origin/main at e4f8154cb, 54 s)

| Check | Result |
| --- | --- |
| Scope | ⚠️ 39 files, +9538 −6; 9450 lines of code: over 500, split it if it can ship in parts |
| Cargo.lock | ✅ unchanged |
| Public tree | ✅ |
| Python tests | ✅ 236 passed (18 s) |
| rustfmt | ✅ (5 s) |
| clippy | ✅ (29 s) |
| tests: horizon-core | ✅ 94 passed (1 s) |
| tokens.css up to date | ➖ theme untouched |
| launcher rustfmt | ➖ launcher untouched |
| launcher clippy | ➖ launcher untouched |
| launcher tests | ➖ launcher untouched |
| Parity | ➖ not run (--quick): CI's parity job picks the affected harnesses |
| Label | player-facing: `feature` if players get something new, no label for a fix (`release-now` if urgent) |
