import { readFile } from "node:fs/promises";
// A scripted model double only. It owns no Connect credential or communications API.
export class ScriptedModel {
    constructor(script) { this.script = structuredClone(script); }
    respond(stage) {
        const result = this.script[stage];
        if (!result) throw Error("unsupported scripted model stage");
        return { ...structuredClone(result), role: "assistant", simulated: true, source: "scripted-model-fixture" };
    }
    static async load() { return new ScriptedModel(JSON.parse(await readFile(new URL("./model-script.json", import.meta.url), "utf8"))); }
}
