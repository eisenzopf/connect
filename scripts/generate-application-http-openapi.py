#!/usr/bin/env python3
"""Derive the current application HTTP reference from real Rust serde DTOs.

Generation proves source/schema agreement, not runtime authorization or media.
The optional protocol schema feature does not participate in production dispatch.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "contracts/openapi-application-http.json"
CATALOG = ROOT / "contracts/application-http-dispatch.json"


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def inputs():
    paths = list((ROOT / "crates/connect-protocol").rglob("*.rs")) + [
        ROOT / "crates/connect-protocol/Cargo.toml", ROOT / "Cargo.toml", ROOT / "Cargo.lock",
        ROOT / "dependencies/source-lock.json", Path(__file__).resolve(),
        ROOT / "crates/connect-server/src/transports/http.rs",
        ROOT / "contracts/openapi-canonical-http.json"]
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths)}


def ref(name):
    return {"$ref": "#/components/schemas/" + name}


def resolve(value, schemas):
    while "$ref" in value:
        value = schemas[value["$ref"].removeprefix("#/components/schemas/")]
    return value


def bundle_schema(root, prefix, schemas):
    root = copy.deepcopy(root)
    definitions = root.pop("$defs", {})
    root.pop("$schema", None)
    root_name = prefix + root["title"]

    def references(value):
        if isinstance(value, dict):
            if "$ref" in value:
                reference = value["$ref"]
                if reference == "#":
                    value["$ref"] = "#/components/schemas/" + root_name
                else:
                    assert reference.startswith("#/$defs/"), reference
                    value["$ref"] = "#/components/schemas/" + prefix + reference[8:]
            for item in value.values():
                references(item)
        elif isinstance(value, list):
            for item in value:
                references(item)
    references(root)
    for name, body in definitions.items():
        references(body)
        # serde Result is externally tagged and admits exactly one map entry.
        # Schemars leaves unrelated keys open; close this known wire invariant.
        if name.startswith("Result_of_"):
            assert all(set(v["properties"]) in ({"Ok"}, {"Err"}) for v in body["oneOf"])
            for variant in body["oneOf"]:
                variant["additionalProperties"] = False
        schemas[prefix + name] = body
    assert root_name not in schemas
    schemas[root_name] = root
    return root


def render(derived, source):
    profile = derived["profile"]
    schemas = {}
    wire = bundle_schema(derived["input"], "Input_", schemas)
    response = bundle_schema(derived["output"], "Output_", schemas)
    policy = derived["dispatch"]
    kinds = sorted({p["response_kind"] for p in policy if p["response_kind"]})
    tags = [v["properties"]["op"]["const"] for v in wire["oneOf"]]
    assert len(tags) == len(set(tags))
    assert set(tags) == {p["sample"]["op"] for p in policy}
    supported, retired = [], []
    for variant in wire["oneOf"]:
        tag = variant["properties"]["op"]["const"]
        rows = [p for p in policy if p["sample"]["op"] == tag]
        allowed = [p for p in rows if p["response_kind"]]
        if not allowed:
            retired.append(variant)
            continue
        request = copy.deepcopy(variant)
        if tag == "membership":
            body = copy.deepcopy(resolve(request["properties"]["request"], schemas))
            changes = resolve(body["properties"]["change"], schemas)
            actions = {p["sample"]["request"]["change"]["action"] for p in allowed}
            refused = copy.deepcopy(variant)
            refused_body = copy.deepcopy(body)
            refused_body["properties"]["change"] = {"oneOf": [v for v in changes["oneOf"]
                if v["properties"]["action"]["const"] not in actions]}
            refused["properties"]["request"] = refused_body
            retired.append(refused)
            body["properties"]["change"] = {"oneOf": [v for v in changes["oneOf"]
                if v["properties"]["action"]["const"] in actions]}
            request["properties"]["request"] = body
        elif tag == "message_receipt":
            states = {p["sample"]["state"] for p in allowed}
            refused = copy.deepcopy(variant)
            refused["properties"]["state"] = {"type":"string","enum":sorted({p["sample"]["state"] for p in rows}-states)}
            retired.append(refused)
            request["properties"]["state"] = {"type": "string", "enum": sorted(states)}
        else:
            assert len(rows) == 1 and len(allowed) == 1, tag
        schemas["Request_" + tag] = request
        supported.append(ref("Request_" + tag))
    schemas["WireOperation"] = wire
    schemas["ApplicationOperation"] = {"oneOf": supported,
        "description": "Current top-level named API. Nested Task Invoke retains the distinct full wire DTO and current Task policy. The schema describes serde shape; business validation/current authorization may still refuse."}
    schemas["RetiredTopLevelOperation"] = {"oneOf": retired,
        "description": "Shape-only reference for intentional HTTP501 refusals, including mixed membership/receipt branches. Never a fallback or a supported communication interface."}
    outcome = copy.deepcopy(schemas["Output_Outcome"])
    outcome["oneOf"] = [v for v in outcome["oneOf"] if v["properties"]["kind"]["const"] in kinds]
    assert {v["properties"]["kind"]["const"] for v in outcome["oneOf"]} == set(kinds)
    schemas["ApplicationOutcome"] = outcome
    result = copy.deepcopy(resolve(response["properties"]["result"], schemas))
    for variant in result["oneOf"]:
        if "Ok" in variant["properties"]:
            variant["properties"]["Ok"] = ref("ApplicationOutcome")
    response["properties"]["result"] = result
    response["properties"]["profile"] = {"type": "string", "const": profile}
    schemas["ApplicationResponse"] = response

    def json_response(description):
        return {"description": description, "content": {"application/json": {"schema": ref("ApplicationResponse")}}}
    def text_response(description):
        return {"description": description, "content": {"text/plain": {"schema": {"type": "string"}}}}
    responses = {str(code): json_response(description) for code, description in {
        200: "Named result or typed business refusal. Inspect result.Err as well as status; invalid limits, changed intent, conflicts and pending/unknown effects need not change HTTP status.",
        401: "Current ordinary credential or explicit verified Task principal missing, expired, revoked or malformed.",
        403: "Current scope, resource, recipient, Task/grant/assurance or verified provider-header ceiling refused.",
        501: "Recognized retired top-level command/action or an explicitly unsupported named capability. Use the canonical interface for retired calls; no fallback dispatch occurs.",
        503: "Shared capacity exhausted or current authority/storage unavailable. Preserve original logical IDs and inspect acceptance after uncertainty.",
    }.items()}
    responses.update({str(code): text_response(description) for code, description in {
        400: "Framework malformed JSON rejection before authentication.",
        413: "Framework aggregate body limit before authentication: max(configured artifact_chunk_bytes,65536), default65536.",
        415: "Framework JSON content type required before authentication.",
        422: "Framework closed serde shape/type/enum/duplicate-field rejection before authentication.",
    }.items()})
    canonical = json.loads((ROOT / "contracts/openapi-canonical-http.json").read_text())
    schemas["AdmissionProblem"] = copy.deepcopy(canonical["components"]["schemas"]["Problem"])
    schemas["AdmissionRejection"] = copy.deepcopy(canonical["components"]["schemas"]["Rejection"])
    schemas["AdmissionRejection"]["properties"]["problem"] = ref("AdmissionProblem")
    responses["429"] = json_response("Additional enrollment-Origin admission capacity refused; no operation authority is granted.")
    for code in ("401", "403", "429", "503"):
        responses[code]["content"]["application/json"]["schema"] = {"oneOf": [ref("ApplicationResponse"), ref("AdmissionRejection")]}
    operation = dict(operationId="executeNamedApplicationOperation", security=[dict(bearerAuth=[])],
        description="Explicit application route. The additional enrollment ceiling checks exact browser Origin (including native absence) for authenticated ordinary and explicit Task bearers before decoding; it never grants an operation. Invalid bearer syntax/authentication remains with this route. Framework JSON body parsing precedes the route own bounded shared admission and bearer authentication. Host/Origin/enrollment/capacity policy can reject earlier with a canonical problem response. Canonical communication/control is deliberately excluded here. Current ordinary actors and verified narrow Task principals keep their own ceilings; X-Call-ID/X-Chat-ID do not create provider authority and are refused with Task principals. CORS only permits configured loopback browser origins; it is not authentication and does not make arbitrary headers trusted. Service/Store reauthorize current scope, membership, resource audience, expiry and immutable intent on replay. Administrator status is not private content access. Task request/step/reply attribution and original IDs remain distinct. Pairing proves Identity possession without SMS consent. Provider status/readiness are observations, not successful external effects or decoded media. The route adds no no-store header and has no route-wide absolute transaction deadline; named service waits and explicit SDK bounds retain their separate contracts.",
        requestBody=dict(required=True, content={"application/json":dict(schema=ref("ApplicationOperation"))}),
        responses=responses,
        **{"x-body-limit":"max(limits.artifact_chunk_bytes,65536); default65536", "x-automatic-retries":0,
           "x-schema-scope":"Actual serde DTO shape and current top-level dispatch. Business normalization, cursor/ID/range limits, authorization and lifecycle are service policy; schema validity is not acceptance."})
    dispatch = []
    for row in policy:
        sample = row["sample"]
        selector = sample["op"]
        if selector == "membership": selector += "." + sample["request"]["change"]["action"]
        if selector == "message_receipt": selector += "." + sample["state"]
        dispatch.append(dict(selector=selector, supported=row["response_kind"] is not None,
                             response_kind=row["response_kind"]))
    return dict(openapi="3.1.0", info=dict(title="Connect current named application HTTP", version=profile,
        description="Generated from real protocol serde DTOs in separate deserialize/serialize contexts and executable ApplicationOperation policy. Optional schema tooling is absent from production builds. Generation is not runtime, media, provider, host or remote qualification."),
        paths={"/v1/operations":dict(post=operation)},
        components=dict(securitySchemes=dict(bearerAuth=dict(type="http",scheme="bearer")),schemas=schemas),
        **{"x-connect-dispatch":dispatch,"x-connect-generation-inputs":source,
           "x-connect-generation-refinements":["Serde externally tagged Result contains exactly one Ok/Err key.",
             "Only top-level Outcome is narrowed; TaskEffect nested outcomes retain full attribution/recovery models.",
             "Duplicate metadata JSON keys are refused by a custom serde byte decoder; parsed JSON Schema cannot express duplicate keys."]})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    subprocess.run(["bash","scripts/prepare-dependencies.sh"],cwd=ROOT,check=True,stdout=subprocess.DEVNULL)
    source=inputs()
    result=subprocess.run(["cargo","run","--release","--offline","--locked","--quiet","-p","connect-protocol",
                           "--features","schema","--example","application_http_schema"],cwd=ROOT,
                          env=os.environ.copy(),capture_output=True,check=False)
    if result.returncode:
        raise RuntimeError("Protocol schema build failed; inspect Cargo diagnostics:\n"+result.stderr.decode(errors="replace"))
    assert source==inputs(),"Protocol/reference source changed during generation"
    derived=json.loads(result.stdout)
    content=json.dumps(render(derived,source),indent=2)+"\n"
    catalog=json.dumps(dict(scope="Synthetic serde-shape and dispatch samples only. Not authorization, business-valid examples, provider effects or media evidence.",
        cases=derived['dispatch'],task_cases=derived['task_dispatch'],inputs=source),indent=2)+"\n"
    if args.check:
        assert TARGET.read_text()==content,"Current application HTTP reference is stale"
        assert CATALOG.read_text()==catalog,"Current application dispatch catalog is stale"
    else:
        TARGET.write_text(content)
        CATALOG.write_text(catalog)
    value=json.loads(content)
    print(json.dumps(dict(wire_tags=len(value['components']['schemas']['WireOperation']['oneOf']),
        dispatch_cases=len(value['x-connect-dispatch']),supported_cases=sum(v['supported'] for v in value['x-connect-dispatch']),
        scope="Source/schema/dispatch generation only; no runtime qualification.")))


if __name__=="__main__":
    main()
