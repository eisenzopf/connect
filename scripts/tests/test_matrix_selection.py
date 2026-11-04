import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_matrix_execution import browser_selection_verified


class MatrixSelectionTests(unittest.TestCase):
    def test_requires_real_failed_paths_original_reads_and_joined_owners(self):
        fixture = dict(selection=dict(format="connect/browser-matrix-selection/1", passed=True,
            selected_transport="wss-webrtc", fallback_reason="preferred-transport-timeout",
            refusal=dict(kind="transport-uncertain", certainty="not-submitted"),
            attempts=[dict(phase=phase, transport=transport) for phase in ("fallback", "neither")
                for transport in ("webtransport", "wss-webrtc")],
            original_operation_id="original", original_reference_recovered=2, application_mutations_submitted=0,
            commands=[dict(phase=phase, type="operation.get", operation_id="original")
                for phase in ("fallback", "recovery-after-refusal")],
            original_physical_owners=4, original_physical_owners_joined=True, cleanup_elapsed_ms=10),
            unavailable_transports=dict(original_sockets_joined=True, udp_datagrams=4, tcp_connections=1, errors=0, overflow=False))
        self.assertTrue(browser_selection_verified(fixture))
        for field, value in [("fallback_reason", "preferred-not-configured"), ("original_reference_recovered", 1),
                ("original_physical_owners_joined", False), ("application_mutations_submitted", 1),
                ("refusal", dict(kind="authentication", certainty="not-submitted"))]:
            altered=copy.deepcopy(fixture);altered["selection"][field]=value
            self.assertFalse(browser_selection_verified(altered), field)
        for field, value in [("udp_datagrams", 0), ("tcp_connections", 0), ("original_sockets_joined", False)]:
            altered=copy.deepcopy(fixture);altered["unavailable_transports"][field]=value
            self.assertFalse(browser_selection_verified(altered), field)
        altered=copy.deepcopy(fixture);altered["selection"]["commands"][1]["operation_id"]="replacement"
        self.assertFalse(browser_selection_verified(altered))
        altered=copy.deepcopy(fixture);altered["selection"]["commands"][1]["type"]="conversation.create"
        self.assertFalse(browser_selection_verified(altered))


if __name__ == "__main__":
    unittest.main()
