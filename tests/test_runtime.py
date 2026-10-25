"""Platform JVM rules used by the Java comparison benchmarks."""

import unittest

from tools import runtime


class RuntimeTests(unittest.TestCase):
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

    def test_native_architecture_is_selected_even_with_os_only_rules(self):
        intel = {"name": "org.lwjgl:lwjgl:3.4.3:natives-macos"}
        arm = {"name": "org.lwjgl:lwjgl:3.4.3:natives-macos-arm64"}
        self.assertTrue(runtime.native_library_matches(arm, "aarch64"))
        self.assertFalse(runtime.native_library_matches(intel, "aarch64"))
        self.assertTrue(runtime.native_library_matches(intel, "x86_64"))
        self.assertFalse(runtime.native_library_matches(arm, "AMD64"))
