# Private SIP candidate packet authority

`Store::try_private_initial_telephone_media_read` acquires the original private
call's complete current context. It requires a recorded, confirmed audience
application in this runtime, or its sealed private lifetime. Admission and
planning can prepare the muted call but cannot obtain this packet reader.

The pending context still requires the original controller's current execution
authority. The sealed lifetime survives that controller ending. Both require
the captured call, Endpoint and Device, original member epochs and native
signing credentials, reciprocal private pair, current media rights and recipient
ceilings. A captured pending signal or packet context for an AI candidate also
requires live provider readiness; preparation alone does not supply it.

`try_initial_telephone_native_read` selects this protected reader for a private
SIP origin, and accepts only its original native partner. Held remotes and public
siblings cannot borrow the pair. The read rechecks complete scope coverage inside
the owned interval, and its final drop releases the authority guards. It never
adds the private candidate to the public ready-origin registry. Ordinary initial
SIP media readers continue to reject private origins. Captured return application
closes the packet context before the original physical call is cleaned up.

The Store tests cover these authority boundaries with controlled observations.
They do not qualify actual RTP or enable runtime private SIP-candidate selection.
Host lifetime handoff, private return control, actual media/INFO/RFC4733 and the
full G6-046 matrix remain required. Public command behavior remains documented
in the [server HTTP reference](canonical-http.md).
