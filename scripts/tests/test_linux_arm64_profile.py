"""A local ARM qualification must never adopt an x86/emulated result."""
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.linux_arm64_profile import native_host


class NativeArmProfile(unittest.TestCase):
    def check(self, *, machine="aarch64", system="Linux", cpu="CPU architecture: 8\n", rust="rustc 1.97.1 (fixture)\nhost: aarch64-unknown-linux-gnu\n", version="24.04"):
        with patch("lib.linux_arm64_profile.platform.system", return_value=system), \
                patch("lib.linux_arm64_profile.platform.machine", return_value=machine), \
                patch("lib.linux_arm64_profile.platform.release", return_value="fixture-kernel"), \
                patch("lib.linux_arm64_profile.platform.freedesktop_os_release", return_value={"ID":"ubuntu", "VERSION_ID":version, "PRETTY_NAME":"Ubuntu fixture"}), \
                patch("lib.linux_arm64_profile.Path.read_text", return_value=cpu), \
                patch("lib.linux_arm64_profile.subprocess.check_output", return_value=rust):
            return native_host()

    def test_native_arm_kernel_toolchain_and_distribution_are_reported(self):
        host=self.check()
        self.assertEqual(host["architecture"], "aarch64")
        self.assertEqual(host["execution_scope"], "owner-local-ARM-VM")

    def test_foreign_architecture_kernel_toolchain_and_distribution_refuse(self):
        for change in [dict(machine="x86_64"), dict(system="Darwin"), dict(cpu="vendor_id: fixture\n"),
                dict(rust="rustc 1.97.1 (fixture)\nhost: x86_64-unknown-linux-gnu\n"), dict(version="22.04")]:
            with self.subTest(change=change), self.assertRaises(AssertionError): self.check(**change)


if __name__ == "__main__": unittest.main()
