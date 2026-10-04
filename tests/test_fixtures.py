import json
import subprocess
import unittest

from tools.test_dispatch import failure_summary
from tools.test_client_graphics import graphics_failure_summary


class FixtureDiagnosticTests(unittest.TestCase):
    def test_graphics_diagnostics_separate_demo_auth_and_exclude_methods_messages(self):
        output = '\n'.join([
            'com.mojang.authlib.exceptions.InvalidCredentialsException: private-token',
            ' at com.mojang.authlib.minecraft.client.MinecraftClient.buildHttpException(File.java:1)',
            'java.lang.AssertionError: /private/world/path',
            'Caused by: java.lang.RuntimeException: private-message VK_ERROR_FEATURE_NOT_PRESENT',
            'HORIZON_CLIENT_GRAPHICS_INITIALIZER_READY',
            'HORIZON_CLIENT_GRAPHICS_READBACK_REQUESTED',
        ])
        result = graphics_failure_summary(subprocess.CompletedProcess([], 1, output), 0)
        self.assertEqual(result['java_exception_types'], ['java.lang.AssertionError', 'java.lang.RuntimeException'])
        self.assertTrue(result['expected_demo_authentication_errors_observed'])
        self.assertTrue(result['client_initializer_reached'])
        self.assertTrue(result['readback_requested'])
        self.assertEqual(result['vulkan_error_codes'], ['VK_ERROR_FEATURE_NOT_PRESENT'])
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('buildHttpException', json.dumps(result))

    def test_only_exit_count_and_exception_types_leave_private_log(self):
        output = '\n'.join([
            'Exception in thread "private-thread-name" java.lang.NoClassDefFoundError: private-token',
            'Caused by: java.lang.ClassNotFoundException: /private/world/path',
            'Caused by: java.lang.ClassNotFoundException: other-private-message',
            '    at private.World.method(World.java:42)',
            'session-token=private-token',
        ])
        run = subprocess.CompletedProcess(['private-command'], 1, output)
        report = failure_summary(run, 0)
        self.assertEqual(report, {'exit_code': 1, 'result_count': 0,
            'java_exception_types': ['java.lang.NoClassDefFoundError', 'java.lang.ClassNotFoundException']})
        self.assertNotIn('private', json.dumps(report))

    def test_graphics_stack_locations_are_selected_and_bounded(self):
        frames = ['    at knot//com.mojang.renderpearl.backend.vulkan.Buffer.method%d(PrivateFile.java:42)' % index
                  for index in range(12)]
        output = '\n'.join([
            'message: at com.mojang.blaze3d.Bad.message(Private.java:1)',
            '    at com.mojang.authlib.Auth.privateToken(PrivateFile.java:2)',
            '    at net.minecraft.client.Screenshot.takeScreenshot(/private/file.java:3)',
            *frames, *frames,
        ])
        result = graphics_failure_summary(subprocess.CompletedProcess([], 1, output), 0)
        methods = result['observed_graphics_stack_methods']
        self.assertEqual(len(methods), 8)
        self.assertEqual(methods[0], 'net.minecraft.client.Screenshot.takeScreenshot')
        self.assertEqual(methods[1], 'com.mojang.renderpearl.backend.vulkan.Buffer.method0')
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('Private', json.dumps(result))
        self.assertNotIn('Bad', json.dumps(result))

    def test_owned_probe_failure_is_separate_from_logged_vulkan_codes(self):
        output = '\n'.join([
            'logged separately: VK_ERROR_FEATURE_NOT_PRESENT private-message',
            'java.lang.AssertionError: Uniform title readback',
            'HORIZON_CLIENT_GRAPHICS_READBACK_RECEIVED',
            'HORIZON_CLIENT_GRAPHICS_READBACK_INSPECTED {"framebuffer_width":1280,"framebuffer_height":646,"sampled_pixels":4160,"distinct_sampled_rgb":1,"ignored":"private-token"}',
            'HORIZON_CLIENT_GRAPHICS_READBACK_INSPECTED {"framebuffer_width":"private-path"}',
            'HORIZON_CLIENT_GRAPHICS_READBACK_INSPECTED private-malformed',
        ])
        result = graphics_failure_summary(subprocess.CompletedProcess([], 1, output), 0)
        self.assertEqual(result['probe_failure_categories'], ['uniform_readback'])
        self.assertTrue(result['readback_callback_received'])
        self.assertEqual(result['readback_inspections'][0]['distinct_sampled_rgb'], 1)
        self.assertEqual(len(result['readback_inspections']), 1)
        self.assertEqual(result['vulkan_error_codes'], ['VK_ERROR_FEATURE_NOT_PRESENT'])
        self.assertNotIn('private', json.dumps(result))

    def test_type_list_and_names_are_bounded(self):
        output = '\n'.join('Caused by: sample.Failure%d: private-message' % index for index in range(20))
        output = 'Caused by: sample.' + 'A' * 201 + ': private-message\n' + output
        report = failure_summary(subprocess.CompletedProcess([], 2, output), 3)
        self.assertEqual(len(report['java_exception_types']), 8)
        self.assertEqual(report['java_exception_types'][0], 'sample.Failure0')
        self.assertEqual(report['result_count'], 3)

    def test_unknown_failure_still_reports_exit_and_count(self):
        report = failure_summary(subprocess.CompletedProcess([], -15, 'private unrelated message'), 0)
        self.assertEqual(report, {'exit_code': -15, 'result_count': 0, 'java_exception_types': []})
