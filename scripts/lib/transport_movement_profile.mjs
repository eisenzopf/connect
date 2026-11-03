// The owner-approved movement cadence is shared by production and custody checks.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";

const contract = JSON.parse(readFileSync(new URL("../../contracts/gate6-transport-matrix.json", import.meta.url), "utf8"));
const profile = contract.workload.movement;
assert.equal(profile.profile, "gate6-realistic-device-roundtrip-v1");
assert.equal(profile.total_moves, 4);
assert.equal(profile.moves_per_session, 2);
assert.deepEqual(profile.start_offsets_ms, [15000, 40000]);
assert.equal(profile.programme_ms, 62000);
export const MOVE_START_OFFSETS_MS = Object.freeze([...profile.start_offsets_ms]);
export const MOVES_PER_SESSION = profile.moves_per_session;
export const TOTAL_MOVES = profile.total_moves;
export const MOVEMENT_PROFILE = profile.profile;
export const MOVEMENT_PROFILE_SHA256 = createHash("sha256").update(JSON.stringify(profile)).digest("hex");
