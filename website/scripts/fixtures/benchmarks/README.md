# Benchmark summary fixtures

Shared by the website's aggregator test (`website/scripts/benchmarks.test.mjs`) and the telemetry server's
`GET /v1/benchmarks/summary` test: the same `benchmark` items in, the same groups out.

- `items.jsonl`: one stored item per line (`install_id`, `day`, `version`, `data`: a benchmark report). Group
  RTX 4080 + i7-13700KF at render distance 12 has three installs (one with two runs); the same install at 32,
  and the RTX 3060 group, have fewer than three and are left out. The other runs are rejected: VSync on,
  offscreen, a frame cap, a missing scene, a one-scene scenario, a report of another version, no install ID.
- `expected.json`: `min_installs` and `groups` of the summary (`generated` varies).
