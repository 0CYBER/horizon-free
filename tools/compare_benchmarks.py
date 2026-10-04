#!/usr/bin/env python3
"""Compare two real Horizon JSON reports and flag mismatched workloads."""

import argparse
import json
from pathlib import Path

MATCH_FIELDS = ("minecraft", "kind", "os", "os_version", "architecture", "gpu", "graphics_backend", "graphics_driver",
                "java_version", "heap_max_bytes", "framebuffer_width", "framebuffer_height", "render_distance",
                "simulation_distance", "vsync", "fps_limit", "graphics_preset", "ambient_occlusion",
                "improved_transparency", "mipmap_levels", "entity_distance_scaling", "entity_shadows", "particles",
                "clouds", "cloud_range", "fov", "world_difficulty", "dh_capture_status", "dh_rendering_enabled", "dh_renderer_mode",
                "dh_lod_radius_chunks", "dh_horizontal_quality", "dh_vertical_quality", "dh_max_horizontal_resolution",
                "dh_transparency", "dh_lod_shading", "dh_lod_only_mode", "dh_camera_quality_dropoff",
                "dh_ambient_occlusion", "iris_capture_status", "iris_shaders_enabled", "iris_shader_pack")


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
    if baseline["kind"] == "frames":
        for field in ["adaptive_budget_enabled", "adaptive_target_fps", "precise_worker_pacing", "debounce_queue_wait",
                      "wakeable_idle_wait", "dispatch_retry_backoff", "tick_membership_dedup", "empty_culling_whitelist", "interruptible_terrain_read", "dh_thread_count", "dh_runtime_ratio"]:
            if field in baseline and field in candidate and baseline[field] != candidate[field]:
                warnings.append(f"Experiment {field} differs: {baseline[field]!r} vs {candidate[field]!r}.")
        for name, report in [("baseline", baseline), ("candidate", candidate)]:
            if report.get("dh_capture_status") not in ["available", "not_installed"]:
                warnings.append(f"{name} distant-terrain configuration was not captured; quality equivalence is unknown.")
            if report.get("iris_capture_status") not in ["available", "not_installed"]:
                warnings.append(f"{name} Iris state was not captured; shader equivalence is unknown.")
            if report.get("visual_settings_changed"):
                mismatches.append(f"{name} visual settings changed during recording")
                warnings.append(f"{name} visual settings changed between capture boundaries: {report['visual_settings_changed']!r}.")
    for field in ["compact_object_headers", "garbage_collectors"]:
        if field not in baseline or field not in candidate:
            warnings.append(f"{field} was not captured in both reports; JVM configuration equivalence is unknown.")
        elif baseline[field] != candidate[field]:
            warnings.append(f"JVM {field} differs: {baseline[field]!r} vs {candidate[field]!r}. Treat this as an explicit experiment variable.")
    if abs(baseline.get("elapsed_seconds", 0) - candidate.get("elapsed_seconds", 0)) > max(
            2, min(baseline.get("elapsed_seconds", 0), candidate.get("elapsed_seconds", 0)) * 0.1):
        warnings.append("Recording durations differ; match durations when comparing memory peaks and collection totals.")
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
                "process_rss_last_sampled_bytes", "process_rss_peak_sampled_bytes", "process_cpu_mean_cores", "gc_time_ms", "observed_ticks_per_second"]:
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
