# Private consultation and remote hold (G6-021)

The managed rvoip Session owns reusable audio audience changes. This is the
upstream media engine contract; durable Connect operations, HTTP/SDK methods,
private message/history projection and application decisions belong to their
later Gate 6 tasks. Local media outcomes are not journal receipts or presence.

## Prepare a private leg

Create the consulting Connection through the existing bounded physical admission
path, negotiate actual resources, mute its publishing audio, then use
`admit_private_peer_session_connection_with_held`. The supplied policy must
explicitly approve `PreparePrivateAudio` for the exact operation, revision,
Connection, owner and membership epoch using its actual current domain grant.
The private admission accepts audio ports only. It refuses unmuted publishing
sources before taking their inputs. Unsupported or unready resources do not
switch the original speaking path.

Private admission installs a one-Connection quarantine before activating its
source. Existing and later receivers inherit that restriction. A shared
Participant or credential does not join an old public AI Context to the new
private Context. An ordinary unmute or subscription cannot bypass the quarantine.
Admission and audience changes preserve the retained Session and remote physical
Connection; neither creates a replacement call.

## Switch the intended bridge

`plan_private_consultation` names the primary, retained remote, consulting
Connection and optional admitted hold source. No hold source means silence.
The plan replaces only those three receiver programmes: primary and consultant
hear each other; the remote hears the hold source. A common closed-audience rule
also fences unrelated receivers and later admissions. Their unrelated public
routes remain available. The lower-level `AudioAudienceRequest` supports other
explicit closed audio audiences and complete receiver programmes.

`start_private_consultation_with_held` requires fresh approval of the actual
scene-issued audience plan. Core freezes the participating controllers, stages
all programmes and the source unmute, drains admitted sends, revalidates every
target under the same held grant and publishes all changes together. It never
clears and subscribes through separate public mutations. Stale revisions,
foreign plans, policy denial, deadline, cancellation or dropping a pending drain
leave the original programmes and revisions intact. Queued frames from the old
audience cannot cross a successful switch.

A broad Participant selection remains subject to the receiver's closed audience
on every frame. Restoration can resume an allowed public device while another
device of the same Participant remains quarantined. Exact prohibited source
requests fail explicitly.

## Observe audio and recover

`wait_private_consultation_audio` observes successfully decoded, freshly admitted
audio from the exact consulting Connection after the committed switch. It
returns `DecodedAudioObserved`, `MediaTimeout`, `Cancelled`, `PrimaryEnded`,
`RemoteEnded`, `ConsultantEnded` or `AlreadyRestored`. It retains no permission
reader while waiting. Neither resource readiness nor activity on an older
public AI Connection satisfies this observation. This result alone does not
assert `connection.ready` or Participant presence.

After a timeout, cancellation, failed preparation or completion, obtain a new
restore plan from the actual handle and apply it with fresh current authority.
Restoration reports departed Connections, restores only surviving original
programmes and preserves their original owner/controller pins. It cannot lend
an old private or public programme to a replacement membership epoch. A denied
restore leaves the private audience closed; the host can retry under new current
authority. Handles contain metadata, not cached authorization.

Restoration mutes the consulting source and leaves its exact old private Context
quarantined while the original public bridge resumes. Provider cleanup may be
uncertain; it must not make that Context public. Remove the consulting Connection
through the existing owned lifecycle after cleanup. External cleanup is a
separate outcome and is never represented as a successful rollback of external
side effects. Business approval, prompts and Context content stay in the host.

## Retain native resource confirmation

Every successful audience commit issues an opaque `AudioAudienceCommit` through
its handle. The native adapter's `observe_audio_audience_commit` checks the exact
installed controller, credential, committed revision and unchanged physical
ports, then carries forward an existing signed resource confirmation. Public
outcome fields cannot manufacture this proof. Repeated observation is safe;
foreign controllers and stale commits fail. It creates no new wire confirmation,
media flow observation or permission. Hosts must present the proof to each
involved native adapter before subsequent native reconciliation.

## Evidence boundary

The qualification driver is `scripts/check-gate6-private-consult.py`. Its report
records actual executed cases, source hashes and executable hashes. The default
lane uses isolated policy/adapter fixtures and local TLS native endpoints.
Decoded speaker markers use distinct owners in the Core scene. The native
confirmation fixture uses two endpoints of one Participant and retains MCU
self-echo suppression; it does not independently prove distinct-owner private
audio. Linux x86_64, remote/native/browser acceptance, named provider rehearsals
with G6-093 configuration, full Gate 6 acceptance and publication are separate
requirements.
