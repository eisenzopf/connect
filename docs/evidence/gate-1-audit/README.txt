Gate 1 additional completion audit

Run from the Connect repository root with the retained Gate 1 candidate installed
at .local/releases/connect-0.1.0-alpha.1-macos-arm64-263fe1592ceb.
Python 3.11+ and Node with the global WebSocket client are required.

mkdir -p .local/gate-1-audit
cp docs/evidence/gate-1-audit/observer.mjs .local/gate-1-audit/observer.mjs
cp docs/evidence/gate-1-audit/check-combined.py .local/gate-1-audit/check-combined.py
python3 .local/gate-1-audit/check-combined.py

The harness uses the repository scripts/lib/local_process.py helper and public
packaged commands. It creates temporary state, binds only loopback, launches
two packaged native example processes and one Node WebSocket observer, checks
identical native/WS history, then stops its own processes and removes its state.
It does not access live provider resources or existing application databases.

Original candidate evidence is retained under docs/evidence/gate-1.
The new combined result supplements that evidence; it is not a full suite rerun.
