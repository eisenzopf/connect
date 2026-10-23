# Explicit tenant vCon readers

The current implementation adds a tenant record-reader role through the protected
provisioning API and `ApplicationClient::provision`. Qualification is recorded
separately; this document does not qualify all G6-046 behavior.

`POST /v2/identities/{id}/record-read-role` requires current administrator authority,
the same tenant and a permitted target Identity. Its body target must agree with
the route. The [OpenAPI reference](../../contracts/openapi-provisioning.json)
defines the closed request and response. The provisioning CLI accepts the same
saved command on standard input.

```json
{
  "type": "record_read_role",
  "operation_id": "grant-auditor-001",
  "identity_id": "id_auditor",
  "role": "auditor",
  "private_sources": false,
  "active": true,
  "expected_revision": null
}
```

Both booleans are explicit. Omit/null `expected_revision` only for the first
assignment; changes require the current revision. Setting `active:false` revokes
the role without removing its revision history. Retry the original complete
command and operation ID after uncertainty. As with other provisioning commands,
replay reports current authorized state and cannot reapply an old active grant.

A current role and a credential with `record_get` allow an Identity to fetch a
known Conversation/Session/record selector without joining that Conversation.
No Participant or Session appearance is manufactured. Ordinary `record_get`
credentials still need their original participant/history authority when no
active tenant role exists. A role cannot cross the Identity's tenant.

The role's `private_sources` policy explicitly controls Participant-restricted
sources. When false, those sources remain hidden. When true, the role can select
those sources only within the credential's independent recipient ceiling.
Metadata/content expiry, unavailable states, canonical validation and original
record scope still apply. The current supported producer remains the metadata
outcome profile; this setting does not enable media capture or provider access.

Core and Connect carry a typed Participant-or-tenant-role viewer. It includes the
Participant membership epoch or tenant role revision and policy. Signed native,
bearer HTTP and issued TLS references use that exact binding. Changing a role
invalidates its old role-derived issued views, including after the role is
reactivated. Shared-only grants preserve eligible ordinary participant rights;
views admitted by that independent authority retain their membership binding. Resolving
an old URL as a new fetch selector still requires a completely new current read;
the URL itself never grants access.

An Identity read fence stays held through actual response IO. Provisioning a role
change acquires that Identity's writer, so revocation cannot pass an in-flight
read. Record bytes and exact revision custody are retained independently. Queued
writers do not force the existing read to reacquire another policy reader.

A tenant read role does not authorize redaction/deletion. Nonadministrative
erasure still requires actual participant/source authority and its own action
permission; installation erasure retains its independent administrative grant.
Record-to-provider context delivery, captured-media policies, named remote lanes
and full Gate 6 qualification remain separate work.
