/** Owned RFC5389 Binding fixture. Reflects the source; never proxies traffic. */
import dgram from 'node:dgram';

export async function openLoopbackStun(bindIp='127.0.0.1') {
    if(!['127.0.0.1','10.203.0.2'].includes(bindIp))throw Error('Owned local or matrix namespace address required');
    const socket=dgram.createSocket('udp4');
    const stats={received:0,answered:0,malformed:0,send_errors:0,original_socket_joined:false};
    let closing,stopped=false;
    socket.on('message',(packet,source)=>{
        if(stopped)return;
        stats.received++;
        if(stats.received>4096||packet.length<20||packet.length>2048||packet.readUInt16BE(0)!==1
            ||packet.readUInt32BE(4)!==0x2112a442||packet.readUInt16BE(2)!==packet.length-20
            ||(packet.length-20)%4!==0||source.family!=='IPv4') {stats.malformed++;return;}
        const response=Buffer.alloc(32);
        response.writeUInt16BE(0x0101,0);response.writeUInt16BE(12,2);packet.copy(response,4,4,20);
        response.writeUInt16BE(0x0020,20);response.writeUInt16BE(8,22);response[25]=1;
        response.writeUInt16BE(source.port^0x2112,26);
        const address=source.address.split('.').map(Number);
        for(let index=0;index<4;index++)response[28+index]=address[index]^packet[4+index];
        socket.send(response,source.port,source.address,error=>{if(error)stats.send_errors++;else stats.answered++;});
    });
    await new Promise((resolve,reject)=>{
        const failed=error=>{socket.removeListener('listening',ready);reject(error);};
        const ready=()=>{socket.removeListener('error',failed);resolve();};
        socket.once('error',failed);socket.once('listening',ready);socket.bind(0,bindIp);
    });
    socket.on('error',()=>{stats.send_errors++;});
    return {url:`stun:${bindIp}:${socket.address().port}`,stats,
        close:()=>closing??=(async()=>{
            stopped=true;await new Promise(resolve=>socket.close(resolve));stats.original_socket_joined=true;
            return {...stats};
        })()};
}
