# Gate 6 authentication scope audit

Reviewed 2026-10-16 against G6-026, the 42 requirements U6-REQ-0025 through
U6-REQ-0066 in `contracts/uctp-gate-6-requirements.json`, and the corrected
`contracts/connect-uctp.md` profile. The reviewed task-level mapping is `contracts/gate6-authentication-evidence.json`;
`scripts/check-gate6-authentication-audit.py` binds it to observed runtime cases.
This is reusable-auth implementation evidence, not independent conformance or a
change to the ledger's full-profile conformance status.

G6-026 owns refresh/expiry, installed signing methods, assurance provenance,
step-up, replay and authentication of correlated replies. G6-037 explicitly
depends on G6-026 and owns durable Connect service dispatch/policy installation.
G6-038 owns service projection of participant/session state. Their downstream
integration must not become a prerequisite cycle for closing the reusable auth
implementation. Neither local fixture evidence nor this ownership distinction
qualifies those downstream tasks.

| Requirements | Implemented behavior and evidence | Remaining boundary |
|---|---|---|
| 0025–0031: handshake, Device, session token, keepalive, logout | The actual canonical peer authenticates one Identity/Device with an installed method, finite expiry and signed correlated proof. Refresh rotates the credential incarnation and token; old waiters retire. `v2_auth_peer` covers exact challenge scope, token non-portability, downgrade/provenance refusal, cancelled/failed refresh and real TLS/QUIC EOF. | The token is not a reconnect/resumption credential. An unscoped handshake cannot fabricate a conversation Participant. Durable reconnect is unqualified. |
| 0032–0034: reachability | `v2_reachability` covers trusted adapter contacts, bounded renewal, multiple Devices and owner domains, recipient authorization after dequeue, stale incarnations, queue pressure and actual TLS/QUIC submission. Canonical transport factories install the physical context. | IdentityProvider/Connect EndpointRuntime projection and incoming Session invitation selection/acceptance remain separate integration. A hint or local write is not delivery or readiness. |
| 0035–0036: method discovery | `handshake_selects_only_installed_method_and_retains_identity_profile_without_fabricated_participant` and `unknown_method_does_not_fall_back_or_execute_held_intent` verify discovery and refusal. The approved profile makes backends conditional. | The installed canonical credential backend currently advertises bearer only. Other named methods, including experimental AAuth, are not inferred from the registry or unrelated auth libraries. |
| 0037–0045: signing and covered fields | Principal-owned Ed25519 and P-256/SHA-256 JCS verification covers the complete original envelope, including unknown optional fields. The challenge advertises EdDSA and ES256; held reply verification checks the current exact key under the existing lease. `v2_auth_dispatch`, `sig9421_gate` and the auth regressions exercise key ownership, tampering and field coverage. | Canonical inline signing does not claim HTTP-header signature processing. The baseline ES256 SHOULD is implemented in every verifier path and advertised. PS256/RS256 remain optional and unadvertised; DER signatures are rejected. |
| 0046–0049: replay and signature problems | `v2_auth_recovery` exercises actual replay-store TTL expiry followed by the non-evicting physical-peer fence, refresh/step-up continuity, bounded exhaustion, fresh-envelope logical retry and signed safe error taxonomy. | The physical fence is process-local and covers all Connections owned by that physical peer. A reconnect requires fresh authentication and a new Connection; it cannot borrow the old route. Persistent logical Session/operation recovery belongs to G6-037/G6-052; the fixture effect set is not that ledger. |
| 0050–0052: commands/replies, floors and physical limits | `v2_secure_profile` covers all typed command floors, denied IDs, pre-auth capacity and reply floors. `peer_exchange` exercises actual QUIC/WT/WSS same-peer replies, exact family/CID/SID/Connection/operation correlation, shared key/replay authority, queued-writer liveness and lifetime cancellation. Core/native suites cover 32 associated Sessions, 64 Connections and retained 16-new-stream history. | These observations do not complete media negotiation. Media-control fixtures use controlled readiness/store projections. Other service/media responsibilities retain their own tasks. |
| 0053–0058: assurance and Identity/Device/Participant distinctions | The advertised identified authentication floor, authenticated binding, task/user provenance, exact task floor, current requirement recheck and forbidden downgrade are covered by `v2_auth_peer`/`v2_step_up`. Core relationships retain separate conversation membership, Session appearance and Device/endpoint binding. | Actual Connect membership and endpoint projection must use current store authority. A role, Device claim or assurance label grants no resource permission. |
| 0059–0060: assurance-change observations | Verified step-up emits a scoped signed `identity.assurance-changed` with the original result. The retirement increment adds a required policy lifetime hook, exact current/candidate incarnation watches, and a signed terminal assurance-change/goodbye pair for credential/task expiry and factual revocation causes. See `assurance-retirement.md` and its source-bound report. | Terminal anonymous means the physical authenticated binding has ended, not a new execution grant. Blocked/failed writers cannot guarantee remote observation; business outcomes remain subject to reconciliation. Real Connect store-policy installation is still G6-037. |
| 0061–0066: step-up and blocked mutation | `v2_step_up` and `v2_auth_recovery` cover exact challenge/peer/resource binding, one-use held proof, current old/new credential authority, wrong scope/signature recovery, requirement changes during validation, timeout/supersession, original expected revision, dual failure correlations and interrupted execution with unknown outcome. | Business results after uncertain execution still require the durable application ledger. Successful step-up does not reauthorize old detached work or silently rotate a retained media controller's original credential. |

The current media-control increment supplies exact-intent held grant composition
for the engine, while ordinary packets still acquire fresh current policy.
Regression cases reproduce and correct the earlier task-delegation expiry gap:
neither a still-held current grant nor a later principal expiry can extend the
original delegation's control or packet authority. This is not checked credential
rotation, ParticipantPolicy held-authority composition or Connect store policy.

The source-bound runtime evidence is in `docs/evidence/gate-6/es256.json`;
`docs/evidence/gate-6/authentication-audit.json` records each reviewed clause's
actual observed cases, source bindings and remaining shared integration. Earlier
reports retain their recorded source snapshots. Counts alone establish no
lifecycle conformance; each mapped case was reviewed against its stated behavior.

The replay-lifetime scope follows the actual owned route: baseline section 4.2
ties a Connection to its WT physical session and requires new authentication on
reconnect; section 7.3 describes reconnect as a new Connection in the retained
logical Session. The canonical physical fence never evicts consumed IDs during
that binding, including refresh and step-up. Core retirement fences its actual
routes on physical EOF, and another physical peer cannot borrow them. The shared
atomic replay store additionally enforces the five-minute freshness/cache window
across verifier instances. This does not claim restart persistence for that
physical fence or restored media merely because a logical Session exists.
Durable logical operation and cursor recovery retain G6-037/G6-052; accepted
business outcomes must be reconciled under their stable operation identity.

The current identified floor is a declared deployment minimum, permitted by the
baseline assurance policy. Lower gradient values remain representable and cannot
silently gain authority. Only installed authentication methods are advertised.
The selected inline signing form does not advertise the optional HTTP-header
alternative. Signatures prove the registered key; current resource grants and
verified delegation provenance remain independently required.

The unchanged G6-026 task contract is reviewed against handshake/method discovery,
refresh/expiry and revocation, EdDSA/ES256 signing, exact assurance/step-up,
physical/scoped replay, fresh-envelope logical retry and authenticated correlated
replies. Its reusable implementation can close when the runtime qualification and
the complete reviewed mapping both pass. Downstream G6-037/G6-038/G6-046/G6-052
installation and full independent G6-094 conformance remain explicit work; every
Gate 6 acceptance criterion remains unqualified. No full-goal completion is
inferred from closing this implementation task.
