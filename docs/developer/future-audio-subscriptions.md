# Audio intent before negotiated audio (Gate 6 in progress)

An admitted scene member can retain an audio subscription before either endpoint
has an audio port. The scene owns its stable audio consumer and selection gate;
the child mixer adopts that same gate when audio is added. Selecting intent does
not allocate a port, consume a transport handle, change a speaking role or grant
publication. A receiver without an audio port still needs current audio receive
permission and a supported common output format.

Participant selection pins the original Identity, membership epoch and principal
owner. It covers later selected audio and other admitted devices of that same
owner. Exact stream selection still requires a current publishing logical stream
and retains its original controller incarnation. Subscription revisions, bounded
program size, per-stream exclusions, send drains and cancellation rules continue
to apply. Clear and unsubscribe also work while audio is absent.

For an owner without publishing audio, the engine supplies a
`MediaParticipantSource` to the installed policy. It identifies the admitted
owner, current credential, membership epoch and requested medium. It does not
invent a logical stream or transport descriptor. `current_participant_receive`
must validate current source membership and recipient audience authorization.
The held path uses `control_participant_receive` under the same invocation guard.
Both hooks default to unsupported. There is no fallback to a fabricated stream,
a generic control grant or a fresh store reader behind a queued writer.

Selection validates current authority before and after draining the previous
routing generation. No grant remains in the saved program. Audio publication and
each contributor's delivery still acquire current media authority and retain the
existing role, scene, transport and audience guards through local IO. Revocation
after selecting intent can therefore prevent later audio. Removal/readmission
cannot transfer a retained selector to a different membership epoch.

The qualification runner is `scripts/check-gate6-future-audio.py`. It records
source hashes, required behavior cases, private logs and compiled test binaries.
Core cases include selection before both audio ports, later exact-source receive,
explicit exclusions, revocation, removed membership, unsupported policy and
receiver capability refusal. The held case runs with an actual queued authority
writer. Signed local QUIC and WebTransport tests install audio after selection,
add a later listener, reselect the microphone and preserve selected video and
physical connection quotas.

This is local macOS arm64 qualification with controlled media/readiness policy
and the trusted participant SPI. Public durable subscription and negotiation
handlers, production Store media policy, general non-audio Participant selectors,
native reliable data through the approved canonical message/held writer profile,
other guarded transports and the remaining Gate 6 tasks are still open. These
checks do not qualify video rendering, remote providers or publication.
