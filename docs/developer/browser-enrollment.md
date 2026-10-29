# Browser client enrollment

Use `BrowserEnrollmentKey` and `BrowserEnrollmentStore` from
`@rudeless/connect/browser` in an HTTPS browser context. The client generates a
nonextractable Ed25519 or ES256 private key. It sends only its public key and
complete bounded request to a trusted backend, which approves through the
[administration endpoint](remote-enrollment.md). Never give a browser an operator
bearer or provider credentials.

Save the original request and client key before asking the backend for approval:

```ts
import { BrowserEnrollmentKey, BrowserEnrollmentStore } from "@rudeless/connect/browser";

const key = await BrowserEnrollmentKey.generate("ES256");
const vault = new BrowserEnrollmentStore("my-app-enrollment-1");
const original = await vault.prepare("saved-enrollment-name", serviceOrigin, {
    ...savedOriginalIntent,
    algorithm: key.algorithm,
    public_key: key.publicKey,
    browser_origin: location.origin,
}, key);
// The app's trusted backend authenticates the request and chooses its grant.
const offer = await backend.approve(original);
const enrollment = await vault.acceptOffer("saved-enrollment-name", offer);
const credential = await enrollment.claim({ signal });
const client = await enrollment.open(credential, trustedServerContact, { signal });
```

The caller supplies original approval/completion operation IDs, Identity/Device/
key IDs, Endpoint address/capabilities, literal operations/recipients and absolute
expiries in `savedOriginalIntent`. `serviceOrigin` is an exact HTTPS origin;
`trustedServerContact` separately supplies the approved control contact URL,
server signing trust and any explicit development WebTransport certificate pins.
The Endpoint declaration describes the enrolled client; it does not replace
server trust or prove network/media readiness.

The vault normalizes the original request and commits it with the nonextractable
key handles to origin-local IndexedDB before returning. It requests strict
transaction durability and waits for transaction completion. A name cannot
replace an earlier original intent. Approval must match every normalized request
field, including the exact current HTTPS Origin. A different offer/challenge
cannot replace an already accepted offer, even after its challenge expires.

After a reload or interruption, use the same vault and name:

```ts
const enrollment = await vault.load("saved-enrollment-name");
const credential = await enrollment.claim({ signal });
const client = await enrollment.open(credential, trustedServerContact, { signal });
```

The saved key, offer, challenge and original completion ID remain unchanged.
No issued bearer is saved by the vault; current original claim recovery obtains
it again. Completed recovery may outlive challenge expiry, subject to current
issuer, credential, Device, key, Endpoint and receipt authorization. An expired
fresh challenge cannot create a grant. The client never retries automatically or
generates a replacement ID after cancellation, timeout or lost observation.

Claims use browser Fetch with cookies omitted, no Authorization header, no
manually supplied Origin, redirect refusal, no caching and no referrer. The
browser supplies the actual Origin. One bounded deadline includes signing,
network IO, streamed response collection and decoding. Requests/responses are
limited to 64 KiB. Success must match the complete original offer, issuer,
installation, scopes/recipients and absolute expiry. Refusal text is sanitized;
server-provided messages/details are not exposed by SDK errors.

Installation keeps the child bearer in memory and fixes the control transport to
the approved WebTransport or WSS profile. A credential approved for one cannot
be used for the other's fallback. A transport change requires a fresh explicitly
approved enrollment/key. Use `vault.resolveProfiles(...)` with separately saved WT/WSS profiles to resolve
the correct original credential for each attempted contact; see
[transport selection](browser-transport-recovery.md). This does not change either
enrollment's immutable binding.

Rotation uses a fresh vault name, approval/completion IDs and client key. The
backend may approve the exact existing Device/Endpoint for the replacement,
then revoke the old enrollment through administration. Replacing a key in saved
old state is refused. `vault.forget(name)` removes only local recovery material;
it does not revoke a server credential.

Key/session objects serialize to no fields. Deliberate `snapshot()` exports
contain sensitive nonextractable CryptoKey handles and the protected offer;
use structured clone/IndexedDB only. JSON/localStorage cannot preserve those
key handles. Same-origin application code that can use a key remains trusted
client code. Do not log snapshots, claims, offers or protected credential results.

[Browser evidence](../evidence/gate-6/enrollment-browser-client.json) records
actual local Chrome HTTPS, IndexedDB and signed WebTransport/WSS checks. The
fixture trusts only its synthetic leaf SPKI in an owned temporary browser
profile; it does not modify system trust or personal browser state. Native
installation is documented in [native enrollment](native-enrollment.md).


## Explicit application read service

A typed browser SDK may configure `resolveApplicationRead` with
`vault.resolveApplicationReads({ name, url })`. Use a separately approved saved
browser enrollment for the same Identity/Device and exact Origin. Receipt
queries require the existing `history` scope and current original audience;
that scope also covers receipt reporting/preferences in the server API. The
client read interface sends queries only. It never borrows its active signaling
credential or derives an HTTP endpoint from a WT/WSS URL.

The service URL is an explicit HTTPS origin matching the enrollment service.
`BrowserEnrollmentSession.applicationReadOptions(credential, url)` validates the
original claimed grant and returns memory-only HTTP client options. The server's
finite HTTP Origin ceiling is independent of the approved signaling transport;
no `http-json` EnrollmentTransport is added. A separately enrolled `wss-webrtc`
profile can therefore supply this HTTP service while its original scopes,
recipient ceiling, key/Device lifetime and exact Origin remain enforced.

Resolution restores and reclaims the same original enrollment, not a replacement
identity, key or operation. The SDK's one read deadline/cancellation signal covers
this work and all subsequent pages. See [the receipt API](browser-sdk.md) for
current authorization, bounded reads, cancellation and separate delivery facts.
