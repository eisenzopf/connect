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

    def test_original_commit_diagnostic_profile_is_forwarded_without_account_environment(self):
        self.owner.created = True
        base = "info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug"
        selected = base + ",connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,rvoip_core::audio_summary=debug"
        command = self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=selected))
        self.assertIn("--setenv=RUST_LOG=" + selected, command)
        self.assertFalse(any("secret" in value or "API_KEY" in value or "VAPI" in value for value in command))
        self.assertNotIn("CONNECT_SDK_HELPER_TIMING", " ".join(command))

    def test_commit_diagnostics_cannot_enable_unbounded_packet_or_sql_tracing(self):
        self.owner.created = True
        base = "info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug"
        commit = base + ",connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,rvoip_core::audio_summary=debug"
        for suffix in [",connect_service::canonical_packet=trace", ",connect_store::sql_custody=trace", ",private-token=trace"]:
            with self.subTest(suffix=suffix), self.assertRaises(AssertionError):
                self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=commit + suffix))

    def test_shared_retention_events_preserve_each_bounded_server_profile(self):
        self.owner.created = True
        base = "info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug"
        retention = ",connect_service::runtime_retention=debug"
        for suffix in ["", ",connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,rvoip_core::audio_summary=debug",
                       ",connect_service::canonical_replacement=debug,connect_service::replacement_phase_trace=debug,connect_store::authority_lifetime=debug"]:
            selected = base + retention + suffix
            with self.subTest(profile=suffix):
                command = self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=selected))
                self.assertIn("--setenv=RUST_LOG=" + selected, command)
                self.assertIn("--property=NoNewPrivileges=yes", command)
                self.assertIn("--property=RuntimeMaxSec=600", command)
                self.assertFalse(any("secret" in value or "API_KEY" in value for value in command))
        for suffix in [retention, ",connect_service::runtime_retention=trace", ",connect_service::canonical_packet=trace",
                       ",connect_store::sql_custody=trace", ",private-token=trace", ",trace"]:
            with self.subTest(refused=suffix), self.assertRaises(AssertionError):
                self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=base + retention + suffix))

    def test_full_native_bounded_authority_summary_is_forwarded_with_original_provenance(self):
        self.owner.created = True
        base = "info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug"
        commit = ",connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,rvoip_core::audio_summary=debug"
        summary = ",connect_store::authority_summary=debug,connect_service::peer_authority_lifetime=debug"
        for retention in ["", ",connect_service::runtime_retention=debug"]:
            selected = base + retention + commit + summary
            with self.subTest(retention=retention):
                command = self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=selected))
                self.assertIn("--setenv=RUST_LOG=" + selected, command)
                self.assertIn("--property=RuntimeMaxSec=600", command)
                self.assertIn("--property=NoNewPrivileges=yes", command)
                self.assertIn("--uid=" + str(os.getuid()), command)
                self.assertFalse(any("secret" in value or "API_KEY" in value for value in command))

    def test_bounded_authority_summary_does_not_allow_arbitrary_or_duplicate_trace_directives(self):
        self.owner.created = True
        base = "info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug"
        commit = ",connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug,rvoip_core::audio_summary=debug"
        summary = ",connect_store::authority_summary=debug,connect_service::peer_authority_lifetime=debug"
        for extra in [summary, ",trace", ",connect_store::sql_custody=trace", ",connect_service::canonical_packet=trace",
                      ",private-token=trace", ",connect_store::authority_summary=trace"]:
            with self.subTest(extra=extra), self.assertRaises(AssertionError):
                self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=base + commit + summary + extra))
        for selected in [base + summary, base + commit + ",connect_store::authority_summary=debug"]:
            with self.subTest(incomplete=selected), self.assertRaises(AssertionError):
                self.owner.server_command("/owned/connect", "/private/server.toml", dict(self.env, RUST_LOG=selected))

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





    def test_three_real_cycles_scale_only_original_namespace_lifetime(self):
        self.owner.created = True
        normal = self.owner.server_command("/owned/connect", "/private/server.toml", self.env)
        self.assertIn("--property=RuntimeMaxSec=600", normal)
        owner = NativeNamespace(self.root / ".local/three", self.root, self.env, resource_cycles=3)
        owner.created = True
        three = owner.server_command("/owned/connect", "/private/server.toml", self.env)
        self.assertIn("--property=RuntimeMaxSec=1800", three)
        self.assertEqual([x for x in normal if not x.startswith("--property=RuntimeMaxSec=") and not x.startswith("--unit=")],
                         [x for x in three if not x.startswith("--property=RuntimeMaxSec=") and not x.startswith("--unit=")])
        for invalid in (0, 2, 4, True, "3"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                NativeNamespace(self.root / ".local/refused", self.root, self.env, resource_cycles=invalid)


if __name__ == "__main__":
    unittest.main()
