#!/usr/bin/env python3
"""Audit one retained transport ZIP against its GitHub digest and exact sources.

Exit zero means the requested positive/negative evidence was independently
checked. Only component_passed reports runtime success; no complete cell passes.
No artifact code, credentials, raw logs or packet captures are executed/read.
"""
import argparse
import json
from pathlib import Path

from lib.transport_verification import COMPONENTS, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--artifact-sha256", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--component", choices=sorted(COMPONENTS), required=True)
    parser.add_argument("--target", choices=("ubuntu24-arm64", "macos-arm64"), required=True)
    parser.add_argument("--signing", choices=("EdDSA", "ES256", "both"), required=True)
    parser.add_argument("--network", choices=("local", "remote-profile-fixture"), required=True)
    parser.add_argument("--expect-result", choices=("passed", "failed"), required=True)
    parser.add_argument("--sdk-dist", type=Path, default=Path("sdk/typescript/dist"))
    parser.add_argument("--prepared", type=Path, default=Path(".deps"))
    parser.add_argument("--archives", type=Path)
    args = parser.parse_args()
    result = verify(args.artifact, args.artifact_sha256.removeprefix("sha256:"), args.revision,
        args.component, args.target, args.signing, args.network, args.sdk_dist, args.prepared,
        args.archives, args.expect_result == "passed")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
