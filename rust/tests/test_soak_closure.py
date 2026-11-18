"""A timed-out open peer must never pass a rejection/health-check assertion."""
import socket
import unittest
from unittest.mock import Mock, patch

from soak_26_3 import _drain, recv_until_closed, fuzz_round, Stats


class PeerCloseTests(unittest.TestCase):
    def test_fuzz_timeout_is_not_a_peer_close(self):
        sock = Mock()
        sock.recv.side_effect = socket.timeout()
        self.assertEqual(_drain(sock), (0, False))

    def test_http_and_partial_frame_bytes_can_be_drained_before_eof(self):
        sock = Mock()
        parts = [b"HTTP/1.1 101\r\n\r\n", b"\x82", b"\x01x", b""]
        sock.recv.side_effect = parts
        self.assertEqual(_drain(sock), (sum(map(len, parts)), True))

    def test_peer_reset_is_a_close(self):
        sock = Mock()
        sock.recv.side_effect = ConnectionResetError()
        self.assertEqual(_drain(sock), (0, True))

    def test_motd_timeout_does_not_count_as_closed(self):
        client = Mock()
        client.receive_frame.side_effect = [(1, b"{}"), socket.timeout()]
        self.assertEqual(recv_until_closed(client), (1, 0, False))

    def test_motd_eof_counts_as_closed(self):
        client = Mock()
        client.receive_frame.side_effect = [(1, b"{}"), EOFError()]
        self.assertEqual(recv_until_closed(client), (1, 0, True))

    def test_connection_failure_is_not_a_successful_fuzz_rejection(self):
        stats = Stats()
        with patch("soak_26_3.socket.create_connection", side_effect=ConnectionRefusedError()):
            fuzz_round("127.0.0.1", 8081, stats, choice=0)
        counters, errors = stats.snapshot()
        self.assertEqual(len(errors), 1)
        self.assertNotIn("fuzz_rounds", counters)


if __name__ == "__main__":
    unittest.main()
