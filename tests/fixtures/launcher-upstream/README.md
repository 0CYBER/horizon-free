# Upstream identity baseline

Packaging file fixtures are verbatim git objects from `da81e38705` before this
change, obtained with `git show da81e38705:packaging/...`. Underscores in fixture
names encode path separators. `icon-sha256.json` records SHA-256 of the baseline
git objects under `packaging/icons`, not hashes calculated from staging output.

`identity.json` transcribes the pre-change launcher identity contract from
`account.rs`, `data_location.rs`, `backend.rs`, `installer.rs`, and `main.rs` at
that same commit. The metadata test compiles the real identity module twice and
compares upstream output to this independent baseline; the fork output must use
disjoint namespaces. The separate original workflow fixture under
`launcher/tests/` pins upstream asset names while the fork workflow is deliberately
dry-run-only and targets only Linux x86_64 and Windows x64.

These checks compare metadata, not generated package archives. OS installation,
keychain access, Authenticode, codesigning, Gatekeeper, packaged GUI coexistence
and real self-update execution require platform runs and are not established by
these fixtures.
