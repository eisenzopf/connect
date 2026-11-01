import test from "node:test";
import assert from "node:assert/strict";
import { selectOwnedServer, summarizeNativeSample, demangleNativeSymbols } from "../lib/native_server_sample.mjs";
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
