import {HttpClient} from '../typescript/dist/index.js';
let input='';for await(const chunk of process.stdin)input+=chunk;const v=JSON.parse(input);const client=new HttpClient(v.base,v.token);
const c=await client.createConversation('shared-conversation',v.request);if(c.id!==v.cid||JSON.stringify(c.metadata)!==JSON.stringify(v.request.metadata)||c.receipt.operation_id!=='shared-conversation')throw Error('conversation create receipt mismatch');
const page=await client.listConversations({filter:{state:'all',identity_id:v.peer},limit:1});if(page.conversations[0].id!==c.id)throw Error('conversation list mismatch');
console.log(JSON.stringify({passed:true}));
