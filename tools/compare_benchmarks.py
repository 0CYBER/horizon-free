#!/usr/bin/env python3
"""Compare two real Horizon JSON reports and flag mismatched workloads."""

import argparse
import json
import math
from pathlib import Path

MATCH_FIELDS = ("minecraft", "kind", "os", "os_version", "architecture", "gpu", "graphics_backend", "graphics_driver",
                "server_runtime_kind", "client_graphics_capture_status", "java_version", "heap_max_bytes", "framebuffer_width", "framebuffer_height", "render_distance",
                "simulation_distance", "vsync", "fps_limit", "graphics_preset", "ambient_occlusion",
                "improved_transparency", "mipmap_levels", "entity_distance_scaling", "entity_shadows", "particles",
                "clouds", "cloud_range", "fov", "world_difficulty", "dh_capture_status", "dh_rendering_enabled", "dh_renderer_mode",
                "dh_lod_radius_chunks", "dh_horizontal_quality", "dh_vertical_quality", "dh_max_horizontal_resolution",
                "dh_transparency", "dh_lod_shading", "dh_lod_only_mode", "dh_camera_quality_dropoff",
                "dh_ambient_occlusion", "iris_capture_status", "iris_shaders_enabled", "iris_shader_pack")

TICK_STATE_FIELDS = ("configured_tick_rate", "frozen", "stepping", "sprinting", "runs_game_elements")


def tick_state_available(report):
    def number(value):
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            return False
        try:
            return math.isfinite(value)
        except OverflowError:
            return False
    if report.get("tick_state_capture_status") != "sampled_server_ticks":
        return False
    count = report.get("tick_state_samples")
    if not number(count) or count <= 0 or count > 9_007_199_254_740_991 or count != int(count):
        return False
    for boundary in ["tick_state_first", "tick_state_last"]:
        state = report.get(boundary)
        if not isinstance(state, dict) or not number(state.get("configured_tick_rate")) or state["configured_tick_rate"] < 1:
            return False
        if any(not isinstance(state.get(field), bool) for field in TICK_STATE_FIELDS[1:]):
            return False
    changed = report.get("tick_state_changed")
    if not isinstance(changed, list) or any(field not in TICK_STATE_FIELDS for field in changed):
        return False
    return all(number(report.get("tick_state_" + field + "_samples"))
               and 0 <= report["tick_state_" + field + "_samples"] <= count
               and report["tick_state_" + field + "_samples"] == int(report["tick_state_" + field + "_samples"])
               for field in ["frozen", "stepping", "sprinting", "suppressed", "nonstandard_rate"])


def compare_tick_states(baseline, candidate):
    mismatches, warnings = [], []
    available = [tick_state_available(report) for report in [baseline, candidate]]
    for name, report, captured in zip(["baseline", "candidate"], [baseline, candidate], available):
        if not captured:
            warnings.append(f"{name} tick-manager state was not captured or is invalid; simulation workload equivalence is unknown.")
            continue
        if report["tick_state_changed"]:
            mismatches.append(f"{name} tick-manager state changed during recording")
            warnings.append(f"{name} sampled tick-manager changes: " + ", ".join(report["tick_state_changed"]) + ".")
        abnormal = [field for field in ["frozen", "stepping", "sprinting", "suppressed", "nonstandard_rate"]
                    if report["tick_state_" + field + "_samples"] > 0]
        if abnormal:
            warnings.append(f"{name} includes tick-manager samples with " + ", ".join(abnormal) + "; interpret execution times as this workload.")
    if all(available):
        for boundary in ["tick_state_first", "tick_state_last"]:
            for field in TICK_STATE_FIELDS:
                if baseline[boundary][field] != candidate[boundary][field]:
                    mismatches.append(f"{boundary}.{field}: {baseline[boundary][field]!r} vs {candidate[boundary][field]!r}")
    warnings.append("Tick-manager state is sampled after measured server invocations. Changes entirely between samples, paused intervals and completed world work are not verified.")
    return mismatches, warnings


def software_warnings(baseline, candidate):
    warnings = []
    for field in ["java_vendor", "java_vm_name", "java_runtime_version"]:
        if not all(isinstance(report.get(field), str) and report[field] for report in [baseline, candidate]):
            warnings.append(f"{field} was not captured in both reports; Java implementation equivalence is unknown.")
        elif baseline[field] != candidate[field]:
            warnings.append(f"Software {field} differs: {baseline[field]!r} vs {candidate[field]!r}. Treat it as an explicit experiment variable.")
    mods = []
    for report in [baseline, candidate]:
        entries = report.get("mods")
        mods.append(set(entries) if isinstance(entries, list) and all(isinstance(entry, str) and entry for entry in entries) else None)
    if any(entries is None for entries in mods):
        warnings.append("Loaded mod versions were not captured in both reports; software equivalence is unknown.")
    elif mods[0] != mods[1]:
        warnings.append("Loaded mod versions differ. Treat added, removed or upgraded mods as explicit experiment variables.")
        for label, entries in [("baseline", mods[0] - mods[1]), ("candidate", mods[1] - mods[0])]:
            ordered = sorted(entries)
            if ordered:
                suffix = f"; {len(ordered) - 12} more" if len(ordered) > 12 else ""
                warnings.append(f"Only {label}: " + ", ".join(ordered[:12]) + suffix + ".")
    return warnings


def server_context_warnings(baseline, candidate):
    warnings = []
    for name, report in [("baseline", baseline), ("candidate", candidate)]:
        kind = report.get("server_runtime_kind")
        if kind not in ["integrated", "dedicated"]:
            warnings.append(f"{name} server runtime kind was not captured; integrated/dedicated equivalence is unknown.")
        if kind == "integrated":
            captured = report.get("client_graphics_capture_status") == "captured_at_recording_start" and all(
                isinstance(report.get(key), str) and report[key].strip() for key in ["gpu", "graphics_backend", "graphics_driver"])
            if not captured:
                warnings.append(f"{name} integrated-server client graphics were not captured; shared rendering workload is unknown.")
    warnings.append("Integrated-server graphics context is a recording-start snapshot; later graphics changes and full client workload equivalence are not verified.")
    return warnings


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
    warnings.extend(software_warnings(baseline, candidate))
    if baseline["kind"] == "ticks":
        warnings.extend(server_context_warnings(baseline, candidate))
        tick_mismatches, tick_warnings = compare_tick_states(baseline, candidate)
        mismatches.extend(tick_mismatches)
        warnings.extend(tick_warnings)
        warnings.append("Tick durations exclude scheduler sleep. Observed TPS counts measured invocations, including frozen ticks, and includes recording time spent paused; tick execution capacity is not TPS.")
    if baseline["kind"] == "frames":
        for field in ["adaptive_budget_enabled", "adaptive_target_fps", "precise_worker_pacing", "debounce_queue_wait",
                      "wakeable_idle_wait", "dispatch_retry_backoff", "tick_membership_dedup", "empty_culling_whitelist", "culling_ray_scratch", "terrain_position_arithmetic", "terrain_quad_reuse", "interruptible_terrain_read", "dh_thread_count", "dh_runtime_ratio"]:
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
        if report["kind"] == "frames" and report.get("excluded_frames", 0):
            warnings.append(f"{name} excluded {report['excluded_frames']} menu, paused or unfocused frame starts. Process CPU still spans the whole recording.")
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
