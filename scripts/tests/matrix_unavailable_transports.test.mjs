import {test} from 'node:test';
import assert from 'node:assert/strict';
import dgram from 'node:dgram';
import net from 'node:net';
import {setTimeout as delay} from 'node:timers/promises';
import {matrixUnavailableTransports} from '../lib/matrix_unavailable_transports.mjs';

test('unavailable transport fixture owns actual loopback endpoints, refuses TCP and joins its original sockets',{timeout:5000},async()=>{
    const fixture=await matrixUnavailableTransports(),udp=dgram.createSocket('udp4');
    let replies=0;udp.on('message',()=>replies++);
    try{
        const target=new URL(fixture.endpoints.webtransport);assert.equal(target.hostname,'127.0.0.1');
        await new Promise((resolve,reject)=>udp.send(Buffer.alloc(1200),Number(target.port),target.hostname,error=>error?reject(error):resolve()));
        const tcp=new URL(fixture.endpoints['wss-webrtc']);assert.equal(tcp.hostname,'127.0.0.1');
        await new Promise((resolve,reject)=>{const socket=net.connect({host:tcp.hostname,port:Number(tcp.port)});
            socket.once('error',reject);socket.once('close',resolve);});
        await delay(50);assert.equal(replies,0);assert.equal(fixture.observation().udp_datagrams,1);
        assert.equal(fixture.observation().tcp_connections,1);
    }finally{udp.close();await fixture.close();}
    const terminal=await fixture.close();assert.equal(terminal.original_sockets_joined,true);
    assert.equal(terminal.errors,0);assert.equal(terminal.overflow,false);
});
