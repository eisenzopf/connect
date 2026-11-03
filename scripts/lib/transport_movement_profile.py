"""The same owner-approved cadence used by the live Node producer."""
import hashlib
import json
from pathlib import Path

from .transport_matrix import load_contract

PROFILE = load_contract(Path(__file__).resolve().parents[2])["workload"]["movement"]
MOVE_START_OFFSETS_MS = tuple(PROFILE["start_offsets_ms"])
MOVES_PER_SESSION = PROFILE["moves_per_session"]
TOTAL_MOVES = PROFILE["total_moves"]
MOVEMENT_PROFILE = PROFILE["profile"]
MOVEMENT_PROFILE_SHA256 = hashlib.sha256(json.dumps(PROFILE, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
