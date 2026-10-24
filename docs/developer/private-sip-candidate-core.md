# Core private outbound admission

The recorded upstream patch
`patches/rvoip/gate6-private-initial-outbound-admission.patch` adds private audio
admission to the actual provisional outbound SIP ticket. It creates no native
peer admission, signing key or replacement connection.

Call `activate_private_initial_participant_with_held` on the original
`PreparedOutboundConnection`. Signaling activation retains the same ticket and
Session slot, and publishes no Participant or source. The returned
`ActivatedInitialParticipant` seals private mode and the original actor, target
membership and Endpoint. Its commit method permits a fresh Session revision,
while refusing changed identities, membership epochs, roles or physical
Endpoint relationships. Pins are checked after activation and before final
media preparation. They retain metadata, not an authorization reader.

The final callback receives the actual `OutboundParticipantPreparation`.
`is_private_audio` reports its frozen mode without granting authority. Construct
the adapter's controller from that exact preparation and its real negotiated
ports, then mute publishing audio. The managed admission path requires fresh
`PreparePrivateAudio` authority for the operation and controller revision and
uses the existing one-Connection quarantine before activating the source.
Unmuted ports or denied current authority cannot fall through to public
admission. Failed, cancelled or dropped work retains the original supervisor's
cleanup ownership and cannot allocate a second call.

Public initial activation retains public mode. It does not acquire private
preparation authority merely because it uses the same implementation helper.
All ordinary current participant, media, lifecycle and Endpoint checks remain
in effect.

The [component report](../evidence/gate-6/auxiliary-private-sip-candidate-core.json)
records controlled Core ticket and guarded-port evidence and the reproduced
ringing rebind failures. These fixtures do not establish SIP wire delivery or a
completed Connect private candidate. The
[Connect call-custody boundary](private-sip-candidate-initial.md) remains
separate: its host still needs private answer acceptance/readiness, this Core
path, typed candidate media/DTMF, return and physical cleanup. The
[private consultation contract](../../contracts/private-consultation.md) defines
the shared engine behavior. No new public HTTP method is introduced; the
[server HTTP reference](canonical-http.md) retains the existing methods.

Source-lock patch 184 must be reproduced with `scripts/prepare-dependencies.sh`.
When an existing preparation has a different lock, preserve it and prepare a
fresh dependency directory as instructed by that script. Generated source
changes are not an alternative to the recorded patch and digest.
