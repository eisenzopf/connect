# Received DTMF source composition (G6-024 in progress)

A SIP or WebRTC adapter explicitly registers a bounded source subscription against its installed Core adapter and original live Connection, Session, Participant, Conversation and tenant. Unregistered input remains a notification hint. Subscriptions and observations hold metadata without a signal permission reader. Replacement or dropping the subscription invalidates queued observations and already prepared routes. Supported Core binding mutations invalidate a monotonic source epoch, including a return to the same Session after an intervening binding.

Received SIP INFO retains the actual parsed transaction, source lifecycle, stable request digest and receive time. Received RFC 4733 retains the decoder's actual SSRC, payload type, clock rate, extended event timestamp, duration samples, observed start and receive times. SIP media updates retire the original media generation under the existing configuration lock, including a failed update or rollback. Old queued media cannot acquire a successor generation. The decoder retains the first packet’s monotonic receive time across event segments and profile changes, so a tone that starts before an update and ends afterward also cannot acquire the new generation. Source registration applies the same first-packet boundary. WebRTC uses the actual inbound track identity and a separate bounded signal registry; telephone-event input never replaces the primary audio clock anchor. Track replacement, pump completion and peer retirement invalidate its physical fence, even when primary clock capture is disabled.

The host consumes `ReceivedDtmfObservation`, journals and deduplicates its physical observation ID into the original logical operation, then calls `prepare_route` with that operation and the requested gateway method. Source provenance is independent of the requested method. Complete integral timing within the common signal limits can prepare an effect; an unobserved RTP start or fractional duration remains observable but cannot synthesize a clamped or inferred tone. Raw digit events, packet metadata and application-supplied source strings grant no forwarding permission.

The WebRTC adapter connects Core’s authorized method to the existing owned SRTP writer, with an exact target route/peer fence outside Core/source custody. Every actual target write still obtains fresh independent source/recipient signal authority through the existing route policy. Original Core binding, actual source route/lifecycle/media generation or WebRTC track, and actual target IO custody surround each admitted poll. Locks are attempted synchronously and released on Pending. Cancellation, expiry and outcome uncertainty retain the existing owned-write rules; scheduling does not mint a new operation or imply remote receipt or playback.

The checker includes controlled Core queue/registration failures; actual SIP/SDP INFO and RTP input delivered through Core to signed local QUIC/WebTransport recipients; actual local SRTP input routed through Core to a second SRTP peer; stale subscriptions and ended sources; and the previous native, RTP and INFO routing regressions. Host journal and permission policy fixtures do not establish production Store/service/HTTP/SDK wiring. The remaining approved G6-024 media profiles and G6-028, Linux, remote/browser/provider profiles and Gate 6 acceptance remain separate qualification work.

G6-046 adds `SipAdapter::subscribe_managed_dtmf_source` for an exact managed SIP
controller and principal. The source registry attaches a sealed
`ReceivedDtmfDisclosure` before enqueueing the observation. It captures original
private groups and the live managed source incarnation without a permission
reader or native signing fields. A future durable Connect consumer must require
this disclosure, freeze the original signal, and supply fresh independent named
source/recipient authority for actual delivery.

Managed receive capture checks the physical input's monotonic receive/start time
against the latest committed scene audience or membership boundary. Delayed
input crossing a private transition, return, member join, removal or membership
ABA is rejected and counted as dropped. It cannot acquire the later audience.
The boundary is marked after the final mutation while membership custody is
still held, including a retirement delayed by another metadata lock. Busy
capture also counts as dropped; no queued source is later recaptured or silently
retried under a new audience.

`received_native_dtmf_audience_fence` combines the sealed original audience with
the actual native recipient scope during fresh route-policy acquisition. Each
IO poll owns the SIP source's current managed credential and the scene
registries. The native physical writer supplies its own credential custody, so
the audience fence never reenters that controller. Pending releases all scene
and optional source guards. Revoked/changed credentials, departed members and
original/current private audience mismatches refuse before the callback.

Controlled scene tests cover these custody and timing rules. A test-support
accessor also composes the actual configured Connect initial SIP call with the
managed source registry: real INFO and negotiated PT101/8kHz RFC4733 preserve
237ms timing, receive time, original physical observation ID and RTP details for
EdDSA/ES256. The parent fixture retains actual duplex audio thresholds and call
cleanup. This source-queue prerequisite does not install a durable Store ingress
worker, expose a received HTTP/SDK operation, or deliver SIP-originated tones to
native peers. Those integration and restart/privacy cases remain required.
