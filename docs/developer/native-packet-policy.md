# Current packet policy under owned file custody

`Store::try_native_media_read` captures current native source/target facts and
acquires their complete current authority footprint. The SQL version seal rejects
concurrent mutation, and the resulting reader owns the file fences through the
bounded local media action.

`Store::try_native_media_read_under(target, source, held)` supports protected
composition within an active packet's existing reader. It checks that the reader
is current and belongs to this exact Store namespace and runtime owner. It then
derives the requested pair's current facts using the same SQL-versioned metadata
rules and requires coverage of every current scope. Foreign, incomplete or
expired custody is refused without acquiring replacement readers.

Only the file interval is shared. The holder's participant, credentials,
permissions, audience or stream binding do not become the recipient's authority.
The pair still checks its own current source/target credentials, signing keys,
Devices, Endpoints, membership epochs, Connection generations, recipient ceilings
and subscription permission. Its expiry cannot exceed either its current facts
or the original holder's expiry. The SQL mutex is released before returning.

The caller must retain the exact frame's route/subscription guards, cancellation
and original 200 ms age bound. This API performs no media IO and supplies no
permission for queued work to reuse an earlier authorization. Supported writers
remain fenced until the last owned reader drops; composition can finish under
that same interval without entering a new reader behind a waiting writer.

The [component report](../evidence/gate-6/auxiliary-native-packet-reader.json)
records Store-level behavior. Core/Service packet integration and actual 50-peer
qualification are pending at that checkpoint. The canonical HTTP wire contract
is unchanged; see its [OpenAPI reference](../../contracts/openapi-canonical-http.json).
