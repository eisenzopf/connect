#!/usr/bin/env python3
"""Inventory declared HTTP mounts and require current OpenAPI references.

This is a balanced source-call inventory, not runtime/schema/profile proof.
Parameter spelling is normalized only when matching an existing route reference.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
METHODS = {"get", "post", "put", "patch", "delete", "head", "options", "trace"}


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def pointer(value):
    return value.replace("~", "~0").replace("/", "~1")


def normalized(path):
    return re.sub(r"\{[^{}]+\}", "{}", path)


def route_calls(source):
    for found in re.finditer(r"\.(route|route_service|nest|nest_service)\s*\(", source):
        index, depth, quoted, escaped = found.end(), 1, False, False
        while index < len(source) and depth:
            char = source[index]
            if quoted:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    quoted = False
            elif char == '"':
                quoted = True
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            index += 1
        assert depth == 0, "Unbalanced route call"
        yield found[1], source[found.end():index - 1], source.count("\n", 0, found.start()) + 1


def render():
    references, input_hashes = {}, {}
    canonical = None
    for path in sorted((ROOT / "contracts").glob("openapi-*.json")):
        document = json.loads(path.read_text())
        relative = str(path.relative_to(ROOT))
        input_hashes[relative] = sha(path)
        if path.name == "openapi-canonical-http.json":
            canonical = document
        for route, item in document["paths"].items():
            for method in set(item) & METHODS:
                key = (normalized(route), method)
                references.setdefault(key, []).append(dict(
                    document=relative, path=route, method=method,
                    pointer="/paths/" + pointer(route) + "/" + method))
    assert canonical is not None
    discovery_path = ROOT / "crates/connect-protocol/src/canonical_discovery.rs"
    discovery = re.search(r'pub const PATH: &str = "([^"]+)";', discovery_path.read_text())[1]
    input_hashes[str(discovery_path.relative_to(ROOT))] = sha(discovery_path)
    constants = {"connect_protocol::canonical_discovery::PATH": discovery}
    paths = subprocess.check_output(["rg", "--files", "crates/connect-server/src", "crates/connect-mcp/src"],
                                    cwd=ROOT, text=True).splitlines()
    declarations = []
    for relative in sorted(p for p in paths if p.endswith(".rs")):
        path = ROOT / relative
        source = path.read_text()
        for kind, body, line in route_calls(source):
            expression, handler = body.split(",", 1)
            expression = expression.strip()
            literal = re.fullmatch(r'"([^"\n]+)"', expression)
            route = literal[1] if literal else constants.get(expression)
            assert route is not None, (relative, line, "Unresolved route expression", expression)
            input_hashes[relative] = sha(path)
            row = dict(source=relative, line=line, registration=kind, path=route)
            if kind == "route":
                methods = sorted(set(re.findall(r"\b(get|post|put|patch|delete|head|options|trace)\s*\(", handler)))
                assert methods, (relative, line, "Unresolved method router")
                row["operations"] = []
                for method in methods:
                    refs = references.get((normalized(route), method))
                    assert refs, (relative, line, route, method, "Missing OpenAPI reference")
                    row["operations"].append(dict(method=method, references=refs))
            elif kind == "route_service" and route == "/mcp" and relative == "crates/connect-mcp/src/listener.rs":
                assert "/mcp" in canonical["x-connect-canonical-mcp"]["transport"]
                row["protocol_reference"] = dict(document="contracts/openapi-canonical-http.json",
                    pointer="/x-connect-canonical-mcp", scope="Delegated RMCP protocol service; not a REST operation schema.")
            else:
                raise AssertionError((relative, line, kind, "Explicit service/nest reference required"))
            declarations.append(row)
    assert declarations
    transports = []
    for name, relative, key in [
        ("WebSocket", "crates/connect-server/src/canonical_websocket.rs", "x-connect-canonical-websocket"),
        ("WebTransport", "crates/connect-server/src/canonical_webtransport.rs", "x-connect-canonical-webtransport"),
    ]:
        source = (ROOT / relative).read_text()
        assert "pub mount_path: String" in source and key in canonical
        input_hashes[relative] = sha(ROOT / relative)
        transports.append(dict(transport=name, source=relative, path="Explicit runtime-configured mount_path",
            reference=dict(document="contracts/openapi-canonical-http.json", pointer="/" + key),
            scope="Configuration/protocol metadata only; no browser/media/remote qualification inferred."))
    operation_count = sum(len(row.get("operations", [])) for row in declarations)
    return dict(status="Source route/reference inventory only; runtime schemas and authorization need separate evidence.",
        scope="All current route/route_service/nest/nest_service declarations in Connect server and MCP source, plus explicit configured canonical transport mounts. Framework-generated methods and all /v1/operations body variants are not independently qualified by path coverage.",
        declarations=len(declarations), explicit_http_operations=operation_count,
        parameter_matching="Only placeholder names are normalized. Original source path and every referenced path spelling remain recorded.",
        routes=declarations, configured_transports=transports, inputs=dict(sorted(input_hashes.items())))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    value = render()
    content = json.dumps(value, indent=2) + "\n"
    target = ROOT / "contracts/http-endpoints.json"
    if args.check:
        assert target.read_text() == content, "HTTP endpoint inventory is stale"
    else:
        target.write_text(content)
    print(json.dumps(dict(declarations=value["declarations"], explicit_http_operations=value["explicit_http_operations"],
                          uncovered=0, scope="Source/reference inventory only.")))


if __name__ == "__main__":
    main()
