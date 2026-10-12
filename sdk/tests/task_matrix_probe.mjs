// Identical matrix through the real TypeScript native and explicit HTTP clients.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {NativeClient, HttpClient, ConnectError} from '../typescript/dist/index.js';
const [ownerPath, peerPath, url, executable, mode] = process.argv.slice(2);
let input = ''; for await (const chunk of process.stdin) input += chunk;
const {credentials, ...bindings} = JSON.parse(input);
const matrix = JSON.parse(await readFile(new URL('./task_matrix.json', import.meta.url), 'utf8'));
const paths = {owner: ownerPath, peer: peerPath};
function path(value, dotted) {
    for (const key of dotted.split('.')) {
        if (value == null) return undefined;
        value = value[key];
    }
    return value;
}
function expand(value) {
    if (Array.isArray(value)) return value.map(expand);
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,expand(v)]));
    if (typeof value !== 'string') return value;
    if (/^\$\{[^}]+\}$/.test(value)) {
        assert(Object.hasOwn(bindings, value.slice(2,-1)), 'Missing binding');
        return bindings[value.slice(2,-1)];
    }
    return value.replace(/\$\{([^}]+)\}/g,(_,k)=>String(bindings[k]));
}
function verify(result, expected = {}) {
    for (const [field,value] of Object.entries(expected.paths ?? {})) assert.deepEqual(path(result,field),value,field);
    for (const [field,value] of Object.entries(expected.greater ?? {})) assert(path(result,field)>value,field);
    for (const check of expected.contains ?? []) {
        const matches = path(result,check.path).filter(item=>path(item,check.field)===check.value);
        if (check.count !== undefined) assert.equal(matches.length,check.count,check.path);
        else assert(matches.length>0,check.path);
    }
}
async function open(actor) {
    const client = mode === 'native' ? new NativeClient(paths[actor],executable) : new HttpClient(url,credentials[actor]);
    await client.negotiateTasks(actor==='owner'?'connect:agent@local':'connect:reviews@local',bindings.cid);
    return client;
}
const clients = {owner: await open('owner'),peer: await open('peer')};
try {
    for (const template of matrix.steps) {
        const step = expand(template), actor = step.actor;
        if (step.reconnect) {
            if (mode==='native') await clients[actor].close();
            clients[actor] = await open(actor);
        }
        let result;
        try { result = step.command ? await clients[actor].task(step.command) : await clients[actor].request(step.operation); }
        catch (error) {
            assert(error instanceof ConnectError,step.id);
            assert.equal(error.code,step.error,step.id);
            continue;
        }
        assert(!step.error,step.id);
        verify(result,step.expect);
        for (const [name,field] of Object.entries(step.capture ?? {})) bindings[name]=path(result,field);
    }
    console.log(`TypeScript ${mode}: ${matrix.steps.length} real-server task steps passed`);
} finally {
    if (mode==='native') for (const client of Object.values(clients)) await client.close();
}
