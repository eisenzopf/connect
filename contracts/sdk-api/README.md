# Shared SDK contract proofs (G6-059)

These declarations and complete application outlines describe the target public
SDK surface. They are compile-only contract proofs, not transports, mock clients,
installed packages, or evidence of SDK runtime parity. The final method/result
review remains in progress in `contracts/sdk-gate-6-api.json` and
`docs/SDK_COMPATIBILITY.md`.

Each language expresses private send with separate delivery observations,
consultation, handoff, device move, status-first recovery, incoming invitations,
local wait cancellation versus a durable cancellation decision, capability
refusal, journal resumption, consultation return, and client/reader cleanup.
Existing public handles and saved application intent are explicit inputs. No
outline requires a Store, private route identifier, or provider API.

Mutations asynchronously return observable Operations. Explicit bounded `wait`
returns the typed completion. `waitUntilReady` observes operation state; it does
not prove decoded media. Stable logical mutation IDs are saved before I/O.
Recovery reconstructs the original Operation and queries status without another
mutation. Session, topology, conversation, and stream revisions are distinct.
Trusted provisioning is a separate administrative capability.

Run `bash scripts/prepare-dependencies.sh` first. Then, using a verification
Python with `mypy==2.4.0` installed:

```sh
python scripts/check-gate6-sdk-outlines.py --output .local/gate-6/sdk-contract/outlines-new
```

The checker uses the repository's TypeScript compiler and Rust compiler, checks
browser and Node environments separately, targets Python 3.12, and requires six
unsuppressed negative diagnostics in each compiler target. Negative cases reject
mixed resource IDs, wrong revision families, two device-move targets, an empty
private audience, participant-client provisioning, and a message reference used
as a move Operation. Inputs and logs are hashed; private outputs stay ignored.
