# Connect on Ubuntu 24.04 x86_64

The primary Linux installation is a versioned binary archive and a systemd
service. Build on native Ubuntu 24.04 x86_64 with the repository's pinned Rust
1.97.1 toolchain. Publication and external deployment are separate actions.
G6-054 is deferred; this installation does not establish browser/media or full
Gate 6 qualification. Development checkouts are not the installed runtime.

## Build an unpublished package

On a clean committed checkout, install build prerequisites:

```sh
sudo apt-get update
sudo apt-get install -y build-essential clang cmake pkg-config libopus-dev libssl-dev ffmpeg
python3 scripts/package-linux.py
```

The script prepares the recorded upstream dependencies before building the
normal `connect` release binary without `test-support`. It copies license texts,
source-lock provenance, source hashes, linked-library diagnostics and the
installer into `dist/connect-*-ubuntu-24.04-x86_64-<revision>.tar.gz`, with a SHA256
sidecar. No provider credentials, model account, Node, Python SDK, Rust compiler,
source checkout or writable source tree is required to run this native server.
Python 3.12 is needed only for installation. Optional model workers and browser
clients have their own prerequisites and qualification lanes.

The archive has sorted entries, normalized ownership/modes/mtimes and a fixed
gzip timestamp. Repacking identical runtime inputs produces identical bytes.
This does not claim bit-identical compilation across different build machines.
The manifest records the actual compiler and source revision. `dependencies/ldd.txt`
records the build host's dynamic linkage; it is diagnostic provenance, not a
requirement to pin obsolete system security updates.

## Install on a fresh host

Obtain the archive and its trusted digest from the chosen delivery channel;
a digest shipped beside an untrusted archive does not authenticate its publisher.
Keep packages in a private operator directory. Install runtime dependencies:

```sh
sudo apt-get update
sudo apt-get install -y python3 libopus0 libssl3t64 ffmpeg
sha256sum --check connect-*.tar.gz.sha256
tar -xzf connect-<version>-ubuntu-24.04-x86_64-<revision>.tar.gz
sudo python3 connect-<version>-ubuntu-24.04-x86_64-<revision>/deploy/install.py --initialize
```

Use one exact filename for each command. The installer checks Ubuntu/architecture,
all manifest paths/digests, required runtime libraries and the stopped service.
It creates a dedicated `connect` system account and does **not** start listeners.
It refuses existing release destinations and refuses initialization over state.

Installed layout:

| Path | Purpose / ownership |
| --- | --- |
| `/opt/connect/releases/<installation-id>` | Immutable binaries/notices; root owned |
| `/opt/connect/current` | Atomic symlink to the selected release |
| `/etc/connect/server.toml` | Canonical configuration; root:connect, 0640 |
| `/etc/connect/environment` | Optional provider secret injection; root:root, 0600 |
| `/var/lib/connect/installation` | Database, blobs, signing/trust material; connect, 0700 |
| `/etc/systemd/system/connect.service` | Service definition; root owned |

Fresh initialization creates a **local loopback** profile on UDP 4433. Its
initial operator credential and signing pin expire after 30 days. Enroll and
renew deployment credentials through the authenticated administration interface
before expiry. Protect `operator.toml` and its private key as credentials. Do
not copy operator credentials to application participants or browser clients.
The service account owns persistent state but cannot write the binary tree or
root-owned service/configuration. Back up the complete state and protected
configuration, not just the SQLite main file while WAL writers are active.

## Configure listeners, TLS and secrets

Review `/etc/connect/server.toml` before starting. Keep `database` under
`/var/lib/connect`, and use absolute certificate/private/signing paths. A remote
installation requires `network_profile = "remote"`, explicit bind addresses,
advertised secure origins/URLs and the appropriate listener configuration.
Plaintext application HTTP remains loopback. A TCP reverse proxy does not carry
native QUIC or WebTransport UDP. Configure firewalls for each actual UDP/TCP
listener; no firewall changes are made by the installer. Use unprivileged ports
(4433/8443, for example), or terminate externally on a separately configured
proxy. The shipped service has no privileged bind capability.

Replace the generated local TLS identity with the deployment certificate before
remote use. The native, HTTPS, WSS and WebTransport listeners share the configured
DER certificate and PKCS#8 private key. Keep the canonical server signing key and
its separately pinned public key distinct from TLS. Preserve peer trust and key
validity on rotation; never learn a new signing pin from an unauthenticated peer.
Use exact browser origins, not wildcards. Check the configured surfaces against
the [HTTP reference](developer/server-http.md), [canonical OpenAPI](../contracts/openapi-canonical-http.json) and [hosting guide](hosting/index.html).

For environment-backed secret references, create `/etc/connect/environment`
with `sudo install -m 0600 -o root -g root /dev/null /etc/connect/environment`, then
edit it as root. Use systemd `EnvironmentFile` syntax (`NAME=value`), without shell
commands or `export`. Include only names referenced by this installation's
explicit server policy. These values are not command-line arguments and are not
exported into backups. Reinject them independently on replacement hosts.
Provider endpoints/calls and model execution still require their named bounded
rehearsal authorization; installing the package does not authorize those calls.

```sh
sudo systemctl enable --now connect.service
sudo systemctl status connect.service --no-pager
sudo journalctl -u connect.service -n 30 --no-pager
```

`active` means the process started, not that every listener is ready. Wait for the
server's `ready transport=quic ... profile=connect/2` line, then connect with an
enrolled signed client on the actual advertised endpoint and perform an
authorized read. For remote deployments, run that check from a separate host.
A local loopback read or health endpoint alone is not a remote-client pass.
Do not upload raw journal logs, configs, tokens or private keys as evidence.

The unit uses SIGTERM for graceful shutdown and a 45 second stop timeout.
`systemctl reload connect` sends SIGHUP to reload the configured TLS certificate
pair; it does not reload arbitrary configuration. Use a restart for other
configuration changes. Review failures before restarting repeatedly: systemd
limits restart bursts. Diagnostics: check path ownership, key validity, library
availability (`ldd /opt/connect/current/bin/connect`), exact configured ports,
firewall/proxy routing and the server's redacted error. Never solve a failure by
making `/opt/connect` writable to the service or disabling authentication.

## Stop, back up, restore, upgrade

Choose a backup parent under `/var/lib/connect` with mode 0700 so the service
account can create a fresh backup directory. Stop the service first: the canonical
backup command excludes live runtime and maintenance writers while it captures
one Store snapshot, its referenced blobs, TLS/signing files and the selected
operator's enrolled key/pin. Optional Vapi peer files and secret **references**
are included; ambient environment values and external keychain values are not.
Only a completely verified backup directory is published.

```sh
sudo systemctl stop connect.service
sudo install -d -o connect -g connect -m 0700 /var/lib/connect/backups
sudo -u connect /opt/connect/current/bin/connect backup \
  --server-config /etc/connect/server.toml \
  --config /var/lib/connect/installation/operator.toml \
  --output /var/lib/connect/backups/before-upgrade
sudo systemctl start connect.service
```

Copy verified backups to protected encrypted storage with the binary/package and
its digest. Do not expose them as CI artifacts: they contain credential material.
A missing manifest, bad hash, unsupported schema, unsafe path, symlink or missing
referenced blob is a refusal. Blob bytes must also match the database's recorded
size and SHA-256, including when a supplied manifest checksum has been changed.
Blob inclusion uses the recorded snapshot capture time so natural expiry does
not invalidate a stored backup. Restored live reads still enforce current expiry;
backup verification never renews file access. Backup requires a current administrator credential.
Restore does not renew expired credentials or server pins; renew before serving.

Restore only into a **new** directory, on a stopped host, using the matching
current-schema binary:

```sh
sudo -u connect /opt/connect/current/bin/connect restore \
  --backup /var/lib/connect/backups/before-upgrade \
  --directory /var/lib/connect/restored
```

Update the root-owned server configuration to the restored portable `server.toml`
with all four top-level file paths made absolute under `/var/lib/connect/restored`.
Also relocate any optional Vapi peer/reference file paths explicitly. Preserve
listener/profile settings and secrets; the restore command never silently changes
them. Reinject protected environment secrets. Start, wait for readiness and verify
with the restored enrolled client. Preserve the old state until verification;
never run the original and restored identity simultaneously on public endpoints.
Runtime restart recovery remains authoritative for uncertain operations; restore
does not replay calls or create replacement logical operation IDs.

To upgrade, stop and take a verified backup, then run the **new** package's
installer without `--initialize`. It installs a new version and atomically selects
it while preserving configuration, state and injected secrets. Start and repeat
signed client/readiness checks. Retain the old binary with its verified backup.
An incompatible schema is a refusal; initialize a new state path deliberately.
Do not point an older binary at state touched by a newer binary or automatically
roll back after a failed start. There is no historical schema converter or
compatibility/downgrade path in this pre-alpha installation.

## Verification boundary

`scripts/check-linux-installation.py` exercises the package on a disposable native
Ubuntu 24.04 systemd host: install, signed remote-profile client in a separate
network namespace, graceful restart, upgrade activation, stopped backup, restore
and secret/configuration preservation. It then removes only its own installation
roots and freshly initializes the current package, verifies a new empty state,
performs a signed mutation/read and checks the actual running binary and source
isolation. Pre-existing installation paths and any pre-existing unit are refused
before the rehearsal starts. It uses synthetic identities and local
networking only. Raw state/logs remain under ignored `.local/`. Its curated report
must identify source/package digests and the actual assertions before G6-056 can
be marked complete. Browser/media qualification remains G6-054.
