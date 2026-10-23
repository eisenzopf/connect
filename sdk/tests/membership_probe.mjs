import assert from 'node:assert/strict';
import {HttpClient} from '../typescript/dist/index.js';
let input=''; for await (const chunk of process.stdin) input+=chunk;
const v=JSON.parse(input), client=new HttpClient(v.base,v.token);
assert.deepEqual(await client.changeMembership(v.request),v.expected);
const page=await client.membershipInvitations(null,1); assert.ok(Array.isArray(page.invitations));
if(v.snapshot) assert.deepEqual(await client.membershipSnapshot(v.request.conversation_id),v.snapshot);
console.log(JSON.stringify({passed:true}));
