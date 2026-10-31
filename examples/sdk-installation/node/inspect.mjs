// Run from an external project with the matching candidate connect on PATH.
import {NodeClient} from '@rudeless/connect';

if(process.argv.length!==3)throw Error('Usage: node inspect.mjs PRIVATE_SDK_CONFIG');
const client=new NodeClient({config:process.argv[2]});
const options={timeoutMs:4000};
try{
    const value=await client.capabilities.refresh(options);
    process.stdout.write(JSON.stringify({profile:value.profile,feature_count:value.features.size})+'\n');
}finally{
    await client.close(options);
}
