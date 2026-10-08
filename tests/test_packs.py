import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zipfile

from tools import build_pack, compare_benchmarks, runtime


class PackTests(unittest.TestCase):
    def test_rebrand_upgrade_retires_legacy_and_current_names(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            mods = root / "instance/mods"
            mods.mkdir(parents=True)
            previous = [mods / "horizon-lab-0.20.0.jar", mods / "horizon-0.21.0.jar"]
            for old in previous:
                with zipfile.ZipFile(old, "w") as archive:
                    archive.writestr("fabric.mod.json", json.dumps({"id": "horizonlab"}))
            new = root / "horizon-0.22.0.jar"
            new.write_bytes(b"new original fixture")
            runtime.install_original_mod(new, mods)
            self.assertTrue(all(not old.exists() for old in previous))
            self.assertEqual(list(mods.iterdir()), [mods / new.name])
            self.assertEqual(len(list((mods.parent / ".horizon-mod-backups").glob("*.jar"))), 2)

    def test_original_mod_upgrade_preserves_backup_and_other_mods(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            mods = root / "instance/mods"
            mods.mkdir(parents=True)
            old = mods / "horizon-0.2.0.jar"
            with zipfile.ZipFile(old, "w") as archive:
                archive.writestr("fabric.mod.json", json.dumps({"id": "horizonlab"}))
            old_bytes = old.read_bytes()
            other = mods / "other.jar"
            other.write_bytes(b"another mod")
            new = root / "horizon-0.3.0.jar"
            new.write_bytes(b"new original fixture")
            runtime.install_original_mod(new, mods)
            self.assertFalse(old.exists())
            self.assertEqual((mods / new.name).read_bytes(), new.read_bytes())
            self.assertEqual(other.read_bytes(), b"another mod")
            backups = list((mods.parent / ".horizon-mod-backups").glob("*.jar"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_bytes(), old_bytes)

    def test_original_mod_upgrade_refuses_unrelated_named_jar(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with zipfile.ZipFile(root / "horizon-0.1.0.jar", "w") as archive:
                archive.writestr("fabric.mod.json", json.dumps({"id": "someoneelse"}))
            new = root / "horizon-0.3.0.jar"
            new.write_bytes(b"new original fixture")
            with self.assertRaisesRegex(ValueError, "another mod"):
                runtime.install_original_mod(new, root)

    def test_every_profile_has_correct_game_and_exact_dependency_closure(self):
        lock = build_pack.read_lock()
        for profile in build_pack.PROFILES.values():
            build_pack.validate_selection(lock, profile["mods"])

    def test_core_performance_matches_baseline_view_and_simulation(self):
        baseline = build_pack.PROFILES["baseline"]
        candidate = build_pack.PROFILES["core-performance"]
        self.assertEqual(candidate["mods"], build_pack.CORE)
        for key in ["render", "lod", "heap"]:
            self.assertEqual(candidate[key], baseline[key])
        files = build_pack.overrides("core-performance", b"original fixture")
        before = build_pack.overrides("baseline", b"original fixture")
        for name, value in before.items():
            if name != "config/horizon-profile.txt":
                self.assertEqual(files[name], value, name)
        self.assertNotIn("config/DistantHorizons.toml", files)
        self.assertNotIn("config/iris.properties", files)
        self.assertNotIn("distanthorizons", candidate["mods"])
        self.assertNotIn("iris", candidate["mods"])

    def test_missing_exact_iris_sodium_dependency_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Missing pinned dependency"):
            build_pack.validate_selection(build_pack.read_lock(), ["iris"])

    def test_dispatch_experiment_does_not_enable_other_controllers(self):
        for name, profile in build_pack.PROFILES.items():
            files = build_pack.overrides(name, b"original fixture")
            dispatch = files["config/horizon-dispatch-retry.properties"].decode()
            self.assertEqual(dispatch, "enabled=true\n" if name in {"dispatch-retry", "foreground-first", "foreground-vulkan"} else "enabled=false\n")
            if name == "dispatch-retry":
                for setting in ["worker-pacing", "queue-wait"]:
                    self.assertEqual(files[f"config/horizon-{setting}.properties"], b"enabled=false\n")
                self.assertTrue(files["config/horizon-adaptive.properties"].startswith(b"enabled=false\n"))
                self.assertEqual(files["config/DistantHorizons.toml"],
                                 build_pack.overrides("m2-balanced", b"original fixture")["config/DistantHorizons.toml"])

    def test_combined_profiles_enable_all_controls_without_changing_simulation(self):
        for name, backend in [("foreground-first", "opengl"), ("foreground-vulkan", "vulkan")]:
            files = build_pack.overrides(name, b"original fixture")
            self.assertEqual(files["config/horizon-adaptive.properties"], b"enabled=true\ntargetFps=120\n")
            for key in ["worker-pacing", "queue-wait", "dispatch-retry", "tick-dedup"]:
                self.assertEqual(files[f"config/horizon-{key}.properties"], b"enabled=true\n")
            options = dict(line.split(":", 1) for line in files["options.txt"].decode().splitlines())
            self.assertEqual(options["simulationDistance"], "12")
            self.assertEqual(options["renderDistance"], "12")
            self.assertEqual(options["maxFps"], "260")
            self.assertEqual(json.loads(options["preferredGraphicsBackend"]), backend)
            self.assertNotIn("config/iris.properties", files)

    def test_tick_membership_experiment_preserves_visual_and_worker_settings(self):
        original = build_pack.overrides("m2-balanced", b"fixture")
        for name in build_pack.PROFILES:
            files = build_pack.overrides(name, b"fixture")
            self.assertEqual(files["config/horizon-tick-dedup.properties"],
                             b"enabled=true\n" if name in {"tick-dedup", "foreground-first", "foreground-vulkan"} else b"enabled=false\n")
        candidate = build_pack.overrides("tick-dedup", b"fixture")
        for filename in ["options.txt", "config/DistantHorizons.toml", "config/horizon-adaptive.properties",
                         "config/horizon-worker-pacing.properties", "config/horizon-queue-wait.properties",
                         "config/horizon-dispatch-retry.properties"]:
            self.assertEqual(original[filename], candidate[filename])

    def test_empty_culling_experiment_preserves_balanced_settings(self):
        original = build_pack.overrides("m2-balanced", b"fixture")
        for name in build_pack.PROFILES:
            files = build_pack.overrides(name, b"fixture")
            self.assertEqual(files["config/horizon-culling-empty.properties"],
                             b"enabled=true\n" if name == "culling-empty" else b"enabled=false\n")
        candidate = build_pack.overrides("culling-empty", b"fixture")
        for filename in ["options.txt", "config/DistantHorizons.toml", "config/horizon-adaptive.properties",
                         "config/horizon-worker-pacing.properties", "config/horizon-queue-wait.properties",
                         "config/horizon-dispatch-retry.properties", "config/horizon-tick-dedup.properties"]:
            self.assertEqual(original[filename], candidate[filename])

    def test_ray_scratch_experiment_preserves_balanced_settings(self):
        baseline = build_pack.overrides("m2-balanced", b"fixture")
        candidate = build_pack.overrides("culling-rays", b"fixture")
        for name in build_pack.PROFILES:
            self.assertEqual(build_pack.overrides(name, b"fixture")["config/horizon-culling-rays.properties"],
                             b"enabled=true\n" if name == "culling-rays" else b"enabled=false\n")
        for filename in ["options.txt", "config/DistantHorizons.toml", "config/horizon-adaptive.properties",
                         "config/horizon-worker-pacing.properties", "config/horizon-queue-wait.properties",
                         "config/horizon-dispatch-retry.properties", "config/horizon-tick-dedup.properties",
                         "config/horizon-culling-empty.properties", "config/horizon-terrain-read-wait.properties"]:
            self.assertEqual(baseline[filename], candidate[filename])

    def test_position_experiment_preserves_balanced_settings(self):
        baseline = build_pack.overrides("m2-balanced", b"fixture")
        candidate = build_pack.overrides("terrain-position", b"fixture")
        for name in build_pack.PROFILES:
            self.assertEqual(build_pack.overrides(name, b"fixture")["config/horizon-terrain-position.properties"],
                             b"enabled=true\n" if name == "terrain-position" else b"enabled=false\n")
        for filename in ["options.txt", "config/DistantHorizons.toml", "config/horizon-adaptive.properties",
                         "config/horizon-worker-pacing.properties", "config/horizon-queue-wait.properties",
                         "config/horizon-dispatch-retry.properties", "config/horizon-tick-dedup.properties",
                         "config/horizon-culling-empty.properties", "config/horizon-culling-rays.properties", "config/horizon-terrain-read-wait.properties"]:
            self.assertEqual(baseline[filename], candidate[filename])

    def test_quad_reuse_profile_is_isolated_at_equal_visual_quality(self):
        baseline = build_pack.overrides("m2-balanced", b"fixture")
        candidate = build_pack.overrides("quad-reuse", b"fixture")
        for name in build_pack.PROFILES:
            self.assertEqual(build_pack.overrides(name, b"fixture")["config/horizon-quad-reuse.properties"],
                             b"enabled=true\n" if name == "quad-reuse" else b"enabled=false\n")
        for filename, content in baseline.items():
            if filename not in {"config/horizon-profile.txt", "config/horizon-quad-reuse.properties"}:
                self.assertEqual(content, candidate[filename], filename)

    def test_terrain_read_shutdown_fix_only_requested_with_distant_terrain(self):
        for name, profile in build_pack.PROFILES.items():
            files = build_pack.overrides(name, b"fixture")
            self.assertEqual(files["config/horizon-terrain-read-wait.properties"],
                             b"enabled=true\n" if profile["lod"] else b"enabled=false\n")

    def test_wrong_game_or_download_path_is_rejected(self):
        for field, value in [("game_versions", ["1.21"]), ("filename", "../bad.jar"), ("url", "http://unsafe.invalid")]:
            lock = copy.deepcopy(build_pack.read_lock())
            lock["projects"]["sodium"][field] = value
            with self.assertRaises(ValueError):
                build_pack.validate_selection(lock, ["sodium"])

    def test_portable_pack_contains_only_original_embedded_mod_and_download_references(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "packs").mkdir()
            (root / "packs/mods.lock.json").write_text(json.dumps(build_pack.read_lock()))
            (root / "packs/config").mkdir()
            (root / "packs/config/m2-balanced.toml").write_text("[client.advanced.graphics.quality]\nlodChunkRenderDistanceRadius = 128\n")
            original = root / "original.jar"
            original.write_bytes(b"original fixture")
            with mock.patch.object(build_pack, "ROOT", root):
                pack = build_pack.build("m2-balanced", original)
            with zipfile.ZipFile(pack) as archive:
                names = archive.namelist()
                self.assertEqual([n for n in names if n.endswith(".jar")],
                                 [f"overrides/mods/horizon-{build_pack.read_lock()['pack_version']}.jar"])
                index = json.loads(archive.read("modrinth.index.json"))
                self.assertEqual(index["dependencies"]["minecraft"], "26.3")
                options = dict(line.split(":", 1) for line in archive.read("overrides/options.txt").decode().splitlines())
                # A missing/currently wrong version invokes the game's graphics API reset migration.
                self.assertEqual(options["version"], "5023")
                self.assertEqual(json.loads(options["preferredGraphicsBackend"]), "opengl")
                # The default Fancy startup preset otherwise overwrites the requested view distance.
                self.assertEqual(json.loads(options["graphicsPreset"]), "custom")
                for entry in index["files"]:
                    self.assertEqual(set(entry["hashes"]), {"sha1", "sha512"})
                    self.assertTrue(entry["downloads"][0].startswith("https://cdn.modrinth.com/"))

    def test_native_library_rules_choose_only_matching_operating_system(self):
        library = {"rules": [{"action": "allow", "os": {"name": "osx", "arch": "aarch64|arm64"}}]}
        self.assertTrue(runtime.library_allowed(library, "osx", "aarch64"))
        self.assertFalse(runtime.library_allowed(library, "linux", "aarch64"))
        self.assertFalse(runtime.library_allowed(library, "osx", "x86_64"))

    def test_conditional_client_jvm_arguments_follow_official_platform_rules(self):
        windows = "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"
        manifest = {"arguments": {"jvm": [
            {"rules": [{"action": "allow", "os": {"name": "osx"}}], "value": ["-XstartOnFirstThread"]},
            {"rules": [{"action": "allow", "os": {"name": "windows"}}], "value": windows},
            {"rules": [{"action": "allow", "os": {"arch": "x86"}}], "value": "-Xss1M"},
            "-cp", "${classpath}", "--enable-native-access=ALL-UNNAMED",
        ]}}
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "osx", "aarch64"), ["-XstartOnFirstThread"])
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "osx", "x86_64"), ["-XstartOnFirstThread"])
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "windows", "AMD64"), [windows])
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "windows", "arm64"), [windows])
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "linux", "x86_64"), [])
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "windows", "x86"), [windows, "-Xss1M"])
        # Client platform flags must not leak into the shared server JVM policy.
        self.assertEqual(runtime.jvm_options("3G", True, "g1"),
                         ["-Xms512M", "-Xmx3G", "-XX:+UseCompactObjectHeaders", "-XX:+UseG1GC"])

    def test_conditional_client_jvm_arguments_observe_rule_order_and_reject_placeholders(self):
        argument = {"rules": [{"action": "allow", "os": {"name": "osx"}},
                              {"action": "disallow", "os": {"arch": "x86_64"}}], "value": "-XstartOnFirstThread"}
        manifest = {"arguments": {"jvm": [argument]}}
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "osx", "aarch64"), ["-XstartOnFirstThread"])
        self.assertEqual(runtime.conditional_client_jvm_options(manifest, "osx", "x86_64"), [])
        self.assertEqual(runtime.conditional_client_jvm_options({}, "linux", "x86_64"), [])
        argument["value"] = "${unsupported}"
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            runtime.conditional_client_jvm_options(manifest, "osx", "aarch64")

    def test_demo_command_includes_mac_first_thread_flag_before_main_class(self):
        manifest = {"id": "26.3", "assetIndex": {"id": "fixture"}, "arguments": {"jvm": [
            {"rules": [{"action": "allow", "os": {"name": "osx"}}], "value": "-XstartOnFirstThread"}]}}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with mock.patch.object(runtime, "prepare_client", return_value=(manifest, [root / "game.jar"], root / "natives")), \
                 mock.patch.object(runtime.platform, "system", return_value="Darwin"), \
                 mock.patch.object(runtime.subprocess, "run") as run, \
                 mock.patch("sys.argv", ["runtime.py", "demo", "--instance", str(root / "instance")]):
                runtime.main()
            command = run.call_args.args[0]
            self.assertEqual(command.count("-XstartOnFirstThread"), 1)
            self.assertLess(command.index("-XstartOnFirstThread"),
                            command.index("net.fabricmc.loader.impl.launch.knot.KnotClient"))
            self.assertEqual(command.count("-cp"), 1)
            self.assertTrue(run.call_args.kwargs["check"])

    def test_native_architecture_is_selected_even_with_os_only_rules(self):
        intel = {"name": "org.lwjgl:lwjgl:3.4.3:natives-macos"}
        arm = {"name": "org.lwjgl:lwjgl:3.4.3:natives-macos-arm64"}
        self.assertTrue(runtime.native_library_matches(arm, "aarch64"))
        self.assertFalse(runtime.native_library_matches(intel, "aarch64"))
        self.assertTrue(runtime.native_library_matches(intel, "x86_64"))
        self.assertFalse(runtime.native_library_matches(arm, "AMD64"))


class ComparisonTests(unittest.TestCase):
    def report(self, kind="frames"):
        return {"schema": 1, "kind": kind, "elapsed_seconds": 120, "framebuffer_width": 1920,
                "stats": {"count": 1000, "meanRate": 100, "lowOnePercentRate": 50,
                          "meanMs": 10, "p95Ms": 15, "p99Ms": 20, "maxMs": 30}}

    def test_integrated_tick_graphics_and_server_kind(self):
        baseline = self.tick_report()
        baseline.update(server_runtime_kind="integrated", client_graphics_capture_status="captured_at_recording_start",
                        gpu="GPU", graphics_backend="OpenGL", graphics_driver="Driver")
        candidate = copy.deepcopy(baseline)
        candidate["graphics_backend"] = "Vulkan"
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertTrue(any("graphics_backend" in item for item in result["mismatches"]))
        self.assertFalse(any("were not captured" in item for item in compare_benchmarks.server_context_warnings(baseline, candidate)))
        candidate["server_runtime_kind"] = "dedicated"
        self.assertTrue(any("server_runtime_kind" in item for item in compare_benchmarks.compare(baseline, candidate)["mismatches"]))

    def test_legacy_and_missing_integrated_graphics_are_unknown(self):
        legacy = self.tick_report()
        self.assertTrue(any("runtime kind was not captured" in item for item in compare_benchmarks.server_context_warnings(legacy, legacy)))
        legacy.update(server_runtime_kind="integrated", client_graphics_capture_status="captured_at_recording_start", gpu="GPU", graphics_backend="OpenGL")
        self.assertTrue(any("client graphics were not captured" in item for item in compare_benchmarks.server_context_warnings(legacy, legacy)))
        dedicated = {"server_runtime_kind": "dedicated", "client_graphics_capture_status": "not_supplied"}
        self.assertFalse(any("were not captured" in item for item in compare_benchmarks.server_context_warnings(dedicated, dedicated)))

    def test_rate_improvement_and_setting_mismatch(self):
        baseline = self.report()
        candidate = copy.deepcopy(baseline)
        candidate["stats"]["meanRate"] = 150
        candidate["framebuffer_width"] = 1280
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["metrics"][0]["change_percent"], 50)
        self.assertIn("framebuffer_width", result["mismatches"][0])

    def tick_report(self):
        report = self.report("ticks")
        state = dict(configured_tick_rate=20, frozen=False, stepping=False, sprinting=False, runs_game_elements=True)
        report.update(tick_state_capture_status="sampled_server_ticks", tick_state_samples=1000,
                      tick_state_first=state, tick_state_last=copy.deepcopy(state), tick_state_changed=[],
                      observed_ticks_per_second=1000 / 120)
        report.update({"tick_state_" + field + "_samples": 0
                       for field in ["frozen", "stepping", "sprinting", "suppressed", "nonstandard_rate"]})
        return report

    def test_restored_tick_changes_still_disqualify_unchanged_workload(self):
        baseline = self.tick_report()
        candidate = copy.deepcopy(baseline)
        candidate.update(tick_state_changed=["configured_tick_rate", "frozen"],
                         tick_state_frozen_samples=50, tick_state_nonstandard_rate_samples=50)
        self.assertEqual(baseline["tick_state_first"], candidate["tick_state_last"])
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertIn("candidate tick-manager state changed during recording", result["mismatches"])
        self.assertTrue(any("frozen, nonstandard_rate" in warning for warning in result["warnings"]))
        observed = next(row for row in result["metrics"] if row["metric"] == "observed_ticks_per_second")
        self.assertAlmostEqual(observed["baseline"], 8.333333333)
        self.assertTrue(any("execution capacity is not TPS" in warning for warning in result["warnings"]))

    def test_equal_tick_state_ignores_map_order_and_warns_on_custom_rate(self):
        baseline = self.tick_report()
        for boundary in ["tick_state_first", "tick_state_last"]:
            baseline[boundary]["configured_tick_rate"] = 40
        baseline["tick_state_nonstandard_rate_samples"] = 1000
        candidate = copy.deepcopy(baseline)
        candidate["tick_state_first"] = dict(reversed(list(candidate["tick_state_first"].items())))
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["mismatches"], [])
        self.assertTrue(any("nonstandard_rate" in warning for warning in result["warnings"]))
        candidate["tick_state_last"]["frozen"] = True
        self.assertTrue(any("tick_state_last.frozen" in mismatch
                            for mismatch in compare_benchmarks.compare(baseline, candidate)["mismatches"]))

    def test_legacy_and_malformed_tick_state_are_unknown_not_normal(self):
        baseline = self.tick_report()
        for field, bad in [("tick_state_first", None), ("tick_state_samples", True),
                           ("tick_state_samples", -1), ("tick_state_changed", ["invented"]),
                           ("tick_state_frozen_samples", 1001)]:
            candidate = copy.deepcopy(baseline)
            candidate[field] = bad
            self.assertFalse(compare_benchmarks.tick_state_available(candidate))
            result = compare_benchmarks.compare(baseline, candidate)
            self.assertTrue(any("candidate tick-manager state was not captured" in warning for warning in result["warnings"]))
        self.assertFalse(compare_benchmarks.tick_state_available(self.report("ticks")))

    def test_no_samples_and_mixed_kinds_are_rejected(self):
        empty = self.report()
        empty["stats"]["count"] = 0
        for other in [empty, self.report("ticks")]:
            with self.assertRaises(ValueError):
                compare_benchmarks.compare(self.report(), other)

    def test_position_hook_change_is_an_explicit_experiment(self):
        baseline = self.report()
        baseline["terrain_position_arithmetic"] = False
        candidate = copy.deepcopy(baseline)
        candidate["terrain_position_arithmetic"] = True
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["mismatches"], [])
        self.assertTrue(any("terrain_position_arithmetic differs" in warning for warning in result["warnings"]))

    def test_quad_reuse_flag_warns_without_implying_changed_quality(self):
        baseline = self.report()
        baseline["terrain_quad_reuse"] = False
        candidate = copy.deepcopy(baseline)
        candidate["terrain_quad_reuse"] = True
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["mismatches"], [])
        self.assertTrue(any("terrain_quad_reuse differs" in warning for warning in result["warnings"]))

    def test_mod_comparison_detects_upgrades_and_ignores_order(self):
        baseline = self.report()
        baseline.update(mods=["minecraft@26.3", "horizonlab@0.16.0", "sodium@0.9.2"],
                        java_vendor="Eclipse Adoptium", java_vm_name="OpenJDK 64-Bit Server VM",
                        java_runtime_version="25.0.4.1+1")
        candidate = copy.deepcopy(baseline)
        candidate["mods"] = list(reversed(candidate["mods"]))
        self.assertFalse(compare_benchmarks.software_warnings(baseline, candidate))
        candidate["mods"].remove("horizonlab@0.16.0")
        candidate["mods"].extend(["horizonlab@0.17.0", "iris@1.11.7"])
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["mismatches"], [])
        self.assertIn("Only baseline: horizonlab@0.16.0.", result["warnings"])
        self.assertIn("Only candidate: horizonlab@0.17.0, iris@1.11.7.", result["warnings"])
        for field in ["java_vendor", "java_vm_name", "java_runtime_version"]:
            candidate[field] = "Different implementation"
            self.assertTrue(any(f"Software {field} differs" in warning
                                for warning in compare_benchmarks.software_warnings(baseline, candidate)))

    def test_legacy_or_invalid_software_identity_stays_unknown(self):
        baseline = self.report("ticks")
        for entries in [None, "sodium@0.9.2", [3], [""]]:
            candidate = copy.deepcopy(baseline)
            candidate["mods"] = entries
            result = compare_benchmarks.compare(baseline, candidate)
            self.assertTrue(any("Loaded mod versions were not captured" in warning for warning in result["warnings"]))
            self.assertFalse(any("Loaded mod versions differ." in warning for warning in result["warnings"]))

    def test_excluded_frames_disclose_that_process_counters_cover_inactive_time(self):
        baseline = self.report()
        candidate = copy.deepcopy(baseline)
        candidate["excluded_frames"] = 28
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["mismatches"], [])
        self.assertTrue(any("candidate excluded 28" in warning and "Process CPU still spans" in warning
                            for warning in result["warnings"]))
        self.assertFalse(any("baseline excluded" in warning for warning in result["warnings"]))

    def test_jvm_experiment_warns_without_rejecting_an_intentional_change(self):
        baseline = self.report()
        baseline.update(compact_object_headers=False, garbage_collectors=["G1 Young Generation"])
        candidate = copy.deepcopy(baseline)
        candidate["compact_object_headers"] = True
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["mismatches"], [])
        self.assertTrue(any("compact_object_headers differs" in warning for warning in result["warnings"]))
        del candidate["compact_object_headers"]
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertTrue(any("not captured" in warning for warning in result["warnings"]))

    def test_terrain_quality_and_shader_changes_cannot_look_like_equal_workloads(self):
        baseline = self.report()
        baseline.update(dh_capture_status="available", dh_lod_radius_chunks=128, dh_horizontal_quality="MEDIUM",
                        iris_capture_status="available", iris_shaders_enabled=False, dh_thread_count=2, world_difficulty="NORMAL")
        candidate = copy.deepcopy(baseline)
        candidate.update(dh_lod_radius_chunks=256, iris_shaders_enabled=True, iris_shader_pack="Example pack",
                         dh_thread_count=1, world_difficulty="PEACEFUL", visual_settings_changed=["render_distance"])
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertTrue(any("dh_lod_radius_chunks" in field for field in result["mismatches"]))
        self.assertTrue(any("iris_shaders_enabled" in field for field in result["mismatches"]))
        self.assertTrue(any("world_difficulty" in field for field in result["mismatches"]))
        self.assertTrue(any("dh_thread_count differs" in warning for warning in result["warnings"]))
        self.assertTrue(any("changed between capture boundaries" in warning for warning in result["warnings"]))
        legacy = self.report()
        result = compare_benchmarks.compare(legacy, copy.deepcopy(legacy))
        self.assertTrue(any("terrain configuration was not captured" in warning for warning in result["warnings"]))


if __name__ == "__main__":
    unittest.main()
