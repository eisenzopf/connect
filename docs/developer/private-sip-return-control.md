# Private SIP candidate return control

A return closes its recorded original SIP candidate under a distinct control
projection. The candidate's `connection.offer`, `stream.subscribe` and codec
preferences do not authorize closure and their withdrawal does not prevent it.
Current route dialing authority, credentials, Endpoint/Device, member epoch,
physical incarnation, exact captured root/child/call and fresh return authority
remain required while the candidate is physically present.

The closure purpose follows that exact candidate through snapshot comparison,
whole-scene scope derivation and final reader construction. Every other SIP
source keeps its media checks. A public or established SIP origin cannot stand
in for the private candidate. Its closure reader always has `may_receive=false`,
and the return exposes no media send or receive edge involving the candidate.
Returning native programmes still need their own current authority.

Captured return application closes the private packet context. Physical cleanup
and completion still require the original call's recorded absence; neither a
return receipt nor withdrawn media rights prove termination. Store tests cover
withdrawal, recovery boundaries, exact selection, uncertainty, replay, physical
absence and sibling preservation using controlled observations. Actual host
handoff, SIP media/INFO/RFC4733 and return qualification remain open in G6-046.
