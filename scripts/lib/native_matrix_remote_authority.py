"""Current-authority native clients in the owned ARM guest, issuer on the Mac.

Each independent expiry/revocation probe gets its own existing-VM process owner.
The short credential is issued after source staging. No VM or cloud instance is
created, and no Mac administration configuration is exposed as a guest route.
"""
import json
from pathlib import Path
import tomllib

from .transport_fixture import publish, toml_bytes
from .transport_matrix import digest


class RemoteNativeAuthorityFactory:
    def __init__(self, parent, *, fixture_class=None):
        if fixture_class is None:
            from .mac_linux_fixture import MacLinuxFixture
            fixture_class = MacLinuxFixture
        self.parent, self.fixture_class = parent, fixture_class
        self.client_executable = parent.client_binary
        self.client_binary_sha256 = parent.client_binary_sha256
        if not self.client_executable or not self.client_binary_sha256:
            raise ValueError("Original verified ARM native executable required")
        self.prepared, self.owners = {}, {}

    def prepare(self, case):
        case = Path(case).resolve()
        if case in self.prepared:
            raise ValueError("Original remote authority case was already prepared")
        remote = self.fixture_class(self.parent.root, case / "remote-owner", self.parent.env)
        remote.lease_parent = self.parent
        remote.start(self.parent.revision, image=False)
        if (remote.server_ip != self.parent.server_ip or remote.client_ip != self.parent.client_ip
                or remote.revision != self.parent.revision):
            raise ValueError("Authority client changed the original remote topology")
        self.prepared[case] = remote

    def __call__(self, command, directory, log, *, cwd, env, wrap=None):
        directory = Path(directory).resolve()
        case = directory.parent
        if case not in self.prepared or case in self.owners or wrap is not None:
            raise ValueError("Use the original prepared remote authority owner once")
        if len(command) != 4 or list(map(str, command[:2])) != ["node", "scripts/check-transport-native-authority.mjs"]:
            raise ValueError("Remote authority factory only runs its original public SDK probe")
        source, output = map(lambda p: Path(p).resolve(), command[2:])
        if source.parent != case or output.parent != case:
            raise ValueError("Original authority IO scope differs")
        remote = self.prepared[case]
        value = json.loads(source.read_text())
        if (value["executable"] != self.client_executable or value["binary_sha256"] != self.client_binary_sha256
                or value["source_revision"] != self.parent.revision or value["network_profile"] != "remote"):
            raise ValueError("Authority probe changed its verified original ARM executable")
        # Keep the issuer-generated SDK file intact. This second config is a
        # named guest transport projection, with only native control custody.
        original_config = Path(value["config"]).resolve()
        if original_config.parent != case:
            raise ValueError("Original issued authority configuration required")
        guest_config = case / "guest-sdk.toml"
        config = tomllib.loads(original_config.read_text())
        config.pop("application", None)
        publish(guest_config, toml_bytes(config))
        remote.stage_native_config(guest_config)
        remote.stage_json(source, dict(value, config=str(guest_config)))
        remote.launch(["node", "scripts/check-transport-native-authority.mjs",
            remote.mapped(source), remote.mapped(output)], case, log, timeout=45, role="helper")
        owner = RemoteNativeAuthorityOwner(remote, case, output)
        self.owners[case] = owner
        return owner


class RemoteNativeAuthorityOwner:
    def __init__(self, remote, case, output):
        self.remote, self.case, self.output = remote, case, output
        self.receipt = None
        self.code = None

    def alive(self):
        # SSH may finish just after poll(); the next poll still must collect its
        # original guest receipt rather than misclassify that boundary as loss.
        return self.receipt is None

    def poll(self):
        if self.receipt is not None:
            return self.code
        self.remote.poll()  # Only bounded original revoke request/response IO.
        if self.remote.owner.poll() is None:
            return None
        self.close()
        return self.code

    def close(self):
        if self.receipt is not None:
            return self.receipt
        observed = self.remote.close()  # Joins guest and SSH; verifies collected hashes.
        guest = observed.get("guest", {})
        self.code = guest.get("exit_code")
        if type(self.code) is not int:
            self.code = -1
        try:
            if self.output.is_symlink() or self.output.stat().st_size > 16384:
                raise ValueError("Bounded original remote authority observation required")
            local = json.loads(self.output.read_text())
        except (OSError, ValueError):
            local = {}
        joined = observed.get("original_joined") is True and guest.get("original_joined") is True
        actual_helpers = (local.get("original_helpers_joined") is True
            and local.get("original_owners_joined") is True
            and local.get("opened_client_owners") == local.get("closed_client_owners")
            and type(local.get("observed_helpers")) is int and local["observed_helpers"] > 0)
        self.receipt = dict(original_supervisor_joined=joined,
            original_workers_joined=joined and actual_helpers, forced=observed.get("forced") is not False,
            exit_code=self.code, client_platform="ubuntu24-arm64",
            source_revision=self.remote.revision, executable_sha256=local.get("binary_sha256"),
            original_guest_receipt_sha256=digest(self.case / "guest-owner.json")
                if (self.case / "guest-owner.json").is_file() else None)
        return self.receipt
