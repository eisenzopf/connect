#!/usr/bin/env python3
"""Isolated media/control process fixture; no server, account or native media."""
import asyncio
import base64
import json
import os
from pathlib import Path
import struct
import sys

config=json.loads(Path(sys.argv[sys.argv.index('--config')+1]).read_text())
with open(config['pids'],'a') as out:out.write(str(os.getpid())+'\n')
HEADER=struct.Struct('!4sBIII')
PROFILE='connect-sdk-media/1'
writer=None
actor=None
jobs=set()
last_rtp=struct.pack('!BBHII',0x80,0,7,1120,0x1234)+b'synthetic-packet'
last_sequence=7
lock=asyncio.Lock()


def capture(value):
    with open(config['capture'],'a') as out:out.write(json.dumps(value,separators=(',',':'))+'\n')


async def read_frame(reader):
    magic,kind,id,size,body=HEADER.unpack(await reader.readexactly(17))
    assert magic==b'C6M1' and 0<id<2**32 and size<=16384 and body<=16396
    return kind,id,json.loads(await reader.readexactly(size)),await reader.readexactly(body)


async def write_frame(kind,id,value,payload=b''):
    raw=json.dumps(value,separators=(',',':')).encode()
    async with lock:
        data=HEADER.pack(b'C6M1',kind,id,len(raw),len(payload))+raw+payload
        if config.get('mode')=='fragmented':
            for start in range(0,len(data),7):
                writer.write(data[start:start+7]);await writer.drain();await asyncio.sleep(.001)
        else:
            writer.write(data);await writer.drain()


async def released():
    while not Path(config['release']).exists():await asyncio.sleep(.005)


async def media(id,value,payload):
    global last_rtp,last_sequence
    command=value['command'];action=command['action']
    capture({'channel':'media','id':id,'request':value,'payload':base64.b64encode(payload).decode()})
    mode=config.get('mode','normal')
    if mode=='die-media':os._exit(3)
    if action=='describe':
        result={'type':'negotiation','binding':command['binding'],'source_clock':True,'stream_revision':1,
            'streams':[{'id':'audio','medium':'audio','direction':'sendrecv','codec':{'name':'pcmu','params':{'sample_rate':8000,'channels':1}},'stream_local_id':7}]}
        await write_frame(0x81,id,result)
    elif action in ('send_rtp','send_clock'):
        if action=='send_rtp':last_rtp=payload;last_sequence=command['datagram_sequence']
        if mode=='send-held':await released()
        result={'type':'submitted','binding':command['binding'],'stream_id':command['stream_id'],
            'boundary':'local-media-queue','datagram_sequence':command['datagram_sequence']}
        if mode=='foreign-reply':result['binding']={**command['binding'],'connection_id':'foreign'}
        await write_frame(0x81,id,result)
    elif action=='receive':
        if mode=='receive-held':await released()
        _,flags,seq,ts,ssrc=struct.unpack('!BBHII',last_rtp[:12])
        item={'type':'rtp','datagram_sequence':last_sequence,'header':{'payload_type':flags&127,'sequence_number':seq,'timestamp':ts,'ssrc':ssrc,'marker':bool(flags&128)}}
        body=last_rtp
        if mode=='clock':
            item={'type':'clock','datagram_sequence':8,'report':{'ssrc':1,'ntp':str(2**64-1),'rtp':2,'packets':3,'octets':4,'cname':'synthetic-source'}};body=b''
        elif mode=='gap':item={'type':'gap','dropped_packets':str(2**64-1)};body=b''
        elif mode=='bad-header':item['header']['ssrc']+=1
        result={'type':'packet','binding':command['binding'],'stream_id':command['stream_id'],'packet':item}
        await write_frame(0x82,id,result,body)
    elif action=='stats':
        counters=('received_datagrams','rtp_packets','clock_reports','malformed_datagrams','unknown_handles','overflow_drops','control_drops','binding_change_drops')
        await write_frame(0x81,id,{'type':'stats',**{k:str(2**64-1) for k in counters},'queued_packets':0,'queued_bytes':0})
    else:raise AssertionError('Unsupported fixture action')


async def serve_media(reader):
    try:
        while True:
            kind,id,value,payload=await read_frame(reader)
            assert kind==1
            task=asyncio.create_task(media(id,value,payload));jobs.add(task);task.add_done_callback(jobs.discard)
    except (asyncio.IncompleteReadError,ConnectionError):pass


async def detach():
    global actor,writer
    if actor is not None:
        actor.cancel();await asyncio.gather(actor,return_exceptions=True);actor=None
    for task in tuple(jobs):task.cancel()
    if jobs:await asyncio.gather(*jobs,return_exceptions=True)
    if writer is not None:
        writer.close();await writer.wait_closed();writer=None


async def main():
    global writer,actor
    reader=asyncio.StreamReader();protocol=asyncio.StreamReaderProtocol(reader)
    transport,_=await asyncio.get_running_loop().connect_read_pipe(lambda:protocol,sys.stdin)
    try:
        while line:=await reader.readline():
            value=json.loads(line);request=value['request'];action=request['action'];kind=action['type']
            assert value['profile']=='connect-sdk-helper/4'
            capture({'channel':'control','action':action})
            if kind=='media_attach':
                endpoint=action['attachment']['endpoint'];host,port=endpoint.rsplit(':',1)
                peer,writer=await asyncio.open_connection(host,int(port))
                token=action['attachment']['token']
                await write_frame(1,1,{'profile':PROFILE,'token':token})
                reply=await read_frame(peer)
                assert reply==(0x81,1,{'profile':PROFILE,'token':token},b'')
                actor=asyncio.create_task(serve_media(peer))
                result={'profile':PROFILE,'attached':True,'window':16,'header_bytes':17,'metadata_max_bytes':16384,'payload_max_bytes':16396}
            elif kind=='media_detach':
                await detach();result={'profile':PROFILE,'closed':True}
            elif kind=='capabilities_refresh':
                result={'profile':'connect/2','runtime_role':'rust','selected_transport':'quic','fallback_reason':None,
                    'features':['original-id-operations','native-programmatic-media','durable-events','incoming-invitations','raw-canonical','message-receipts']
                        +([] if config.get('mode')=='old-helper' else ['managed-media-ipc-v1'])}
            else:raise AssertionError('Unsupported control action')
            print(json.dumps({'profile':'connect-sdk-helper/4','id':value['id'],'phase':'request','result':{'Ok':result}}),flush=True)
    finally:
        await detach();transport.close()


asyncio.run(main())
