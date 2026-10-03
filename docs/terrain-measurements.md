# Capture terrain quality as well as speed

Version 0.12 adds effective DH configuration to frame reports: LOD radius,
horizontal/vertical quality, maximum horizontal resolution, renderer mode,
rendering enable setting, transparency, shading, ambient occlusion, LOD-only
mode, camera quality falloff, worker count and runtime ratio. It reads the
actual public API values, including active overrides. Pack names and TOML files
alone do not establish which settings reached the running game.

Reports also capture configured field of view and optional Iris enabled state.
An enabled Iris pack's name is included when available. Iris is queried through
its public API; missing or incompatible APIs are marked unavailable, rather than
assumed disabled. Other shader frameworks are not observed by this adapter.

Visual settings are read at recording start and finish. The final snapshot is
stored in `visual_settings_end`; `visual_settings_changed` lists detected changes.
The viewer and strict CLI flag these runs as mismatched. An adaptive runtime-ratio
change is expected and is displayed separately; it is not treated as a visual
quality change. Snapshots cannot detect a change that was restored before the
end. Keep settings fixed and avoid menus during the actual comparison.

The comparison tools check captured DH/Iris quality fields, and report different
worker budgets or controller settings as experiment variables. Older reports
without these fields show unknown quality/shader equivalence. A vanilla baseline
without DH versus a distant-terrain profile intentionally has different visual
work: inspect that difference rather than presenting it as equal draw settings.

These are configuration observations, not counts of visible geometry, completed
terrain or GPU timings. A 128-chunk configured radius does not prove every chunk
has generated or drawn. Warm-up, world/route equivalence, occlusion, terrain cache,
weather, actual visual detail and external shader frameworks still need testing.
Base FOV does not capture every transient sprint/zoom camera effect.

The adapter reads at capture boundaries on the owning client thread. It performs
no per-frame polling and changes no settings. Tests exercise the pinned upstream
API contracts, effective overrides, absent/unavailable APIs and comparison
mismatches. Game smoke checks validate the actual captured values separately.
