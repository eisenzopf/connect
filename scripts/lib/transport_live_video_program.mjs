// A finite private source description, never a media/feedback authorization.
import assert from "node:assert/strict";
import { isAbsolute, resolve, sep } from "node:path";

export function liveVideoProgram(program, frames) {
    assert.ok(program && typeof program === "object");
    assert.deepEqual(Object.keys(program).sort(), ["executable", "executable_sha256", "frame_count", "raw_path", "raw_sha256"]);
    assert.ok(Number.isInteger(frames) && frames >= 1 && frames <= 1860);
    assert.equal(program.frame_count, frames);
    for (const field of ["executable", "raw_path"]) {
        const path = program[field];
        assert.ok(typeof path === "string" && isAbsolute(path) && !path.includes("\0") &&
            path.split(sep).includes(".local") && !path.split(sep).includes(".."));
    }
    assert.notEqual(program.executable, program.raw_path);
    for (const field of ["executable_sha256", "raw_sha256"]) assert.match(program[field], /^[0-9a-f]{64}$/);
    return Object.freeze({ ...program });
}

export function liveVideoProgramFromManifest(directory, manifest, index) {
    assert.equal(manifest.live_encoder?.file, "live-vp8-encoder");
    assert.equal(manifest.sources[index].raw_video_file, "source-" + index + ".i420");
    return liveVideoProgram({ executable: resolve(directory, manifest.live_encoder.file),
        executable_sha256: manifest.live_encoder.executable_sha256,
        raw_path: resolve(directory, manifest.sources[index].raw_video_file),
        raw_sha256: manifest.sources[index].raw_video_sha256, frame_count: 1860 }, 1860);
}
