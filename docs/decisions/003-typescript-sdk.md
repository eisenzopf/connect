# 003 — TypeScript native transport experiment

Measured on macOS arm64 / Node v26.7.0 / Rust 1.97.1, 2026-10-07.

## Options

1. In-process binding: Node on this host rejects `import('node:quic')` with
   `ERR_UNKNOWN_BUILTIN_MODULE`. A Rust N-API binding remains feasible, but
   Connect has no built, packaged `.node` module yet. It requires native
   artifact delivery, cancellation across the JS/Rust boundary, and ownership
   tests for runtime shutdown. No available in-process package is claimed.
2. Explicit native helper: a TypeScript script starts the Rust fixture client,
   which performs authenticated QUIC request/response with verified trust.
   The helper executable and script are copied to an isolated directory and
   run without their source checkout or dependency tree at runtime. No WS
   transport is involved. The extra process is explicit.

Command: `python3 scripts/check-typescript.py`, exit 0. One observed cold copied
binary exchange took 431.33 ms; an earlier warm exchange took 100.02 ms. These
include process launch, QUIC/TLS, fixture assertions and shutdown; they are not
network or SDK performance claims. The helper and fixture server both shut down
within bounded timeouts, with exactly one application execution.

## Decision and next action

Keep the Rust SDK as Gate 1's qualified native client. For the broader TypeScript
SDK, pursue a persistent JSON-stdio helper first with scoped config files,
correlated requests, bounded queues, cancellation and explicit child lifecycle.
Compare that implemented event/recovery path with N-API before finalizing
v0.1's installed package. A one-shot architecture probe is not that full SDK.
The protocol profile and logical operation IDs remain identical across options.
No native TypeScript SDK availability is promised by this experiment.

## Gate 2 follow-through

The persistent JSON-stdio helper is now implemented. TypeScript `NativeClient`
and Python `NativeClient` use the Rust SDK for verified UCTP/QUIC; `HttpClient`
is an explicit local HTTP/JSON alternative. The earlier experiment remains
historical evidence, not a published package claim. See [SDK.md](../SDK.md).
Distribution, supported runtime packaging and browser media remain unqualified.
