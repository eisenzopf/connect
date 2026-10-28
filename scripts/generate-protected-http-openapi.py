#!/usr/bin/env python3
"""Render protected issuance, Task access and raw artifact HTTP; no runtime proof."""
import argparse
import copy
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def obj(properties, required=None):
    return dict(type="object", properties=properties,
                required=list(properties) if required is None else required,
                additionalProperties=False)


def ref(name):
    return {"$ref": "#/components/schemas/" + name}


def enum_names(name):
    source = (ROOT / "crates/connect-protocol/src/tasks/models.rs").read_text()
    body = re.search(r"pub enum " + name + r" \{(.*?)\n\}", source, re.S)[1]
    names = re.findall(r"^    ([A-Z][A-Za-z0-9]*),$", body, re.M)
    assert names and len(set(names)) == len(names)
    return [re.sub(r"(?<!^)(?=[A-Z])", "_", value).lower() for value in names]


def render():
    ident = dict(type="string", pattern=r"^[A-Za-z0-9_.:-]{1,128}$")
    unix = dict(type="integer", minimum=1, maximum=9223372036854775807)
    constants = (ROOT / "crates/connect-protocol/src/tasks/mod.rs").read_text()
    scope_max = int(re.search(r"MAX_SCOPE_ITEMS: usize = (\d+)", constants)[1])
    ttl = int(re.search(r"MAX_GRANT_TTL_SECONDS: i64 = (\d+)", constants)[1])
    permissions = dict(type="array", minItems=1, maxItems=scope_max,
                       uniqueItems=True, items=dict(enum=enum_names("TaskPermission")))
    binding = obj(dict(kind=dict(enum=enum_names("InvocationKind")),
                       adapter=ident, external_id=ident))
    definition = obj(dict(task_id=ident, binding=ref("InvocationBinding"),
                          operations=permissions, expires_at=unix))
    definition["description"] = (
        f"Current immutable Task/binding permission subset; expiry must be later than "
        f"server now and no more than {ttl} seconds away, within current parent/Task expiry. "
        "Only native/host_session bindings may be registered by this local exchange. "
        "Provider kinds are parsed but refused without their verified adapters.")
    definition["x-max-ttl-seconds"] = ttl
    issue = obj(dict(action=dict(const="issue"), request_id=ident,
                     definition=ref("GrantDefinition")))
    revoke = obj(dict(action=dict(const="revoke"), grant_id=ident))
    grant = obj(dict(id=dict(type="string", format="uuid"), task_id=ident,
                     binding=ref("InvocationBinding"), operations=permissions,
                     expires_at=unix, revoked=dict(type="boolean")))
    protected = obj(dict(grant=ref("InvocationGrant"), bearer=dict(
        type="string", maxLength=256,
        pattern=r"^ctg1\.[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[0-9a-f]{64}$",
        description="Protected intended credential exchange only; never log, cache or place in model input.")))
    outcome = {"oneOf": [obj(dict(kind=dict(const="issued"), data=ref("ProtectedTaskGrant"))),
                          obj(dict(kind=dict(const="revoked"), data=obj(dict(grant_id=ident))))]}
    error = obj(dict(code=dict(type="string"), message=dict(type="string")))
    def envelope(profile, success=None):
        results = [obj(dict(Err=ref("Error")))]
        if success is not None:
            results.insert(0, obj(dict(Ok=ref(success))))
        return obj(dict(profile=dict(const=profile), result={"oneOf": results}))
    schemas = dict(
        Error=error, InvocationBinding=binding, GrantDefinition=definition,
        InvocationGrant=grant, ProtectedTaskGrant=protected, TaskAccessOutcome=outcome,
        TaskAccessRequest=obj(dict(profile=dict(const="connect-task-access/1"),
                                   command={"oneOf": [issue, revoke]})),
        TaskAccessResponse=envelope("connect-task-access/1", "TaskAccessOutcome"),
        ControlRejection=envelope("connect-control/1"),
    )
    # Reuse the existing public metadata shape, without importing unrelated
    # operation variants or projecting artifact bytes into JSON control results.
    attachments = json.loads((ROOT / "contracts/openapi-attachments.json").read_text())
    schemas["Artifact"] = copy.deepcopy(attachments["components"]["schemas"]["Artifact"])
    schemas["ArtifactOutcome"] = obj(dict(kind=dict(const="artifact"), data=ref("Artifact")))
    schemas["ArtifactWriteResponse"] = envelope("connect-control/1", "ArtifactOutcome")
    def json_response(description, schema):
        return dict(description=description, content={"application/json": dict(schema=ref(schema))})
    def text_response(description):
        return dict(description=description, content={"text/plain": dict(schema=dict(type="string"))})
    task_responses = {str(code): json_response(text, "TaskAccessResponse") for code, text in {
        200: "Issued/revoked result, or typed business rejection including invalid profile/intent/conflict. Inspect result, not only status.",
        401: "Missing, expired, revoked, malformed or model-grant bearer. Current ordinary backing credential required.",
        403: "Host/origin/provider-header or current Task/grant/permission authority denied.",
        503: "Capacity/storage unavailable or interrupted acceptance. Preserve the complete original issuance request.",
    }.items()}
    task_responses.update({str(code): text_response(text) for code, text in {
        400: "Framework JSON syntax rejection after admission.",
        413: "Framework body limit exceeds65536 bytes after admission.",
        415: "Framework JSON content type required after admission.",
        422: "Framework closed JSON shape/type/enum rejection after admission.",
    }.items()})
    task = dict(post=dict(
        operationId="protectedTaskAccess", security=[dict(bearerAuth=[])],
        description="Protected developer exchange on loopback HTTP or an explicit administration HTTPS surface, absent from webhook-only hosting. Shared capacity is reserved before Host/current ordinary bearer admission and JSON parsing. Host must match the configured HTTPS authority or the concrete loopback listener/localhost:port; Origin, X-Call-ID and X-Chat-ID refuse. Current task_grant, owner/membership/lifecycle and immutable subset/expiry checks apply inside issuance/replay. Same current issuer/request ID and unchanged intent recover the identical grant/secret/expiry; changed inputs conflict and revocation never refreshes authority. Local Native/HostSession binding registration and new grant commit atomically. Revoke retires only that grant. Authentication and admitted handler each have a five-second deadline; dropped IO does not roll back an already-running transaction. No automatic retry, renewal or model exposure.",
        requestBody=dict(required=True, content={"application/json": dict(schema=ref("TaskAccessRequest"))}),
        responses=task_responses,
        **{"x-max-request-bytes": 65536, "x-auth-deadline-seconds": 5,
           "x-handler-deadline-seconds": 5, "x-cache-control": "no-store",
           "x-automatic-retries": 0}))
    raw_ident = dict(type="string", pattern=r"^[A-Za-z0-9._~-]{1,128}$")
    issuance_unix = dict(type="integer", minimum=1, maximum=9007199254740991)
    operation = dict(type="string", pattern=r"^[A-Za-z0-9_.:-]{1,128}$",
                     **{"not": {"enum": ["admin", "credential.delegate"]}})
    issuance_operations = dict(type="array", minItems=1, maxItems=64, items=operation)
    issuance_recipients = dict(type="array", maxItems=128, items=raw_ident)
    schemas["CredentialIssueRequest"] = obj(dict(
        operation_id=raw_ident, identity_id=raw_ident, operations=issuance_operations,
        recipients=issuance_recipients, expires_at=issuance_unix))
    schemas["CredentialIssueRequest"]["description"] = (
        "Persist this complete original intent before IO. Operations and recipients are literal sets; "
        "ordering and duplicates normalize after raw item-count bounds. Expiry must be future, "
        "within365days of server now and no later than the current administrator expiry. "
        "No caller-selected issuer, installation, user assurance or inherited administrator scope.")
    issuance_fields = copy.deepcopy(schemas["CredentialIssueRequest"]["properties"])
    issuance_fields["operations"]["uniqueItems"] = True
    issuance_fields["recipients"]["uniqueItems"] = True
    issuance_fields.update({name:dict(type="string", format="uuid") for name in
                           ["credential_id", "issuer_identity_id", "issuer_credential_id", "installation_id"]})
    issuance_fields.update(issued_at=issuance_unix, credential=dict(
        type="string", pattern=r"^[0-9a-f]{64}$",
        description="Intended protected exchange only; do not log or expose to model input."))
    schemas["CredentialIssueOutcome"] = obj(issuance_fields)
    schemas["CredentialIssueResponse"] = envelope("connect/2", "CredentialIssueOutcome")
    issuance_headers = {name:dict(schema=dict(const=value)) for name,value in
                        [("Cache-Control","no-store"),("Pragma","no-cache"),("Connection","close")]}
    issuance_responses = {str(code):{**json_response(text,"CredentialIssueResponse"),
                                     "headers":copy.deepcopy(issuance_headers)} for code,text in {
        200:"Current original independently scoped credential. Secret delivery ends at admission deadline, credential/issuer expiry, cancellation or socket close.",
        400:"Invalid JSON/content type/closed shape/bounded intent, or unsupported HTTP version/upgrade.",
        401:"Missing/malformed/duplicate Authorization, Task/model grant, expired or revoked ordinary credential.",
        403:"Host/origin/provider-header refusal, missing owned socket custody, nonadministrator or current tenant/scope/recipient/expiry ceiling denied.",
        409:"Changed original intent/issuer or operation ID reused by another command.",
        410:"Original receipt tombstoned; no replacement credential is minted.",
        413:"JSON body exceeds65536 bytes after administrator admission.",
        429:"Shared response/admission or durable receipt capacity exhausted.",
        503:"Storage/authority unavailable, incomplete reconciliation or interrupted observation; preserve the original intent and issuer.",
    }.items()}
    issuance = dict(post=dict(operationId="issueScopedCredential",security=[dict(bearerAuth=[])],
        description="Protected administrator exchange on loopback HTTP or an explicit administration HTTPS surface, absent from webhook-only hosting. Requires closing HTTP/1.1 with owned socket custody. Reserve shared capacity, validate Host matching configured HTTPS authority or the concrete loopback listener/localhost:port, refuse Origin/X-Call-ID/X-Chat-ID/Upgrade, and authenticate current ordinary administrator before JSON parsing. Admission and handler share one five-second absolute deadline. Current issuer/target/recipient fences remain owned through physical response IO, even after Body EOS. Exact replay with original current issuer and normalized intent returns the same credential ID/secret/issued time/absolute expiry. Changed authority or revoked/expired child never issues a replacement. A running transaction may commit after observation ends; explicit replay reconciles. The child is independent administrative authority, without human-consent claims or parent-revocation chaining. Native Device/signing-key enrollment is separate; no automatic retry or renewal.",
        requestBody=dict(required=True,content={"application/json":dict(schema=ref("CredentialIssueRequest"))}),
        responses=issuance_responses,
        **{"x-max-request-bytes":65536,"x-total-deadline-seconds":5,"x-automatic-retries":0,
           "x-response-custody":"Physical closing HTTP/1.1 socket; shared capacity retained with authority until IO ends."}))
    path_parameters = [dict(name=name, **{"in": "path"}, required=True, schema=raw_ident)
                       for name in ["conversation_id", "artifact_id"]]
    def query(name, schema):
        return dict(name=name, **{"in": "query"}, required=True, schema=schema)
    offset = dict(type="integer", minimum=0, maximum=18446744073709551615)
    limit = dict(type="integer", minimum=1, maximum=4294967295,
                 description="Must also be no larger than configured limits.artifact_chunk_bytes (default32768).")
    common = {"401": json_response("Current ordinary bearer required; Task grants use their explicit Task API.", "ControlRejection"),
              "403": json_response("Current source/recipient/owner authority, readiness or artifact lifetime denied.", "ControlRejection"),
              "503": json_response("Current authority, storage or capacity unavailable.", "ControlRejection")}
    headers = {
        "Cache-Control": dict(schema=dict(const="no-store")),
        "X-Content-Type-Options": dict(schema=dict(const="nosniff")),
        "X-Connect-Offset": dict(schema=offset),
        "X-Connect-Eof": dict(schema=dict(enum=["true", "false"])),
    }
    get_responses = {**common,
        "200": dict(description="Exact bounded byte window; headers retain offset and EOF, not JSON/base64.",
                    headers=headers, content={"application/octet-stream": dict(schema=dict(type="string", format="binary"))}),
        "400": dict(description="Invalid configured range or framework query parsing; inspect content type.",
                    content={"application/json": dict(schema=ref("ControlRejection")),
                             "text/plain": dict(schema=dict(type="string"))})}
    put_responses = {**common,
        "200": json_response("Artifact metadata or typed business rejection. Some late authority/storage rejections retain HTTP200; always inspect result.", "ArtifactWriteResponse"),
        "400": dict(description="Empty/oversized configured chunk, invalid range, changed original intent or framework query rejection.",
                    content={"application/json": dict(schema=ref("ControlRejection")),
                             "text/plain": dict(schema=dict(type="string"))}),
        "413": text_response("Framework aggregate body limit: max(configured artifact chunk bytes,65536).")}
    raw = {
        "get": dict(operationId="downloadArtifactBytes", security=[dict(bearerAuth=[])],
                    description="Ordinary current artifact_get and original source/recipient authorization. Artifact must be ready/current; offset cannot exceed size. No new visibility, delegation, reference or custody is created.",
                    parameters=path_parameters + [query("offset", offset), query("limit", limit)], responses=get_responses, **{"x-closed-query":True}),
        "put": dict(operationId="appendArtifactBytes", security=[dict(bearerAuth=[])],
                    description="Ordinary current original owner and artifact_append authority. The body is opaque bytes; query parsing/body collection and configured byte-size checks precede bearer authentication. Save request_id, offset and exact bytes before IO; exact replay appends once and changed intent refuses. A successful metadata result is not final artifact readiness; finalize with the separate named ArtifactFinish API.",
                    parameters=path_parameters + [query("request_id", raw_ident), query("offset", offset)],
                    requestBody=dict(required=True, content={"application/octet-stream": dict(schema=dict(type="string", format="binary", minLength=1))}),
                    responses=put_responses, **{"x-closed-query":True}),
        "x-configured-chunk-limit": "limits.artifact_chunk_bytes; default32768",
        "x-automatic-retries": 0,
    }
    return json.dumps(dict(openapi="3.1.0", info=dict(
        title="Connect protected issuance, Task access and raw artifact HTTP", version="connect/2",
        description="Current local application listener extensions. Task-access wire profile is connect-task-access/1; artifact control rejections/metadata use connect-control/1 and downloads use raw bytes. Separate canonical/provisioning/attachment-link/record references retain their own contracts. Generation is not runtime/profile qualification."),
        paths={"/v2/credentials": issuance, "/v1/task-access": task,
               "/v1/conversations/{conversation_id}/artifacts/{artifact_id}": raw},
        components=dict(securitySchemes=dict(bearerAuth=dict(type="http", scheme="bearer")), schemas=schemas)), indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = ROOT / "contracts/openapi-protected-http.json"
    content = render()
    if args.check:
        assert path.read_text() == content, "Protected HTTP reference is stale"
    else:
        path.write_text(content)


if __name__ == "__main__":
    main()
