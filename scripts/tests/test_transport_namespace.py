"""Privilege/account isolation guards; no root or network mutation is executed."""
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_namespace import NativeNamespace, owned_vm_guard
from lib.transport_fixture import FixtureError


class NamespaceBoundary(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.env = dict(PATH="/safe/bin", TELNYX_API_KEY="synthetic-provider-secret",
            VAPI_PRIVATE_KEY="synthetic-agent-secret", OPENAI_API_KEY="synthetic-model-secret")
        self.owner = NativeNamespace(self.root / ".local/network", self.root, self.env)

    def test_only_explicit_owned_arm_vm_can_start(self):
        with self.assertRaises(FixtureError):
            owned_vm_guard({"GITHUB_ACTIONS": "true"})
        with patch("lib.transport_namespace.platform.system", return_value="Darwin"), self.assertRaises(FixtureError):
            owned_vm_guard({"CONNECT_GATE6_OWNED_LINUX_VM": "connect-gate6"})
        with patch("lib.transport_namespace.platform.system", return_value="Linux"), \
                patch("lib.transport_namespace.platform.machine", return_value="aarch64"), \
                patch("lib.transport_namespace.platform.freedesktop_os_release", return_value={"ID":"ubuntu","VERSION_ID":"24.04"}), \
                patch("lib.transport_namespace.os.getuid", return_value=1001), \
                patch("lib.transport_namespace.Path.is_dir", return_value=True):
            owned_vm_guard({"CONNECT_GATE6_OWNED_LINUX_VM": "connect-gate6"})
            with patch("lib.transport_namespace.platform.machine", return_value="x86_64"), self.assertRaises(FixtureError):
                owned_vm_guard({"CONNECT_GATE6_OWNED_LINUX_VM": "connect-gate6"})
            with patch("lib.transport_namespace.os.getuid", return_value=0), self.assertRaises(FixtureError):
                owned_vm_guard({"CONNECT_GATE6_OWNED_LINUX_VM": "connect-gate6"})

    def test_client_drops_root_groups_and_capabilities_before_private_configuration(self):
        self.owner.created = True
        command = self.owner.client_command(["/owned/connect", "request", "--config", "/private/participant.toml"])
        self.assertIn("--reuid=" + str(os.getuid()), command)
        self.assertIn("--clear-groups", command)
        self.assertIn("--no-new-privs", command)
        self.assertLess(command.index("setpriv"), command.index("/owned/connect"))
        self.assertEqual(command[command.index("env") + 1], "-i")
        self.assertIn("PATH=/safe/bin", command)
        self.assertFalse(any("secret" in value or "API_KEY" in value or "VAPI" in value for value in command))

    def test_server_has_ordinary_user_bounded_lifetime_and_no_account_environment(self):
        self.owner.created = True
        command = self.owner.server_command("/owned/connect", "/private/server.toml", self.env)
        self.assertIn("--uid=" + str(os.getuid()), command)
        self.assertIn("--property=RuntimeMaxSec=600", command)
        self.assertIn("--property=TimeoutStopSec=10s", command)
        self.assertIn("--property=NoNewPrivileges=yes", command)
        self.assertFalse(any("secret" in value or "API_KEY" in value or "VAPI" in value for value in command))

    def test_names_are_fresh_bounded_kernel_identifiers(self):
        second = NativeNamespace(self.root / ".local/another", self.root, {})
        self.assertNotEqual(self.owner.name, second.name)
        for name in (self.owner.interface, self.owner.peer):
            self.assertLessEqual(len(name), 15)
            self.assertRegex(name, r"^g6t[hc][0-9a-f]{10}$")

    def test_unstarted_cleanup_does_not_claim_resource_or_server_verification(self):
        result = self.owner.close()
        self.assertFalse(result["passed"])
        self.assertEqual(result["systemd_servers_verified"], 0)
        self.assertIsNone(result["remaining_client_processes"])
        self.assertIs(self.owner.close(), result)


if __name__ == "__main__":
    unittest.main()
