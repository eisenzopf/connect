# Local record redaction and deletion

G6-046 implementation is in progress. `POST /v2/records/{uuid}/mutations` is a
named HTTP application API with the [OpenAPI reference](../../contracts/openapi-record-mutations.json).
It accepts an original `operation_id`, Conversation/Session/record selectors,
`expected_revision` and a `redact` or `delete` action. Obtain UUID/revision from
public record discovery. Full canonical or manifest hashes are not public inputs.
The Rust `ApplicationClient::mutate_record` method uses this explicit endpoint.

Redaction takes up to128 semantic targets and a `content` or `metadata` mode.
Targets identify the document, Session outcome, capture diagnostics, a Participant
with its recorded membership epoch, or a Connection with its recorded generation.
These identities appear in authorized record content. Internal manifest indexes
are resolved only under the original current source and record guards. Unknown
and unauthorized targets have the same refusal. Core applies canonical
transformation and dependency propagation; actual changed sources must also be
authorized. Connection lineage retains an explicit dependency on its matching
Participant appearance.

Ordinary callers need explicit `record_redact` or `record_delete`, `record_get`,
current Conversation/Session access and the applicable frozen original source
epoch or current explicit Session history share. Recipient ceilings and private
Connection ownership remain enforced. These mutation capabilities are absent from
default collaboration credentials. Installation erasure requires independent
current `admin` and `record_admin` capabilities plus the action permission;
operator wildcard credentials qualify. Fetch authority remains independently
checked. No role label or assistant prompt supplies these capabilities.

Save and retry the complete original request after an uncertain response. Current
credentials of the same original Identity can recover its exact receipt after
later redaction, deletion or restart. Changed payload, expected revision, source
order or actor conflicts under the original logical ID. Narrower/revoked
credentials cannot borrow old authority. Private durable replay facts retain the
original subject and source-group constraints after content erasure. They remain
protected local state, and are not serialized as client grants.

Canonical bytes, original mutation fact, replay authority facts and current
record availability journal hints commit in one transaction. Capacity/validation
or authorization failure rolls everything back. Replay adds no journal entry and
performs no canonical transformation. New snapshots and current journal
projection suppress stale handles; all future reads authenticate again. Deletion
removes live body/manifest access while retaining an audience-safe tombstone and
original mutation facts. `remote_deletion: "not-attempted"` distinguishes this
local result from provider erasure. No physical disk sanitization is asserted.

Authentication precedes the closed64KiB JSON body. Responses are `no-store` and
bounded by a five-second transport deadline; interruption can leave acceptance
unknown, so reuse the logical ID. Source/record guards survive any owned blocking
transaction and current expiry is checked before commit. This local profile does
not qualify remote hosting, captured media, retention workers or full Gate6
acceptance by itself.
