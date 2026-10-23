# HTTP record retrieval

`CanonicalHttpClient::fetch_record` sends `recording.vcon-fetch` through
`POST /v2/commands` using the current bearer. It uses the same selector, projected
vCon schema and exact SHA-512 digest checks as the native SDK. Discovery is
available through `conversation_records`. UUIDs, URLs, prior hashes and membership
labels are not read grants. Arbitrary selector URLs are never fetched over the
network.

The participant reader requires explicit `record_get`, current Conversation/Session source
access and the supported record's original membership epoch or current explicit
Session history share. Participant recipient ceilings, private Connection source
ownership, capture diagnostics, retention and current record revision are checked
before projection. The actual Store tenant is used. HTTP bearer admission does
not mint or borrow a signed native peer grant.

[Explicit tenant reader roles](tenant-record-readers.md) use independent current
role/tenant/source policy. Their reads use the same retained HTTP and TLS custody,
with role revisions pinned in issued views; no membership is created.

The response profile is HTTP/1.1 with `Connection: close` and `Cache-Control:
no-store`. HTTP/2 and upgrade requests are refused for record delivery. Small
records are inline, bounded to 128 KiB of vCon bytes and the canonical 2 MiB JSON
batch limit. Larger views use the optional separately bound HTTPS endpoint
described below; without it they return explicit delivery-unavailable. A missing record returns
a correlated 404; a changed conditional view returns 409. No secret URL fallback
is installed.

The Service acquires one original current bearer/source fence and derives source
facts under it. Core then holds the exact protected record read; revision/body/
manifest mismatches refuse delivery. Source validity performs no SQL or authority
reader reacquisition, including when a writer is queued. The view identity matches
the native reader so the same authorized view has the same digest across both
transports and successive conditional reads.

The HTTP listener owns the opaque Service response and its concurrency permit in
per-socket custody. Consuming an HTTP Body, copying bytes into Hyper or reaching
Body EOS releases neither. Before each actual socket operation the IO wrapper
checks current projection, cancellation and a bounded deadline (at most 30 seconds
and the original credential expiry). A timer wakes stalled writes. The closing
HTTP/1.1 connection drops record/source custody only when its actual IO ends;
expiry, disconnect, shutdown or runtime retirement aborts remaining IO and frees
the guards and capacity. Server shutdown cancels HTTP IO before draining other
runtime work that might need those authority writers.

The transport qualification uses an explicit 100 KiB permitted metadata fixture,
seeded through a privileged backend CAS after a real public Session outcome. It
is for socket backpressure and exact-byte custody, not a claim about captured
media or public redaction semantics. Public redaction/deletion and named retention
workers have separate recorded qualification. Captured-media references and full
auxiliary protocol coverage remain G6-046 work.

## Authenticated HTTPS references

The canonical server's optional `[record_https]` configuration supplies `bind`,
`server_name`, `capacity` (1–4096 reference entries) and `ttl_seconds` (1–300).
The current profile requires loopback. The native TLS certificate and key also
serve this endpoint; startup verifies the certificate authenticates `server_name`.
The listener advertises only HTTP/1.1. A large `recording.vcon-fetched` reply gives
an HTTPS URL, expiry and the exact projected SHA-512 digest.

`GET /v2/record-references/{token}` requires a current bearer on every request.
The ephemeral registry stores only resource/view metadata. The token supplies no
permission: current explicit `record_get`, Session/source access, viewer, record
revision, body/manifest binding and exact projected hash are checked anew. Changed
or expired views return 409; missing references/records return 404; denied current
authority returns 403. Authentication precedes registry lookup. Restart loses
references; endpoint stop cancels their lifetime. Registry capacity never evicts a
live entry to manufacture a successful response.

Raw vCon responses are bounded to 16 MiB, `application/vcon+json`, `no-store` and
`Connection: close`. Original bearer/source and exact record guards remain owned
through actual TLS flush and socket close, including backpressure. Credential or
reference expiry, disconnect, endpoint stop and runtime retirement abort remaining
IO. The existing 30-second maximum also applies.

The actual private-consultation link case retains the Session producer's original
private/public Connection sources and epochs. Pinned HTTPS downloads distinguish
the consultant's view from the retained remote participant's narrower view; using
the consultant's URL with the remote credential returns 403. Public source erasure
of the consultant's private Connection invalidates both original revision-bound
URLs with 409. Credentials remain valid, replay preserves the erasure receipt,
and fresh authorized downloads preserve the consultant's public Connection and
the remote participant's own source. Only response size uses explicit trusted
256 KiB metadata padding after real Session finalization; no captured media or
synthetic source audience is asserted.

The native Rust SDK follows references using its original TLS certificate/name/IP
and its current successfully refreshed bearer. The HTTP SDK requires explicit
`with_record_tls(RecordTlsConfig { server_name, trust_certificate })` before it
follows a URL. Both reject other origins, malformed token paths, user information,
queries/fragments, redirects, proxy routing and system trust fallback. Downloads
share the original fetch deadline and credential/key/reference expiry bounds;
length, exact UTF-8 digest, vCon schema, UUID and optional view condition are
verified before returning content. No named remote or captured-media profile is
qualified by this local metadata transport.

The selector can use a live HTTPS URL issued by this server plus its content hash.
This is a fresh control read: current source authority and its projected hash are
recomputed, including recipient attenuation. Conversation/Session scope and an
optional UUID must match the same resource. Expired, foreign or unknown URLs do
not cause network fetches or fall back to a supplied UUID. A changed condition
returns a correlated conflict; obtain a fresh authorized view explicitly.
