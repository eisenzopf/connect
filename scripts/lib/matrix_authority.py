"""Bounded public-CLI authority changes requested by the actual browser test."""
import json
import time
import uuid
from pathlib import Path

from .browser_matrix_actors import browser_actor
from .transport_actors import OPERATIONS
from .transport_fixture import publish


class MatrixAuthority:
    def __init__(self, directory, actors_root, records, instance, admin, cli, algorithm):
        self.directory, self.actors_root = Path(directory), Path(actors_root)
        self.records, self.instance, self.admin, self.cli, self.algorithm = records, instance, admin, cli, algorithm
        self.sequence, self.issued = 0, {}

    def poll(self):
        path = self.directory / f"authority-{self.sequence + 1}-request.json"
        if not path.exists():
            return
        if path.is_symlink() or path.stat().st_size > 1024 or self.sequence >= 3:
            raise ValueError("Original authority request bound")
        request = json.loads(path.read_text())
        if set(request) != {"sequence", "kind"} or request["sequence"] != self.sequence + 1:
            raise ValueError("Original authority request differs")
        kind = request["kind"]
        self.sequence += 1
        try:
            if kind in {"expiry", "revocation"}:
                if kind in self.issued:
                    raise ValueError("Original authority case already issued")
                expires = int(time.time()) + (12 if kind == "expiry" else 120)
                request_path = self.actors_root / ("matrix-" + kind + "-intent.json")
                installation = self.actors_root / ("matrix-" + kind)
                identity = self.records["participant-1"]["identity_id"]
                publish(request_path, json.dumps(dict(operation_id="matrix-authority-" + uuid.uuid4().hex,
                    identity_id=identity, operations=OPERATIONS,
                    recipients=sorted({r["identity_id"] for r in self.records.values()} - {identity}),
                    expires_at=expires)).encode())
                self.cli("authority-issue-" + kind, ["issue", "--config", self.admin,
                    "--native-template", self.instance / "operator.toml", "--request", request_path,
                    "--output", installation, "--signing-algorithm", self.algorithm])
                issued = json.loads((installation / "issued.json").read_text())
                self.issued[kind] = issued["credential_id"]
                actor, _ = browser_actor(installation, scope=self.actors_root, algorithm=self.algorithm)
                response = dict(actor=actor, expires_at=expires)
            elif kind == "revoke" and "revocation" in self.issued:
                outcome = json.loads(self.cli("authority-revoke", ["revoke", "--config", self.admin,
                    "--operation-id", "matrix-revoke-" + uuid.uuid4().hex, self.issued["revocation"]]))
                response = dict(revoked=outcome["result"]["Ok"]["revoked"])
            else:
                raise ValueError("Unknown original authority case")
        except Exception as error:
            response = dict(failure=type(error).__name__)
        publish(self.directory / f"authority-{self.sequence}-response.json", json.dumps(response).encode())
