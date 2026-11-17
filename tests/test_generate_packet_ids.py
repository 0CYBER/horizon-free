import unittest

from tools import generate_packet_ids


class GeneratePacketIdsTests(unittest.TestCase):
    def test_unqualified_packet_id_names_itself(self):
        with self.assertRaisesRegex(ValueError, "unqualified packet id: unqualified_name"):
            generate_packet_ids.const_name("unqualified_name")

    def test_qualified_packet_id_becomes_a_const_name(self):
        self.assertEqual(generate_packet_ids.const_name("minecraft:clientbound/status/ping"), "CLIENTBOUND_STATUS_PING")


if __name__ == "__main__":
    unittest.main()
