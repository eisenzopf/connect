"""Local native Ubuntu ARM qualification; reject foreign userland/emulation."""
from pathlib import Path
import platform
import re
import subprocess


def native_host():
    assert platform.system() == "Linux" and platform.machine() == "aarch64", "Native ARM64 Linux required"
    assert re.search(r"^CPU architecture\s*:\s*8\s*$", Path("/proc/cpuinfo").read_text(), re.M), "ARM kernel required"
    release = platform.freedesktop_os_release()
    assert release.get("ID") == "ubuntu" and release.get("VERSION_ID") == "24.04", "Ubuntu24.04 required"
    rust = subprocess.check_output(["rustc", "-Vv"], text=True)
    assert "rustc 1.97.1 " in rust and "host: aarch64-unknown-linux-gnu" in rust, "Native ARM Rust toolchain required"
    return dict(os="linux", architecture="aarch64", kernel_release=platform.release(),
        distribution=release["PRETTY_NAME"], rust=rust, execution_scope="owner-local-ARM-VM")
