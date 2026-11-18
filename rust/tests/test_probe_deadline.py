"""Probe deadlines must finish runs without hiding unexpected failures."""
import socket
import unittest
from unittest.mock import Mock, patch

from probe_26_3 import receive_before


class ReceiveDeadlineTests(unittest.TestCase):
    def test_idle_at_deadline_ends_the_run(self):
        client = Mock()
        client.receive.side_effect = socket.timeout()
        with patch("probe_26_3.time.monotonic", side_effect=[90.0, 100.0]):
            self.assertIsNone(receive_before(client, 100.0))
        client.sock.settimeout.assert_called_once_with(10.0)

    def test_early_timeout_is_not_a_successful_deadline(self):
        client = Mock()
        client.receive.side_effect = socket.timeout()
        with patch("probe_26_3.time.monotonic", side_effect=[90.0, 95.0]):
            with self.assertRaises(socket.timeout):
                receive_before(client, 100.0)

    def test_peer_disconnect_during_run_remains_a_failure(self):
        client = Mock()
        client.receive.side_effect = EOFError("closed")
        with patch("probe_26_3.time.monotonic", return_value=90.0):
            with self.assertRaises(EOFError):
                receive_before(client, 100.0)

    def test_expired_deadline_does_not_read(self):
        client = Mock()
        with patch("probe_26_3.time.monotonic", return_value=100.0):
            self.assertIsNone(receive_before(client, 100.0))
        client.receive.assert_not_called()


if __name__ == "__main__":
    unittest.main()
