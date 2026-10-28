# Owned distinct-network native audio rehearsal

`scripts/rehearse-gate6-native-media.py` runs a synthetic Ubuntu Linux/amd64
public SDK peer in the existing local Lima VM against a current macOS server
fixture. The other audio peer runs beside that server. This is an owned local
network fixture. It does not use model accounts, providers or telephone numbers.

Both peers use `CanonicalClient::prepare_public_native_audio`, signed admission
and subscription commands, the explicit Remote profile and the fixture CA with
normal hostname verification. The server listens on a wildcard address; local
clients use loopback and the guest resolves `host.lima.internal` to a non-loopback
IPv4 destination. No verifier or hostname checks are disabled.

The runner requires source-bound build records before launching either process:

```sh
python3 scripts/rehearse-gate6-native-media.py \
  --server-build PATH_TO_SERVER_BUILD_SUMMARY \
  --server-inputs PATH_TO_SERVER_SOURCE_HASHES \
  --guest-build PATH_TO_LINUX_CLIENT_BUILD_SUMMARY \
  --output .local/gate-6/native-remote/FRESH_RUN
```

The server summary records a successful `gate6_canonical_runtime` test build,
zero changed inputs and `artifact.sealed` / `artifact.sha256`. Its source hash
manifest maps repository-relative names, including actual prepared upstream
sources, to SHA-256 values. The guest summary records `passed`, `executable`,
`executable_sha256`, `source_inputs_sha256` and the exact Docker `image_id`.
Build `crates/connect-client/examples/native_media_probe.rs` as the Linux guest
example. The default guest image is the existing Ubuntu 24.04 fixture image;
the runner verifies its digest and executes that digest rather than its tag.
Run `scripts/prepare-dependencies.sh` before either build. A successful compile
alone is not a media pass.

For each of EdDSA and ES256, FFmpeg produces four seconds of different synthetic
PCMU tones. The peers send 200 frames at 20 ms cadence and record received PCMU
inside the private fixture directory. The independent observer decodes with
FFmpeg, requires at least 50 active frames and requires the opposite tone to
exceed the self tone by four times in at least 80 percent of active frames.
Silence or self echo cannot qualify duplex audio. These are fixture signal
identification thresholds, not a performance or packet-loss acceptance profile.

The server checks completion of both original signed offer operation IDs,
exactly two durable native offer records, explicit peer shutdown and zero
physical peers before its own shutdown. The runner checks both actual process
exit codes, execution of the exact ignored child test, unchanged build inputs,
unchanged executable bytes and the decoded observations. It records failures
and owns cleanup only for the server process group and the container ID it
created. Raw logs, credentials, audio and build records stay beneath `.local/`.

The Linux/amd64 guest uses QEMU on an arm64 Lima VM. This does not qualify native
Linux x86_64 performance, a Linux-hosted server, WAN deployment, video, browser,
SIP/RTP, TURN, packet loss or ICE restart. Those Gate 6 checks remain separate.
