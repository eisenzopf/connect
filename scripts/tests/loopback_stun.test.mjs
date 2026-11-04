import assert from 'node:assert/strict';
import dgram from 'node:dgram';
import {randomBytes} from 'node:crypto';
import {test} from 'node:test';
import {openLoopbackStun} from '../video/loopback_stun.mjs';

test('owned STUN refuses malformed data, reflects the original UDP transaction and joins its socket',async()=>{
    const fixture=await openLoopbackStun(),client=dgram.createSocket('udp4');let replies=0;
    client.on('message',()=>replies++);
    await new Promise(resolve=>client.bind(0,'127.0.0.1',resolve));
    const port=Number(fixture.url.split(':').at(-1));
    try {
        const malformed=Buffer.alloc(20);await new Promise(resolve=>client.send(malformed,port,'127.0.0.1',resolve));
        await new Promise(resolve=>setTimeout(resolve,40));assert.equal(replies,0);
        const request=Buffer.alloc(20);request.writeUInt16BE(1);request.writeUInt32BE(0x2112a442,4);randomBytes(12).copy(request,8);
        const response=new Promise((resolve,reject)=>{
            const timer=setTimeout(()=>reject(Error('original STUN response deadline')),500);
            client.once('message',(packet,source)=>{clearTimeout(timer);resolve({packet,source});});
        });
        client.send(request,port,'127.0.0.1');const {packet,source}=await response;
        assert.equal(source.port,port);assert.equal(source.address,'127.0.0.1');assert.equal(packet.length,32);
        assert.equal(packet.readUInt16BE(0),0x0101);assert.equal(packet.readUInt16BE(2),12);
        assert.deepEqual(packet.subarray(4,20),request.subarray(4,20));
        assert.equal(packet.readUInt16BE(26)^0x2112,client.address().port);
        assert.deepEqual([...packet.subarray(28)].map((value,index)=>value^request[4+index]),[127,0,0,1]);
        const stats=await fixture.close();assert.deepEqual(stats,{received:2,answered:1,malformed:1,send_errors:0,original_socket_joined:true});
        assert.deepEqual(await fixture.close(),stats);
    } finally {await fixture.close();await new Promise(resolve=>client.close(resolve));}
});
