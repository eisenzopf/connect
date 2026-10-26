#!/usr/bin/env python3
"""Check actual protected HTTP exchanges against the current OpenAPI reference.

Uses a supplied sealed CLI and matching build evidence; creates only isolated
loopback state. Grant-bearing wire samples stay in ignored private .local state.
"""
import argparse
import copy
import datetime
import hashlib
import http.client
import importlib.metadata
import json
import os
import platform
from pathlib import Path
import signal
import socket
import subprocess
import time
import tomllib
import urllib.parse
import uuid

from jsonschema import Draft202012Validator, FormatChecker

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--build-summary", required=True, type=Path)
    parser.add_argument("--build-inputs", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--application-contract", action="store_true",
                        help="Also qualify the complete current operation-body/dispatch reference")
    args = parser.parse_args()
    os.umask(0o077)
    cli = args.executable.resolve()
    build = json.loads(args.build_summary.read_text())
    digest = sha(cli)
    assert cli.stat().st_size > 1_000_000 and os.access(cli, os.X_OK)
    assert build["exit_code"] == 0 and build["source_unchanged"]
    assert any(isinstance(v, dict) and v.get("sha256") == digest
               for k, v in build.items() if k.startswith("canonical-cli-"))
    inputs = json.loads(args.build_inputs.read_text())
    assert all(sha(ROOT / p) == h for p, h in inputs.items()), "build source differs"
    source_names = ["scripts/check-gate6-protected-http.py", "scripts/generate-protected-http-openapi.py",
                    "contracts/openapi-protected-http.json", "contracts/openapi-attachments.json",
                    "contracts/openapi-canonical-http.json", "contracts/openapi-provisioning.json"]
    if args.application_contract:
        source_names += ["scripts/generate-application-http-openapi.py", "scripts/lib/application_http_cases.py",
                         "contracts/openapi-application-http.json", "contracts/application-http-dispatch.json"]
    source_guard = {p: sha(ROOT / p) for p in source_names}
    subprocess.run(["python3", "scripts/generate-protected-http-openapi.py", "--check"], cwd=ROOT, check=True)
    directory = args.output or ROOT / ".local/gate-6/protected-http" / datetime.datetime.now(
        datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    directory = directory.resolve()
    assert directory.is_relative_to((ROOT / ".local").resolve()), "wire evidence must remain private"
    directory.mkdir(parents=True, exist_ok=False, mode=0o700)
    samples = directory / "samples"
    samples.mkdir(mode=0o700)
    documents = {name: json.loads((ROOT / path).read_text()) for name, path in {
        "protected": "contracts/openapi-protected-http.json",
        "attachment": "contracts/openapi-attachments.json",
        "canonical": "contracts/openapi-canonical-http.json",
        "provisioning": "contracts/openapi-provisioning.json",
    }.items()}
    if args.application_contract:
        documents["application"] = json.loads((ROOT / "contracts/openapi-application-http.json").read_text())
    for document in documents.values():
        for schema in document["components"]["schemas"].values():
            Draft202012Validator.check_schema(schema)

    def validate(document, schema, value, case):
        errors = list(Draft202012Validator({**schema, "components": document["components"]},
                                          format_checker=FormatChecker()).iter_errors(value))
        # Schema failures must never print the protected intended grant response.
        assert not errors, (case, [(list(e.absolute_path), e.validator) for e in errors])

    def free(kind):
        with socket.socket(socket.AF_INET, kind) as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    native_port, http_port, mcp_port = free(socket.SOCK_DGRAM), free(socket.SOCK_STREAM), free(socket.SOCK_STREAM)
    state = directory / "state"
    child_env = {k:v for k,v in os.environ.items() if not k.startswith(("VAPI_","TELNYX_","OPENAI_","ANTHROPIC_","TWILIO_","CONNECT_TEST_","CONNECT_GATE6_"))}
    initialized = subprocess.run([str(cli), "init", "--directory", str(state), "--example", "review-loop",
                                  "--bind", f"127.0.0.1:{native_port}", "--http-bind", f"127.0.0.1:{http_port}"],
                                 cwd=ROOT, env=child_env, capture_output=True, timeout=20)
    (directory / "init.log").write_bytes(initialized.stdout + initialized.stderr)
    assert initialized.returncode == 0
    server_config = state / "server.toml"
    server_config.write_text(server_config.read_text() + f'\n[mcp]\nbind = "127.0.0.1:{mcp_port}"\n')
    operator = tomllib.loads((state / "operator.toml").read_text())
    reviews = tomllib.loads((state / "reviews.toml").read_text())
    agent = tomllib.loads((state / "agent.toml").read_text())
    rows = []
    child = None
    summary = dict(status="incomplete", cli_sha256=digest, build_inputs_sha256=sha(args.build_inputs),
                   jsonschema_version=importlib.metadata.version("jsonschema"),
                   platform=dict(system=platform.system(),machine=platform.machine(),python=platform.python_version()),
                   bootstrap_signing_algorithm="EdDSA; unsigned HTTP is not physical signing proof",cases=rows)

    def exchange(case, method, path, token, body=None, raw=None, headers=(),
                 contract=None, request_valid=True, expected=None):
        payload = raw if raw is not None else (json.dumps(body).encode() if body is not None else b"")
        connection = http.client.HTTPConnection("127.0.0.1", http_port, timeout=5)
        try:
            connection.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
            selected = [("Host", f"127.0.0.1:{http_port}"), ("Connection", "close")]
            if token is not None:
                selected.append(("Authorization", "Bearer " + token))
            if not any(k.lower() == "content-type" for k, _ in headers):
                selected.append(("Content-Type", "application/json" if raw is None else "application/octet-stream"))
            selected.extend(headers)
            for key, value in selected:
                connection.putheader(key, value)
            connection.putheader("Content-Length", str(len(payload)))
            connection.endheaders(payload)
            response = connection.getresponse()
            wire = response.read(2 * 1024 * 1024 + 1)
            assert len(wire) <= 2 * 1024 * 1024, case
            status, response_headers = response.status, dict(response.getheaders())
        finally:
            connection.close()
        mime = next((v.split(";", 1)[0].lower() for k, v in response_headers.items()
                     if k.lower() == "content-type"), "")
        parsed = json.loads(wire) if mime == "application/json" else None
        sample = dict(case=case, method=method, path=path,
                      request=body, request_bytes=len(payload), request_schema_valid=request_valid,
                      request_bytes_hex=payload.hex() if raw is not None else None,
                      request_header_names=[k for k, _ in selected], openapi_contract=contract,
                      status=status, response_headers=response_headers,
                      response=parsed if parsed is not None else None,
                      response_bytes_hex=None if parsed is not None else wire.hex())
        sample_path = samples / f"{len(rows):03d}-{case}.json"
        sample_path.write_text(json.dumps(sample, indent=2) + "\n")
        rows.append(dict(case=case, status=status, content_type=mime,
                         sample=str(sample_path.relative_to(ROOT)), sha256=sha(sample_path), schema_checked=contract is not None))
        (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        if expected is not None:
            assert status == expected, (case, status, expected)
        if contract:
            doc_name, route = contract
            document = documents[doc_name]
            operation = document["paths"][route][method.lower()]
            request_errors = []
            if "requestBody" in operation:
                content_type = next(v.split(";",1)[0].lower() for k,v in selected if k.lower()=="content-type")
                schema = operation["requestBody"]["content"].get(content_type,{}).get("schema")
                if schema is None:
                    request_errors.append("content-type")
                else:
                    try:
                        value = json.loads(payload) if content_type=="application/json" else payload.decode("latin1")
                        request_errors.extend(Draft202012Validator({**schema,"components":document["components"]}).iter_errors(value))
                    except (ValueError,UnicodeError):
                        request_errors.append("syntax")
            actual_query = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query,keep_blank_values=True)
            query_specs = {p["name"]:p for p in operation.get("parameters",[]) if p["in"]=="query"}
            if operation.get("x-closed-query") and set(actual_query)-set(query_specs):
                request_errors.append("unknown-query")
            for name,parameter in query_specs.items():
                if name not in actual_query:
                    if parameter.get("required"):request_errors.append("missing-query")
                    continue
                values=actual_query[name]
                if len(values)!=1:
                    request_errors.append("duplicate-query");continue
                value=values[0]
                if parameter["schema"].get("type")=="integer":
                    try:value=int(value)
                    except ValueError:request_errors.append("query-type");continue
                request_errors.extend(Draft202012Validator(parameter["schema"]).iter_errors(value))
            assert (not request_errors)==request_valid,(case,"request schema classification")
            specification = operation["responses"].get(str(status))
            assert specification is not None, (case, "undocumented status", status)
            if "content" in specification:
                assert mime in specification["content"], (case, "undocumented media type", mime)
                schema = specification["content"][mime]["schema"]
                validate(document, schema, parsed if mime == "application/json" else wire.decode("latin1"), case)
            for name, specification in specification.get("headers", {}).items():
                value = next((v for k, v in response_headers.items() if k.lower() == name.lower()), None)
                assert value is not None, (case, "missing response header", name)
                if specification["schema"].get("type") == "integer":
                    value = int(value)
                validate(document, specification["schema"], value, case + ":" + name)
        return status, parsed, wire, {k.lower(): v for k, v in response_headers.items()}

    def named(case, operation, token=None):
        status, result, _, _ = exchange(case, "POST", "/v1/operations", token or operator["credential"], body=operation,
            contract=("application","/v1/operations") if args.application_contract else
                (("attachment","/v1/operations") if operation["op"].startswith("artifact_") else None))
        assert status == 200 and result["profile"] == "connect-control/1" and "Ok" in result["result"], case
        return result["result"]["Ok"]

    def task_access(case, command, token=None, **kwargs):
        return exchange(case, "POST", "/v1/task-access", token or operator["credential"],
                        body=dict(profile="connect-task-access/1", command=command),
                        contract=("protected", "/v1/task-access"), **kwargs)

    try:
        with (directory / "serve.log").open("wb") as output, (directory / "serve.err").open("wb") as error:
            child = subprocess.Popen([str(cli), "serve", "--config", str(state / "server.toml")], cwd=ROOT, env=child_env,
                                     stdin=subprocess.DEVNULL, stdout=output, stderr=error)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline and b"ready " not in (directory / "serve.log").read_bytes():
                assert child.poll() is None, "server exited before ready"
                time.sleep(0.02)
            assert b"ready " in (directory / "serve.log").read_bytes(), "server ready deadline"
            # Discover this credential's own public metadata through configured MCP.
            # No SQL, guessed credential ID or Device-key extraction is needed.
            def inspect_credential(source):
                session = None
                def mcp_rpc(method, params, request_id):
                    nonlocal session
                    connection = http.client.HTTPConnection("127.0.0.1",mcp_port,timeout=5)
                    headers={"Authorization":"Bearer "+source["credential"],"Content-Type":"application/json",
                             "Accept":"application/json, text/event-stream","MCP-Protocol-Version":"2025-11-25","Connection":"close"}
                    if session:headers["MCP-Session-Id"]=session
                    try:
                        connection.request("POST","/mcp",json.dumps(dict(jsonrpc="2.0",id=request_id,method=method,params=params)),headers)
                        response=connection.getresponse();assert response.status==200
                        session=response.getheader("MCP-Session-Id",session)
                        data=response.read(65537);assert len(data)<=65536
                        value=json.loads(data);assert value.get("id")==request_id and "result" in value
                        return value["result"]
                    finally:connection.close()
                mcp_rpc("initialize",dict(protocolVersion="2025-11-25",capabilities={},clientInfo=dict(name="protected-http-check",version="1")),1)
                inspected=mcp_rpc("tools/call",dict(name="connect_inspect",arguments={}),2)
                assert not inspected.get("isError",False)
                metadata=inspected["structuredContent"]["data"]
                assert metadata["identity_id"]==source["identity_id"]
                return metadata["credential_id"]
            recipient_credential_id=inspect_credential(reviews)
            owner_credential_id=inspect_credential(agent)
            envelope = dict(v=2, type="conversation.create", id="http_" + uuid.uuid4().hex,
                            ts=datetime.datetime.now(datetime.timezone.utc).isoformat(), operation_id="protected-room",
                            payload=dict(tenant_id="tnt_default", policy="persistent",
                                         initial_participants=[dict(identity_id=reviews["identity_id"], role="participant")]))
            _, created, _, _ = exchange("canonical-room", "POST", "/v2/commands", operator["credential"],
                                         body=envelope, expected=200, contract=("canonical", "/v2/commands"))
            cid = next(f for f in created["envelopes"] if f["type"] == "ack")["payload"]["details"]["conversation"]["id"]
            task = named("task-create", dict(op="task", request=dict(task_profile="connect-task/1", command=dict(
                action="create", request_id="protected-task", definition=dict(conversation_id=cid,
                    scope=dict(operations=["send", "history", "inbox"], recipients=[reviews["identity_id"]]),
                    expires_at=int(time.time()) + 600, initiating_event_id=None)))))
            task_id = task["data"]["data"]["id"]
            issue = dict(action="issue", request_id="protected-issue", definition=dict(task_id=task_id,
                         binding=dict(kind="native", adapter="protected-http-check", external_id="fixture-native"),
                         operations=["history", "inbox"], expires_at=int(time.time()) + 240))
            _, result, _, headers = task_access("grant-issue", issue, expected=200)
            assert headers.get("cache-control") == "no-store" and headers.get("pragma") == "no-cache"
            grant = result["result"]["Ok"]["data"]
            if args.application_contract:
                from lib.application_http_cases import qualify
                summary["application"] = qualify(exchange, validate, documents["application"],
                    operator, reviews, agent, grant, cid, task_id)
            _, replay, _, _ = task_access("grant-original-replay", issue, expected=200)
            assert replay == result, "same original grant/secret/expiry"
            changed = copy.deepcopy(issue)
            changed["definition"]["operations"] = ["inbox"]
            _, conflict, _, _ = task_access("grant-changed-intent", changed, expected=200)
            assert conflict["result"]["Err"]["code"] == "request-conflict"
            _, denied, _, _ = task_access("ordinary-no-task-grant-scope", issue, token=reviews["credential"], expected=403)
            assert denied["result"]["Err"]["code"] == "forbidden"
            task_access("model-grant-cannot-issue", issue, token=grant["bearer"], expected=401)
            task_access("task-duplicate-authorization",issue,headers=[("Authorization","Bearer "+operator["credential"])],expected=401)
            exchange("task-no-authorization","POST","/v1/task-access",None,
                body=dict(profile="connect-task-access/1",command=issue),expected=401,
                contract=("protected","/v1/task-access"))
            for key, value in [("Origin", "http://localhost"), ("X-Call-ID", "not-a-provider-proof"), ("X-Chat-ID", "not-a-provider-proof"), ("Host", "foreign.invalid")]:
                task_access("task-header-" + key.lower(), issue, headers=[(key, value)], expected=403)
            provider = copy.deepcopy(issue)
            provider["request_id"] = "provider-refused"
            provider["definition"]["binding"]["kind"] = "provider_chat"
            task_access("provider-binding-refused", provider, expected=403)
            invalid = copy.deepcopy(issue)
            invalid["definition"]["operations"] = ["admin"]
            task_access("task-unknown-permission", invalid, request_valid=False, expected=422)
            invalid = copy.deepcopy(issue)
            invalid["definition"]["operations"] = ["inbox", "inbox"]
            _, duplicate, _, _ = task_access("task-duplicate-permission", invalid, request_valid=False, expected=200)
            assert duplicate["result"]["Err"]["code"] == "invalid-request"
            spoofed=copy.deepcopy(issue)
            spoofed["definition"]["actor_id"]=operator["identity_id"]
            task_access("task-caller-authority-field-refused",spoofed,request_valid=False,expected=422)
            too_long=copy.deepcopy(issue)
            too_long["request_id"]="too-long-grant"
            too_long["definition"]["expires_at"]=int(time.time())+600
            _,invalid_lifetime,_,_=task_access("task-relative-ttl-refused",too_long,expected=200)
            assert invalid_lifetime["result"]["Err"]["code"]=="invalid-request"
            _,wrong_profile,_,_=exchange("task-profile-separated","POST","/v1/task-access",operator["credential"],
                body=dict(profile="connect-control/1",command=issue),expected=200,
                contract=("protected","/v1/task-access"),request_valid=False)
            assert wrong_profile["result"]["Err"]["code"]=="unsupported-profile"
            for case, raw, content_type, expected, token in [
                ("task-malformed-json", b"{bad", "application/json", 400, operator["credential"]),
                ("task-no-json-type", b"{}", "text/plain", 415, operator["credential"]),
                ("task-oversized-json", b" " * 65537, "application/json", 413, operator["credential"]),
                ("task-auth-before-json", b"{bad", "application/json", 401, "invalid"),
            ]:
                exchange(case, "POST", "/v1/task-access", token, raw=raw,
                         headers=[("Content-Type", content_type)], expected=expected,
                         contract=("protected", "/v1/task-access"), request_valid=False)
            # File participants use their actual mutually permitted ordinary credentials.
            # The independent administrative Task room is not an artifact authority bypass.
            file_room = copy.deepcopy(envelope)
            file_room["id"] = "http_" + uuid.uuid4().hex
            file_room["operation_id"] = "protected-file-room"
            _, file_created, _, _ = exchange("canonical-file-room", "POST", "/v2/commands", agent["credential"],
                body=file_room, expected=200, contract=("canonical", "/v2/commands"))
            cid = next(f for f in file_created["envelopes"] if f["type"] == "ack")["payload"]["details"]["conversation"]["id"]
            content = b"Canonical raw artifact fixture\x00with two windows."
            artifact = named("artifact-begin-255-name", dict(op="artifact_begin", request_id="artifact-begin",
                conversation_id=cid, to="all", name="n" * 255, media_type="application/octet-stream",
                size=len(content), sha256=hashlib.sha256(content).hexdigest().upper()), token=agent["credential"])
            validate(documents["protected"], {"$ref": "#/components/schemas/ArtifactOutcome"}, artifact, "artifact-begin-255-name")
            artifact_id = artifact["data"]["id"]
            route = "/v1/conversations/{conversation_id}/artifacts/{artifact_id}"
            base = f"/v1/conversations/{cid}/artifacts/{artifact_id}"
            def raw_get(case, token, offset, limit, expected=200, request_valid=True):
                return exchange(case, "GET", base + f"?offset={offset}&limit={limit}", token,
                                expected=expected, contract=("protected", route),request_valid=request_valid)
            def raw_put(case, token, request_id, offset, body, expected=200, request_valid=True):
                query = urllib.parse.urlencode(dict(request_id=request_id, offset=offset))
                return exchange(case, "PUT", base + "?" + query, token, raw=body,
                                expected=expected, contract=("protected", route),request_valid=request_valid)
            raw_get("artifact-not-ready", agent["credential"], 0, 8, 403)
            raw_put("artifact-nonowner-put", reviews["credential"], "foreign-chunk", 0, content[:8], 403)
            raw_put("artifact-empty-chunk", agent["credential"], "empty-chunk", 0, b"", 400,request_valid=False)
            raw_put("artifact-configured-oversize", agent["credential"], "oversize-chunk", 0, b"x" * 32769, 400)
            _, first, _, _ = raw_put("artifact-first-chunk", agent["credential"], "chunk~first", 0, content[:8])
            _, repeated, _, _ = raw_put("artifact-chunk-original-replay", agent["credential"], "chunk~first", 0, content[:8])
            assert first == repeated and first["result"]["Ok"]["data"]["received"] == 8
            _, conflict, _, _ = raw_put("artifact-chunk-changed-intent", agent["credential"], "chunk~first", 0, b"changed!", 400)
            assert conflict["result"]["Err"]["code"] in ("conflict", "request-conflict")
            raw_put("artifact-second-chunk", agent["credential"], "chunk-second", 8, content[8:])
            finished = named("artifact-finish", dict(op="artifact_finish", request_id="artifact-finish",
                                                    conversation_id=cid, artifact_id=artifact_id), token=agent["credential"])
            assert finished["data"]["state"] == "ready" and finished["data"]["received"] == len(content)
            for case, offset, limit, expected_bytes, eof in [
                ("artifact-first-window", 0, 8, content[:8], "false"),
                ("artifact-second-window", 8, 32768, content[8:], "true"),
                ("artifact-empty-eof", len(content), 1, b"", "true"),
            ]:
                _, _, received, headers = raw_get(case, agent["credential"], offset, limit)
                assert received == expected_bytes and headers["x-connect-eof"] == eof
                assert headers["x-connect-offset"] == str(offset)
            sharing = dict(v=2, type="message.send", id="http_" + uuid.uuid4().hex,
                ts=datetime.datetime.now(datetime.timezone.utc).isoformat(), cid=cid, operation_id="protected-file-share",
                payload=dict(msg_id="protected-file-message", to="all", content_type="text/plain",
                    body="Fixture attachment", body_encoding="utf8", label="connect-messages",
                    attachments=[dict(id=artifact_id, content_type="application/octet-stream", size_bytes=len(content))]))
            exchange("canonical-file-message", "POST", "/v2/commands", agent["credential"], body=sharing,
                expected=200, contract=("canonical", "/v2/commands"))
            _, _, received, _ = raw_get("artifact-original-recipient", reviews["credential"], 0, 32768)
            assert received == content
            raw_get("artifact-foreign-identity", operator["credential"], 0, 8, 403)
            raw_get("artifact-grant-requires-task-api", grant["bearer"], 0, 8, 401)
            raw_get("artifact-beyond-eof", agent["credential"], len(content) + 1, 8, 403)
            raw_get("artifact-limit-zero", agent["credential"], 0, 0, 400,request_valid=False)
            raw_get("artifact-query-type", agent["credential"], "invalid", 8, 400,request_valid=False)
            exchange("artifact-query-unknown", "GET", base + "?offset=0&limit=1&credential=not-authority",
                     agent["credential"], expected=400, contract=("protected", route),request_valid=False)
            _, revoked, _, _ = task_access("grant-revoke", dict(action="revoke", grant_id=grant["grant"]["id"]), expected=200)
            assert revoked["result"]["Ok"]["data"]["grant_id"] == grant["grant"]["id"]
            task_access("grant-revocation-refuses-original", issue, expected=403)
            revoke = dict(type="revoke_credential", operation_id="revoke-recipient", credential_id=recipient_credential_id)
            status, result, _, _ = exchange("recipient-credential-revoke", "POST", f"/v2/credentials/{recipient_credential_id}/revoke",
                                             operator["credential"], body=revoke, expected=200, contract=("provisioning","/v2/credentials/{id}/revoke"))
            assert "Ok" in result["result"]
            raw_get("artifact-current-recipient-revocation", reviews["credential"], 0, 8, 401)
            _, _, retained, _ = raw_get("artifact-owner-remains-current", agent["credential"], 0, 32768)
            assert retained == content
            revoke_owner=dict(type="revoke_credential",operation_id="revoke-uploader",credential_id=owner_credential_id)
            _,owner_revoked,_,_=exchange("uploader-credential-revoke","POST",f"/v2/credentials/{owner_credential_id}/revoke",
                operator["credential"],body=revoke_owner,expected=200,contract=("provisioning","/v2/credentials/{id}/revoke"))
            assert "Ok" in owner_revoked["result"]
            raw_put("artifact-current-authority-on-original-replay",agent["credential"],"chunk~first",0,content[:8],401)
            credential_request=dict(operation_id="protected-credential-original",identity_id=agent["identity_id"],
                operations=["operation.get"],recipients=[reviews["identity_id"]],expires_at=int(time.time())+180)
            def credential(case,request=credential_request,token=None,**kwargs):
                return exchange(case,"POST","/v2/credentials",token or operator["credential"],body=request,
                    contract=("protected","/v2/credentials"),**kwargs)
            _,issued,_,headers=credential("credential-issue",expected=200)
            scoped=issued["result"]["Ok"]
            assert scoped["identity_id"]==agent["identity_id"]
            assert scoped["issuer_identity_id"]==operator["identity_id"]
            assert headers["connection"]=="close" and headers["pragma"]=="no-cache"
            _,replayed,_,_=credential("credential-original-replay",expected=200)
            assert replayed==issued
            equivalent=copy.deepcopy(credential_request);equivalent["operations"]*=2
            assert credential("credential-normalized-replay",equivalent,expected=200)[1]==issued
            changed=copy.deepcopy(credential_request);changed["expires_at"]+=1
            credential("credential-changed-intent",changed,expected=409)
            credential("credential-narrow-cannot-issue",token=scoped["credential"],expected=403)
            credential("credential-model-grant-refused",token=grant["bearer"],expected=401)
            for key,value in [("Origin","http://localhost"),("X-Call-ID","not-proof"),("X-Chat-ID","not-proof"),("Host","foreign.invalid")]:
                credential("credential-header-"+key.lower(),headers=[(key,value)],expected=403)
            credential("credential-duplicate-authorization",headers=[("Authorization","Bearer "+operator["credential"])],expected=401)
            for change in ["admin","credential.delegate","*"]:
                invalid=copy.deepcopy(credential_request);invalid["operations"]=[change]
                credential("credential-scope-"+change.replace("*","wildcard"),invalid,expected=400,request_valid=False)
            invalid=copy.deepcopy(credential_request);invalid["user_id"]="synthetic"
            credential("credential-synthetic-user",invalid,expected=400,request_valid=False)
            expired=copy.deepcopy(credential_request);expired["expires_at"]=int(time.time())-1
            credential("credential-expired-intent",expired,expected=403)
            for case,body,token,content_type,status in [
                ("credential-malformed-json",b"{",operator["credential"],"application/json",400),
                ("credential-no-json-type",b"{}",operator["credential"],"text/plain",400),
                ("credential-oversized-json",b" "*65537,operator["credential"],"application/json",413),
                ("credential-auth-before-json",b"{","invalid","application/json",401),
            ]:
                exchange(case,"POST","/v2/credentials",token,raw=body,headers=[("Content-Type",content_type)],
                    contract=("protected","/v2/credentials"),request_valid=False,expected=status)
            revoke_child=dict(type="revoke_credential",operation_id="revoke-issued-child",credential_id=scoped["credential_id"])
            exchange("credential-revoke-child","POST",f'/v2/credentials/{scoped["credential_id"]}/revoke',operator["credential"],
                body=revoke_child,contract=("provisioning","/v2/credentials/{id}/revoke"),expected=200)
            credential("credential-current-child-revocation",expected=401)
            child.send_signal(signal.SIGTERM)
            assert child.wait(timeout=15) == 0
            assert b"stopped" in (directory / "serve.log").read_bytes()
            with socket.socket() as sock:
                sock.settimeout(1)
                assert sock.connect_ex(("127.0.0.1", http_port)) != 0, "listener remained after shutdown"
            with socket.socket() as sock:
                sock.settimeout(1)
                assert sock.connect_ex(("127.0.0.1", mcp_port)) != 0, "MCP listener remained after shutdown"
            assert all(sha(ROOT / p) == h for p, h in inputs.items())
            assert all(sha(ROOT / p) == h for p, h in source_guard.items())
            for source in [operator, reviews, agent, scoped]:
                assert source["credential"].encode() not in (directory / "serve.log").read_bytes() + (directory / "serve.err").read_bytes()
            required={"grant-issue","grant-original-replay","grant-changed-intent","model-grant-cannot-issue",
                "task-duplicate-authorization","task-no-authorization","provider-binding-refused",
                "task-caller-authority-field-refused","task-relative-ttl-refused","task-profile-separated",
                "task-auth-before-json","task-oversized-json","artifact-begin-255-name",
                "artifact-nonowner-put","artifact-chunk-original-replay","artifact-chunk-changed-intent",
                "artifact-first-window","artifact-second-window","artifact-empty-eof",
                "artifact-original-recipient","artifact-foreign-identity","artifact-grant-requires-task-api",
                "artifact-query-unknown","grant-revocation-refuses-original",
                "artifact-current-recipient-revocation","artifact-owner-remains-current",
                "artifact-current-authority-on-original-replay"}
            required.update({"credential-issue","credential-original-replay","credential-normalized-replay",
                "credential-changed-intent","credential-narrow-cannot-issue","credential-model-grant-refused",
                "credential-synthetic-user","credential-auth-before-json","credential-current-child-revocation"})
            assert len(rows)>=74 and len({row["case"] for row in rows})==len(rows)
            assert required<={row["case"] for row in rows},"mandatory behavior evidence missing"
            summary.update(status="passed local protected HTTP OpenAPI checkpoint", source_unchanged=True,
                           reference_inputs=source_guard, protected_samples=len(rows), schema_checked=sum(v["schema_checked"] for v in rows),
                           setup_only=[v["case"] for v in rows if not v["schema_checked"]], sigterm_exit=0,
                           scope="Isolated actual CLI/production loopback HTTP. No model, provider account, remote/Linux/media/package/acceptance qualification or publication.")
            (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
            print(json.dumps(dict(status=summary["status"], cases=len(rows), schema_checked=summary["schema_checked"], directory=str(directory.relative_to(ROOT)))))
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait(timeout=5)


if __name__ == "__main__":
    main()
