// Owned loopback failures: consume QUIC datagrams without replying and reject
// TCP before TLS. No external host, certificate bypass or SDK mock is involved.
import dgram from 'node:dgram';
import net from 'node:net';

export async function matrixUnavailableTransports({cycles=1}={}) {
    if(![1,3].includes(cycles))throw Error('Declared original unavailable transport cycles');
    const udp=dgram.createSocket('udp4'),tcp=net.createServer(),connections=new Set();
    const stats={udp_datagrams:0,tcp_connections:0,errors:0,overflow:false,original_sockets_joined:false};
    let udpBound=false,tcpBound=false,closing;
    udp.on('message',()=>{if(stats.udp_datagrams<256*cycles)stats.udp_datagrams++;else stats.overflow=true;});
    udp.on('error',()=>stats.errors++);tcp.on('error',()=>stats.errors++);
    tcp.on('connection',socket=>{
        stats.tcp_connections++;if(stats.tcp_connections>16*cycles)stats.overflow=true;
        connections.add(socket);socket.on('error',()=>stats.errors++);
        socket.once('close',()=>connections.delete(socket));socket.destroy();
    });
    const close=()=>closing??=(async()=>{
        const joins=[...connections].map(socket=>new Promise(resolve=>socket.once('close',resolve)));
        if(udpBound)joins.push(new Promise(resolve=>udp.close(resolve)));
        if(tcpBound)joins.push(new Promise((resolve,reject)=>tcp.close(error=>error?reject(error):resolve())));
        await Promise.all(joins);stats.original_sockets_joined=true;return {...stats};
    })();
    try{
        await new Promise((resolve,reject)=>{udp.once('error',reject);udp.bind(0,'127.0.0.1',()=>{udp.off('error',reject);udpBound=true;resolve();});});
        await new Promise((resolve,reject)=>{tcp.once('error',reject);tcp.listen(0,'127.0.0.1',()=>{tcp.off('error',reject);tcpBound=true;resolve();});});
        return {endpoints:{webtransport:`https://127.0.0.1:${udp.address().port}/connect`,
            'wss-webrtc':`wss://127.0.0.1:${tcp.address().port}/connect`},observation:()=>({...stats}),close};
    }catch(error){await close();throw error;}
}
