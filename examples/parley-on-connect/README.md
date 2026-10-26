# Parley reference scenario

G6-071 defines the reusable example in [scenario.json](scenario.json). It has
seventeen stages, explicit audiences, business gates, observable outcomes and
public SDK target-contract mappings. The owner, assistant, booker and organizer
are required; the observer is optional. Synthetic travel facts and approval
policy belong to this example.

The scenario follows the [October 13 Parley implementation status](https://github.com/eisenzopf/parley/blob/b2984928916a3b6ccca5c501457e509cd7a50df7/docs/CONFERENCE_IMPLEMENTATION_STATUS.md)
and its [task-content profile](https://github.com/eisenzopf/parley/blob/b2984928916a3b6ccca5c501457e509cd7a50df7/docs/UCTP_CONFERENCE_PROFILE.md#L210).
The prepared upstream checkout is older; no upstream files were edited. The
example uses canonical Connect operations instead of importing the older
application-specific wire dialect.

The owner opens a browser conversation with the assistant. After the closing
speech and authoritative Session end, the assistant gathers an itinerary from
the booker. An attributed affirmative reply to the organizer's call-permission
question permits the organizer invitation. Private consultation holds the same
organizer leg silent. Owner consent and an assistant action are both required
before takeover. A proposal requires an attributed owner decision for that exact
proposal before four separate follow-ups to the selected people.

The later Parley demo moves to the phone on answer/readiness. This Gate 6 example
selects the approved configured DTMF `1` profile, so wrong-digit and no-confirmation
failures can be rehearsed. Answer alone cannot replace confirmation. The policy
and stage graph are identical in fixture and live modes; only model, telephone,
SMS and contact bindings change. Binding a live definition does not activate a
provider. Named live rehearsals require their G6-093 configuration.

Run definition checks with:

```sh
node --test examples/parley-on-connect/tests/scenario.test.mjs
```

The checks validate every stage against `contracts/sdk-gate-6-api.json` and the
canonical envelope schema. Historical context uses the public validated raw
`message.history` interface. The scripted assistant always labels its output
`SIMULATED`; its output cannot become an owner decision.

This checkpoint verifies a definition and model double. It does not execute an
installed SDK application, launch a model, contact a provider, or qualify the
full browser/telephone journey. Transport fixture verification and the later
G6-072–G6-080 application work remain explicit follow-up work.
