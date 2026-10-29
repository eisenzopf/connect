"""App-owned finite PCMU/VP8 bridge; no devices, codec children or control IO."""
import asyncio
from pathlib import Path
import struct
import time
from connect_sdk import RtpPacket, RequestOptions

MAX_FRAME=262144
MAX_FILE=4*1024*1024
MAX_RTP=1180


def invalid():
    raise ValueError('Unsupported or incomplete PCMU/VP8 example input')


def bounded(path,maximum):
    with Path(path).open('rb') as source:
        data=source.read(maximum+1)
    if len(data)>maximum:invalid()
    return data


def parse_ivf(data):
    if len(data)<32 or len(data)>MAX_FILE:invalid()
    magic,version,size,codec,width,height,rate,scale,count,reserved=struct.unpack('<4sHH4sHHIIII',data[:32])
    if (magic,version,size,codec,width,height,reserved)!=(b'DKIF',0,32,b'VP80',640,360,0) or not rate or not scale:invalid()
    offset=32;frames=[];last=-1
    while offset<len(data):
        if offset+12>len(data) or len(frames)>=90:invalid()
        size,pts=struct.unpack('<IQ',data[offset:offset+12]);offset+=12;ticks=pts*90000*scale//rate
        if not 0<size<=MAX_FRAME or offset+size>len(data) or not last<ticks<=270000:invalid()
        frames.append((ticks,data[offset:offset+size]));last=ticks;offset+=size
    if not frames or count not in (0,len(frames)):invalid()
    return frames


def ivf_header():
    return struct.pack('<4sHH4sHHIIII',b'DKIF',0,32,b'VP80',640,360,90000,1,0,0)


def frame_record(frame,first):
    timestamp,data=frame
    return struct.pack('<IQ',len(data),(timestamp-first)&0xffffffff)+data


def rtp(pt,sequence,timestamp,ssrc,payload,marker=False):
    if pt not in (0,96) or not 0<=sequence<=65535 or not 0<=timestamp<=0xffffffff or not 0<ssrc<=0xffffffff or not 0<len(payload)<=MAX_RTP-12:invalid()
    return struct.pack('!BBHII',0x80,pt|(0x80 if marker else 0),sequence,timestamp,ssrc)+payload


def packetize_vp8(encoded,timestamp,sequence,ssrc):
    if not 0<len(encoded)<=MAX_FRAME:invalid()
    return [rtp(96,(sequence+i)&65535,timestamp,ssrc,bytes([0x10 if offset==0 else 0])+encoded[offset:offset+MAX_RTP-13],offset+MAX_RTP-13>=len(encoded)) for i,offset in enumerate(range(0,len(encoded),MAX_RTP-13))]


class Vp8Assembler:
    def __init__(self):
        self.pending=None;self.source=None;self.last=None;self.needs_key=True

    def reset(self):
        self.pending=None;self.needs_key=True

    def push(self,packet,now=None):
        now=time.monotonic() if now is None else now
        if not isinstance(packet,RtpPacket) or len(packet.data)>MAX_RTP or packet.header.payload_type!=96:invalid()
        h=packet.header;p=packet.payload
        if len(p)<2 or p[0] not in (0,0x10):self.reset();invalid()
        if self.source is not None and h.ssrc!=self.source:self.reset();invalid()
        self.source=h.ssrc
        if self.pending is not None and now-self.pending['started']>0.2:self.reset()
        if p[0]==0x10:
            if self.pending is not None:self.reset()
            self.pending={'timestamp':h.timestamp,'next':h.sequence_number,'started':now,'parts':[],'bytes':0}
        f=self.pending
        if f is None:return None
        if f['timestamp']!=h.timestamp or f['next']!=h.sequence_number:self.reset();return None
        f['next']=(f['next']+1)&65535;f['bytes']+=len(p)-1
        if f['bytes']>MAX_FRAME:self.reset();invalid()
        f['parts'].append(p[1:])
        if not h.marker:return None
        self.pending=None;encoded=b''.join(f['parts']);key=encoded[0]&1==0
        if key and (len(encoded)<10 or encoded[3:6]!=b'\x9d\x01\x2a' or struct.unpack('<H',encoded[6:8])[0]&0x3fff!=640 or struct.unpack('<H',encoded[8:10])[0]&0x3fff!=360):self.reset();invalid()
        if self.last==h.timestamp or self.needs_key and not key:return None
        self.needs_key=False;self.last=h.timestamp
        return h.timestamp,encoded


def load_media(audio_path,video_path):
    audio=bounded(audio_path,24000)
    if not audio or len(audio)%160:invalid()
    return audio,parse_ivf(bounded(video_path,MAX_FILE))


async def exchange_media(send_audio,send_video,receive_audio,receive_video,inputs,output_prefix,ssrc):
    """Join every finite operation; caller owns and closes the original Client."""
    deadline=time.monotonic()+4;audio,frames=inputs
    def options():
        milliseconds=int((deadline-time.monotonic())*1000)
        if milliseconds<1:raise TimeoutError('Media example deadline')
        return RequestOptions(milliseconds)

    async def sending():
        start=time.monotonic();ai=vi=aseq=vseq=datagram=0
        while ai<len(audio)//160 or vi<len(frames):
            at=ai*0.02 if ai<len(audio)//160 else float('inf');vt=frames[vi][0]/90000 if vi<len(frames) else float('inf')
            await asyncio.sleep(max(0,min(at,vt)-(time.monotonic()-start)))
            if at<=vt:
                packet=rtp(0,aseq&65535,ai*160,ssrc,audio[ai*160:(ai+1)*160]);ai+=1;aseq+=1
                await send_audio.send_rtp(RtpPacket(datagram,packet),options());datagram+=1
            else:
                timestamp,encoded=frames[vi];vi+=1
                for packet in packetize_vp8(encoded,timestamp,vseq,ssrc+1):
                    await send_video.send_rtp(RtpPacket(datagram,packet),options());datagram+=1;vseq=(vseq+1)&65535
        return datagram

    async def receiving_audio():
        parts=[];previous=None
        while len(parts)<len(audio)//160:
            p=await receive_audio.receive_packet(options())
            if not isinstance(p,RtpPacket) or p.header.payload_type!=0 or len(p.payload)!=160 or previous is not None and p.header.sequence_number!=(previous+1)&65535:invalid()
            previous=p.header.sequence_number;parts.append(p.payload)
        return b''.join(parts)

    async def receiving_video():
        assembler=Vp8Assembler();received=[];size=32
        while len(received)<len(frames):
            p=await receive_video.receive_packet(options())
            if not isinstance(p,RtpPacket):assembler.reset();invalid()
            frame=assembler.push(p)
            if frame is not None:
                size+=12+len(frame[1])
                if size>MAX_FILE:invalid()
                received.append(frame)
        return received

    results=await asyncio.gather(sending(),receiving_audio(),receiving_video(),return_exceptions=True)
    for result in results:
        if isinstance(result,BaseException):raise result
    datagrams,captured,received=results
    with Path(str(output_prefix)+'.mulaw').open('xb') as output:output.write(captured)
    with Path(str(output_prefix)+'.ivf').open('xb') as output:
        output.write(ivf_header())
        for frame in received:output.write(frame_record(frame,received[0][0]))
    return {'audio_bytes':len(captured),'video_frames':len(received),'sent_rtp_packets':datagrams}
