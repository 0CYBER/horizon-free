#!/usr/bin/env python3
"""Compare two real Horizon JSON reports and flag mismatched workloads."""

import argparse
import json
from pathlib import Path

MATCH_FIELDS = ("minecraft", "kind", "os", "architecture", "gpu", "graphics_backend", "graphics_driver",
                "java_version", "heap_max_bytes", "framebuffer_width", "framebuffer_height", "render_distance",
                "simulation_distance", "vsync", "fps_limit", "graphics_preset", "ambient_occlusion",
                "improved_transparency", "mipmap_levels", "entity_distance_scaling", "entity_shadows", "particles",
                "clouds", "cloud_range")


def compare(baseline, candidate):
    for report in [baseline, candidate]:
        if report.get("schema") != 1 or report.get("kind") not in ["frames", "ticks"]:
            raise ValueError("Expected a schema-1 Horizon frame/tick report")
        if report.get("stats", {}).get("count", 0) < 1:
            raise ValueError("Report contains no samples")
    if baseline["kind"] != candidate["kind"]:
        raise ValueError("Compare frames with frames, or ticks with ticks")
    mismatches = [f"{key}: {baseline.get(key)!r} vs {candidate.get(key)!r}" for key in MATCH_FIELDS
                  if baseline.get(key) != candidate.get(key)]
    warnings = ["World, route, warm-up, shader state and LOD cache must also match; these are not verified by this tool."]
    for name, report in [("baseline", baseline), ("candidate", candidate)]:
        if report.get("dropped_samples", 0):
            warnings.append(f"{name} exceeded the recording buffer: {report['dropped_samples']} dropped samples.")
        if report.get("elapsed_seconds", 0) < 60:
            warnings.append(f"{name} is shorter than 60 seconds; use longer repeated runs for decisions.")
    if baseline["kind"] == "ticks" and baseline.get("peak_players") != candidate.get("peak_players"):
        mismatches.append("Peak player count differs")
    rows = []
    keys = ["meanRate", "lowOnePercentRate", "p95Ms", "p99Ms", "maxMs"] if baseline["kind"] == "frames" else ["meanMs", "p95Ms", "p99Ms", "maxMs"]
    for key in keys:
        before, after = baseline["stats"][key], candidate["stats"][key]
        rows.append({"metric": key, "baseline": before, "candidate": after,
                     "change_percent": 100 * (after / before - 1) if before else None})
    for key in ["heap_used_end_bytes", "heap_used_peak_sampled_bytes", "nonheap_used_peak_sampled_bytes",
                "process_cpu_mean_cores", "gc_time_ms", "observed_ticks_per_second"]:
        if key in baseline and key in candidate:
            before, after = baseline[key], candidate[key]
            rows.append({"metric": key, "baseline": before, "candidate": after,
                         "change_percent": 100 * (after / before - 1) if before else None})
    return {"kind": baseline["kind"], "mismatches": mismatches, "warnings": warnings, "metrics": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--strict", action="store_true", help="Exit nonzero on unmatched captured settings")
    args = parser.parse_args()
    result = compare(json.loads(args.baseline.read_text()), json.loads(args.candidate.read_text()))
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        for row in result["metrics"]:
            change = "n/a" if row["change_percent"] is None else f"{row['change_percent']:+.1f}%"
            print(f"{row['metric']:25} {row['baseline']:12.3f} -> {row['candidate']:12.3f} ({change})")
        for mismatch in result["mismatches"]:
            print("UNMATCHED: " + mismatch)
        for warning in result["warnings"]:
            print("NOTE: " + warning)
    return int(args.strict and bool(result["mismatches"]))


if __name__ == "__main__":
    raise SystemExit(main())
