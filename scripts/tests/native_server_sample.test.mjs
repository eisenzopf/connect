import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { selectOwnedServer, summarizeNativeSample, demangleNativeSymbols, normalizeNativeSymbol, finishNativeBeforeStop } from "../lib/native_server_sample.mjs";
test("normal restart keeps its original child alive until the held observer joins", async () => {
    const child = spawn(process.execPath, ["-e", "process.send('ready');setInterval(()=>{},1000)"], { stdio: ["ignore", "ignore", "ignore", "ipc"] });
    const joined = once(child, "close");
    let release;
    const held = new Promise(done => { release = done; });
    try {
        await once(child, "message");
        const pending = finishNativeBeforeStop({ finish: () => held }, async () => { child.kill("SIGTERM"); await joined; });
        await new Promise(done => setImmediate(done));
        assert.equal(child.exitCode, null); assert.equal(child.signalCode, null); assert.doesNotThrow(() => process.kill(child.pid, 0));
        const observation = { observed: true, observer_joined: true };
        release(observation);
        assert.equal(await pending, observation);
        assert.equal(child.signalCode, "SIGTERM");
    } finally { release(); child.kill("SIGTERM"); await joined; }
});
test("an unavailable native observation still stops and joins the original child", async () => {
    const child = spawn(process.execPath, ["-e", "process.send('ready');setInterval(()=>{},1000)"], { stdio: ["ignore", "ignore", "ignore", "ipc"] });
    const joined = once(child, "close");
    try {
        await once(child, "message");
        const observation = await finishNativeBeforeStop({ finish: async () => { throw Error("synthetic observer failure"); } }, async () => { child.kill("SIGTERM"); await joined; });
        assert.deepEqual(observation, { observed: false, reason: "native-sample-result-unavailable" });
        assert.equal(child.signalCode, "SIGTERM");
    } finally { child.kill("SIGTERM"); await joined; }
});
test("native stack observer selects one exact original group and command without fallback", () => {
    const lines = "10 20 /owned/connect serve --config /owned/server.toml\n11 30 /owned/connect serve --config /owned/server.toml\n12 20 /owned/connect sdk-stdio --config /owned/device.toml";
    assert.equal(selectOwnedServer(lines, 20, "/owned/connect serve --config /owned/server.toml"), 10);
    assert.throws(() => selectOwnedServer(lines, 40, "/owned/connect serve --config /owned/server.toml"));
    assert.throws(() => selectOwnedServer(lines + "\n13 20 /owned/connect serve --config /owned/server.toml", 20, "/owned/connect serve --config /owned/server.toml"));
});
test("platform Rust demangling exposes actual production v0 symbols without publishing raw frames", async () => {
    const symbol = "_RNvMs5_NtNtNtCs4URYBXwsqMv_5tokio7runtime8blocking4poolNtB5_5Inner3run";
    const text = `Process: connect [10]\nCall graph:\n + 12 ${symbol} (in connect) + 4 [0xabc]\nTotal number in stack:\n`;
    const names = await demangleNativeSymbols(text);
    assert.equal(names.get(symbol), "<tokio::runtime::blocking::pool::Inner>::run");
    const result = summarizeNativeSample(text, 10, names);
    assert.deepEqual(result.top_stacks, [{ symbol: names.get(symbol), stack_samples: 12 }]);
    assert.ok(!JSON.stringify(result).includes("0xabc"));
    await assert.rejects(demangleNativeSymbols("x".repeat(8 * 1024 * 1024 + 1)));
});
test("GNU crate disambiguators and LLVM Rust names have the same curated spelling", () => {
    const gnu = "<tokio[39464302fedf0c29]::runtime::blocking::pool::Inner>::run";
    const llvm = "<tokio::runtime::blocking::pool::Inner>::run";
    assert.equal(normalizeNativeSymbol(gnu), llvm);
    assert.equal(normalizeNativeSymbol(llvm), llvm);
    assert.equal(normalizeNativeSymbol("core[0123456789abcdef]::array<[u8; 16]>"), "core::array<[u8; 16]>");
});
test("native stack summary binds original PID and excludes raw paths, addresses and unknown labels", () => {
    const text = "Process: connect [10]\nPath: /private/secret/connect\nCall graph:\n" +
        "    + 30 connect_store::peer_authority::read (in connect) + 42 [0xabcdef]\n" +
        "      ! 12 sqlite3_step (in connect) + 5 [0xabcdef]\n" +
        "      ! 9 __ulock_wait2 (in libsystem_kernel) + 4 [0x12345]\n" +
        "      ! 99 private_secret (in /private/secret) + 1\nTotal number in stack:\n  50 connect_store::duplicate (in connect)\n";
    const result = summarizeNativeSample(text, 10);
    assert.equal(result.symbols_observed, 3); assert.equal(result.top_stacks[0].stack_samples, 30);
    assert.ok(!JSON.stringify(result).includes("private")); assert.ok(!JSON.stringify(result).includes("0xabcdef"));
    assert.throws(() => summarizeNativeSample(text, 11));
    assert.throws(() => summarizeNativeSample("x".repeat(8 * 1024 * 1024 + 1), 10));
});
