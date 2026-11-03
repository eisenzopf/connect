"""Owned disposable Ubuntu network namespace and ordinary-user systemd server.

Only the explicitly selected owned local ARM64 VM may create these resources.
No default route, external host, provider or account configuration is installed.
"""
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import uuid

from .transport_actors import exact
from .transport_fixture import FixtureError
from .transport_matrix import digest
from .package_environment import fixture_environment


def owned_vm_guard(environ=None):
    environ = os.environ if environ is None else environ
    if (environ.get("CONNECT_GATE6_OWNED_LINUX_VM") != "connect-gate6" or platform.system() != "Linux"
            or platform.machine() != "aarch64" or os.getuid() == 0
            or not Path("/run/systemd/system").is_dir()):
        raise FixtureError("fixture-disposable-ubuntu-systemd-required")
    release = platform.freedesktop_os_release()
    if release.get("ID") != "ubuntu" or release.get("VERSION_ID") != "24.04":
        raise FixtureError("fixture-disposable-ubuntu-systemd-required")


class NativeNamespace:
    server_ip = "10.203.0.1"
    client_ip = "10.203.0.2"
    prefix = "10.203.0.0/30"

    def __init__(self, directory, root, env):
        self.directory, self.root, self.env = Path(directory), Path(root), fixture_environment(env)
        token = uuid.uuid4().hex[:10]
        self.name, self.interface, self.peer = "g6t-" + token, "g6th" + token, "g6tc" + token
        self.unit = "connect-g658-" + token + ".service"
        self.created = self.link_created = False
        self.active_pid = None
        self.owner = None
        self.closed = None
        self.sequence = 0
        self.servers_verified = 0
        self.client_forced = False

    def tool(self, label, args, *, success=True):
        self.sequence += 1
        path = self.directory / (str(self.sequence) + "-" + label)
        with path.with_suffix(".stdout").open("xb") as out, path.with_suffix(".stderr").open("xb") as err:
            child = subprocess.Popen(args, env=self.env, cwd=self.root, stdout=out, stderr=err,
                stdin=subprocess.DEVNULL, start_new_session=True)
            try:
                code = child.wait(timeout=20)
            except BaseException:
                # The sudo monitor and its command belong to this exact group.
                subprocess.run(["sudo", "-n", "/bin/kill", "-KILL", "--", "-" + str(child.pid)],
                    env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL, timeout=5, check=False)
                child.wait(timeout=5)
                raise
        if success and code != 0:
            raise FixtureError("fixture-network-tool-refused")
        if path.with_suffix(".stdout").stat().st_size > 2 * 1024 * 1024:
            raise FixtureError("fixture-network-tool-output-bound")
        return code, path.with_suffix(".stdout").read_text()

    def sudo(self, label, args, **kwargs):
        return self.tool(label, ["sudo", "-n", *args], **kwargs)

    def start(self):
        owned_vm_guard()
        if not self.directory.resolve().is_relative_to((self.root / ".local").resolve()):
            raise FixtureError("fixture-network-private-path")
        self.directory.mkdir(mode=0o700)
        assert all(shutil.which(name) for name in ("sudo", "ip", "systemd-run", "systemctl", "setpriv", "env"))
        plan = dict(namespace=self.name, interface=self.interface, peer=self.peer, unit=self.unit,
            server_ip=self.server_ip, client_ip=self.client_ip, prefix=self.prefix, uid=os.getuid(), gid=os.getgid())
        exact(self.directory / "intent.json", (json.dumps(plan, sort_keys=True, indent=2) + "\n").encode())
        assert not Path("/var/run/netns", self.name).exists()
        assert not Path("/sys/class/net", self.interface).exists() and not Path("/sys/class/net", self.peer).exists()
        _, addresses = self.tool("original-addresses", ["ip", "-j", "address", "show"])
        assert not any(a.get("local") in {self.server_ip, self.client_ip}
            for interface in json.loads(addresses) for a in interface.get("addr_info", []))
        # Once an exclusive fresh name is submitted, cleanup must inspect that
        # same intent even if the command outcome becomes uncertain.
        self.created = True
        self.sudo("namespace-create", ["ip", "netns", "add", self.name])
        self.namespace_inode = Path("/var/run/netns", self.name).stat().st_ino
        self.link_created = True
        self.sudo("veth-create", ["ip", "link", "add", self.interface, "type", "veth", "peer", "name", self.peer])
        self.sudo("peer-namespace", ["ip", "link", "set", self.peer, "netns", self.name])
        self.sudo("server-address", ["ip", "addr", "add", self.server_ip + "/30", "dev", self.interface])
        self.sudo("server-link-up", ["ip", "link", "set", self.interface, "up"])
        self.sudo("client-address", ["ip", "-n", self.name, "addr", "add", self.client_ip + "/30", "dev", self.peer])
        for interface in ("lo", self.peer):
            self.sudo("client-link-up", ["ip", "-n", self.name, "link", "set", interface, "up"])
        _, raw = self.sudo("client-routes", ["ip", "-j", "-n", self.name, "route", "show"])
        routes = json.loads(raw)
        assert len(routes) == 1 and routes[0]["dst"] == self.prefix and routes[0]["dev"] == self.peer
        assert routes[0]["prefsrc"] == self.client_ip and routes[0].get("gateway") is None
        self.route_sha256 = digest(self.directory / (str(self.sequence) + "-client-routes.stdout"))

    def client_command(self, command):
        assert self.created and self.closed is None
        # Drop root and capabilities before opening any participant configuration.
        return ["sudo", "-n", "ip", "netns", "exec", self.name, "setpriv",
            "--reuid=" + str(os.getuid()), "--regid=" + str(os.getgid()), "--clear-groups", "--no-new-privs",
            "--", "env", "-i", *(k + "=" + v for k, v in sorted(self.env.items())), *map(str, command)]

    def server_command(self, binary, config, env):
        assert self.created and self.active_pid is None and self.owner is None
        selected = fixture_environment(env)
        if "RUST_LOG" in env:
            base = "info,connect_service::canonical_native=debug,connect_service::canonical_native_data=debug,rvoip_uctp::native_subscription_preflight=debug,rvoip_core::native_catalog=debug"
            assert env["RUST_LOG"] in {base, base + ",connect_service::canonical_replacement=debug,connect_service::replacement_phase_trace=debug"}
            selected["RUST_LOG"] = env["RUST_LOG"]
        return ["sudo", "-n", "systemd-run", "--quiet", "--collect", "--wait", "--pipe",
            "--unit=" + self.unit, "--uid=" + str(os.getuid()), "--gid=" + str(os.getgid()),
            "--working-directory=" + str(self.root), "--property=KillMode=control-group",
            "--property=RuntimeMaxSec=600", "--property=TimeoutStopSec=10s", "--property=NoNewPrivileges=yes",
            *("--setenv=" + k + "=" + v for k, v in sorted(selected.items())),
            str(binary), "serve", "--config", str(config)]

    def run_client(self, command, log, budget):
        with log.open("xb") as out:
            child = subprocess.Popen(self.client_command(command), env=self.env, cwd=self.root,
                stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
            joined = False
            try:
                code = child.wait(timeout=budget)
                joined = True
                return code
            except subprocess.TimeoutExpired:
                return "timeout"
            finally:
                if not joined:
                    self.client_forced = True
                    self.sudo("retire-original-client-group", ["/bin/kill", "-KILL", "--", "-" + str(child.pid)], success=False)
                    child.wait(timeout=10)

    def identify_server(self, owner):
        self.owner = owner
        _, value = self.sudo("server-main-pid", ["systemctl", "show", self.unit, "--property=MainPID", "--value"])
        assert value.strip().isdigit() and int(value) > 1
        self.active_pid = int(value)
        assert Path(f"/proc/{self.active_pid}").stat().st_uid == os.getuid()
        self.servers_verified += 1
        return self.active_pid

    def stop_server(self, owner):
        self.sudo("server-stop", ["systemctl", "stop", self.unit])
        code = owner.wait(timeout=10)
        assert code == 0 and (self.active_pid is None or not Path(f"/proc/{self.active_pid}").exists())
        try:
            os.killpg(owner.pid, 0)
        except ProcessLookupError:
            pass
        else:
            raise FixtureError("fixture-systemd-command-group-survived")
        self.active_pid = self.owner = None

    def close(self):
        if self.closed is not None:
            return self.closed
        result = dict(kind="ubuntu-netns", passed=False, original_namespace_removed=False,
            original_veth_removed=False, remaining_client_processes=None, forced_cleanup=self.client_forced,
            systemd_servers_verified=self.servers_verified, unprivileged_client_launch_required=True,
            external_default_ipv4_route=False if hasattr(self, "route_sha256") else None, failures=[])
        self.closed = result
        def attempt(work):
            try:
                work()
            except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
                result["failures"].append(str(error) if isinstance(error, FixtureError) else type(error).__name__)
        if self.owner is not None:
            attempt(lambda: self.stop_server(self.owner))
        if self.created:
            def clients():
                _, text = self.sudo("remaining-client-pids", ["ip", "netns", "pids", self.name])
                pids = [int(value) for value in text.split()]
                result["remaining_client_processes"] = len(pids)
                if pids:
                    result["forced_cleanup"] = True
                    for pid in pids:
                        if Path(f"/proc/{pid}/ns/net").stat().st_ino == self.namespace_inode:
                            self.sudo("retire-owned-client", ["kill", "-KILL", str(pid)])
            attempt(clients)
            if Path("/var/run/netns", self.name).exists():
                attempt(lambda: self.sudo("namespace-delete", ["ip", "netns", "delete", self.name]))
            result["original_namespace_removed"] = not Path("/var/run/netns", self.name).exists()
        if self.link_created:
            if Path("/sys/class/net", self.interface).exists():
                # Destroying an empty namespace may already retire its veth peer.
                # Any surviving original host endpoint still has to be removed.
                attempt(lambda: self.sudo("veth-delete", ["ip", "link", "delete", self.interface]))
            result["original_veth_removed"] = not Path("/sys/class/net", self.interface).exists()
        result["route_observation_sha256"] = getattr(self, "route_sha256", None)
        result["passed"] = bool(self.created and self.link_created and result["original_namespace_removed"]
            and result["original_veth_removed"] and result["remaining_client_processes"] == 0
            and not result["forced_cleanup"] and not result["failures"])
        return result
