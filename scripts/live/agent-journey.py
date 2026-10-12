#!/usr/bin/env python3
"""Qualify installed agent conversations/restart using actual model inference.

Requires an explicitly selected installed runtime, its passing package report,
an authenticated Codex executable and --allow-model-inference. Creates private
isolated local state; never calls communication providers or deploys a server.
"""
import json
from agent_journey.cli import main
from agent_journey.common import ProofFailure

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ProofFailure as error:
        print(json.dumps({"status": "not-started", "failure_label": str(error)}), flush=True)
        raise SystemExit(2)
