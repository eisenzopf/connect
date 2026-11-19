"""Prospective Firefox-only original source-frame witness; closed metadata only.

Never regrades an old profile, fits an offset, decodes SRTP, or creates authority.
The caller still owns the original browser/capture/server joins and every gate.
"""
import copy
from array import array
from bisect import bisect_left, bisect_right
import sys
import hashlib
import ipaddress
import json
import math
import os
import re
import stat
import struct
import time
from pathlib import Path

PROFILE = "connect/native-rtc-original-source-frame/2"
MAP_PROFILE = "connect/rtc-original-video-mapping/1"
MAX_PAIRS, MAX_HEADERS = 179982, 600000
WITNESS_CYCLE_BYTES = 8 * 1024**2
CYCLE_JSON_BYTES = 32 * 1024**2
CYCLES_JSON_BYTES = 96 * 1024**2


class WitnessError(ValueError):
    pass


def need(ok, reason):
    if not ok:
        raise WitnessError(reason)


def uint(value, bits):
    return type(value) is int and 0 <= value < 1 << bits


def delta(a, b, bits=32):
    value = (a - b) % (1 << bits)
    need(value != 1 << (bits - 1), "ambiguous-modular-coordinate")
    return value if value < 1 << (bits - 1) else value - (1 << bits)


def regular(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        before = os.fstat(fd)
        need(stat.S_ISREG(before.st_mode) and 0 < before.st_size <= maximum, "regular-bounded-original-required")
        with os.fdopen(fd, "rb") as source:
            fd = None
            yield source, before
            fields=("st_dev","st_ino","st_mode","st_size","st_mtime_ns","st_ctime_ns","st_nlink","st_uid")
            need(all(getattr(os.fstat(source.fileno()),key)==getattr(before,key) for key in fields), "original-input-changed")
    finally:
        if fd is not None:
            os.close(fd)


# A contextmanager keeps the SAME original descriptor open through validation.
from contextlib import contextmanager
regular = contextmanager(regular)


def read_json(path, maximum=16 * 1024**2):
    with regular(path, maximum) as (source, _):
        raw = source.read(maximum + 1)
        need(len(raw) <= maximum, "json-cap")
        return json.loads(raw), hashlib.sha256(raw).hexdigest()


PUBLICATION_PROFILE = "connect/native-publication-binding/1"
PUBLICATION_MARKER = b"Connect native publication binding"


class PublicationMappings(list):
    """Closed original log records; never a runtime authority or grant."""
    def __init__(self, values=(), source_bindings=()):
        super().__init__(values)
        self.source_bindings = list(source_bindings)


def publication_record(value, original_server_pid):
    need(isinstance(value, dict) and value.get('profile') == PUBLICATION_PROFILE
         and value.get('process') == original_server_pid, 'publication-original-process-unproved')
    need(value.get('complete') is True,'publication-incomplete')
    need(value.get('phase') in ('initial-ready','replacement-published'), 'publication-phase-unproved')
    for key in ('public_connection_id','runtime_connection_id','conversation_id','session_id','participant_id',
                'offer_record_id','publication_operation_id'):
        need(isinstance(value.get(key), str) and re.fullmatch(r'[A-Za-z0-9_-]{1,256}', value[key]),
             'publication-typed-identity-invalid')
    for key in ('public_connection_generation','membership_epoch'):
        need(type(value.get(key)) is int and 0 < value[key] < 2**64, 'publication-public-generation-invalid')
    streams=value.get('streams')
    need(isinstance(streams,list) and 0<len(streams)<=32, 'publication-stream-bound')
    seen=set()
    for stream in streams:
        need(isinstance(stream,dict) and all(isinstance(stream.get(key),str)
             and re.fullmatch(r'[A-Za-z0-9_-]{1,256}',stream[key]) for key in ('logical_id','transport_id')),
             'publication-stream-identity-invalid')
        need(stream['logical_id'] not in seen, 'publication-duplicate-logical-stream');seen.add(stream['logical_id'])
        need(stream.get('medium') in ('audio','video','screen','data') and stream.get('direction') in ('send','receive','both')
             and type(stream.get('source_ready')) is bool and type(stream.get('sink_ready')) is bool,
             'publication-stream-state-invalid')
        need(all(type(stream.get(key)) is int and 0<stream[key]<2**64 for key in
                 ('core_stream_generation','core_connection_generation')), 'publication-core-generation-invalid')
    need(len(json.dumps(value,separators=(',',':'),ensure_ascii=False).encode('utf8'))<=16*1024,
         'publication-record-byte-bound')
    return value


def mapping_summaries(path, original_server_pid, deadline=None):
    result = PublicationMappings()
    digest = hashlib.sha256()
    message = b"RTC original video mapping witness"
    messages = (message,PUBLICATION_MARKER)
    source_public_ids,source_runtime_ids=set(),set()
    selected_cap, chunk_cap = 256 * 1024, 64 * 1024

    def accept(raw):
        # The actual server uses tracing's normal text formatter. Its optional
        # SGR field styling is presentation only; the JSON witness stays exact.
        line = re.sub(rb"\x1b\[[0-9;]*m", b"", raw)
        if PUBLICATION_MARKER in line:
            at=line.find(b'binding=')
            need(at>=0,'publication-export-format')
            value,_=json.JSONDecoder().raw_decode(line[at+len(b'binding='):].decode('utf8'))
            value=publication_record(value,original_server_pid)
            need(len(result.source_bindings)<48,'publication-global-record-bound')
            need(value['public_connection_id'] not in source_public_ids and value['runtime_connection_id'] not in source_runtime_ids,
                 'publication-duplicate-or-ambiguous-namespace')
            source_public_ids.add(value['public_connection_id']);source_runtime_ids.add(value['runtime_connection_id'])
            result.source_bindings.append(value)
            return
        marker = b'witness={"profile":"connect/rtc-original-video-mapping/1"'
        at = line.find(marker)
        need(at >= 0, "mapping-export-format")
        value, _ = json.JSONDecoder().raw_decode(line[at + len(b"witness="):].decode("utf8"))
        need(len(result) < 128 and value.get("profile") == MAP_PROFILE, "mapping-summary-cap")
        need(value.get("process") == original_server_pid and value.get("original_timeline_retired") is True,
             "mapping-original-owner-mismatch")
        epochs = value.get("epochs")
        need(isinstance(epochs, list) and 0 <= len(epochs) <= 32, "mapping-epoch-cap")
        need([e.get("ordinal") for e in epochs] == list(range(len(epochs))), "mapping-epoch-order")
        result.append(value)

    with regular(path, 256 * 1024**2) as (source, _):
        line, line_size, selected, tail = bytearray(), 0, False, b""
        while chunk := source.readline(chunk_cap):
            if deadline is not None: need(time.monotonic() < deadline, "closed-metadata-time-bound")
            digest.update(chunk)
            line_size += len(chunk)
            search = tail + chunk
            selected |= any(marker in search for marker in messages)
            tail = search[-(max(map(len,messages)) - 1):]
            if line_size <= selected_cap:
                line.extend(chunk)
            else:
                # Unrelated audio-facts records can exceed the selected-witness
                # cap. Consume/hash them without retaining their payload. A
                # witness marker anywhere in such a record still fails closed.
                line.clear()
            need(not selected or line_size <= selected_cap, "mapping-line-cap")
            if chunk.endswith(b"\n"):
                if selected:
                    accept(bytes(line))
                line, line_size, selected, tail = bytearray(), 0, False, b""
        if selected:
            accept(bytes(line))
    need(result and len({v["timeline"] for v in result}) == len(result), "mapping-missing-or-reexported")
    return result, digest.hexdigest()


class StatsIndex:
    def __init__(self, cycle, deadline=None):
        self.cycle = cycle
        pipeline = cycle['decoded_media']['rtc_media_pipeline']
        self.samples = pipeline['samples']
        need(isinstance(self.samples, list) and len(self.samples) <= 256, 'bounded-original-statistics')
        self.requested = array('d', (s['requested_seconds'] for s in self.samples))
        self.completed = array('d', (s['completed_seconds'] for s in self.samples))
        need(all(math.isfinite(v) for v in self.requested) and all(math.isfinite(v) for v in self.completed)
             and all(a <= b for a,b in zip(self.requested,self.requested[1:]))
             and all(a <= b for a,b in zip(self.completed,self.completed[1:])), 'original-statistics-clock')
        self.boundaries = pipeline.get('binding_boundaries')
        self.receiver_owners, self.cache, self.roles = {}, {}, {}
        self.publications = None
        self.boundary_bytes_checked=False
        self.deadline = time.monotonic()+30 if deadline is None else deadline
        for sample in self.samples:
            for group in sample['peers']:
                for row in group:
                    tag=row.get('original_track') or {}
                    role=self.native_role(row)
                    if role is not None and not row.get('original_track_refusal') and self.tag_matches_role(tag,role):
                        key=(row.get('connection_id'),row.get('type'),tag.get('stream_id'),tag.get('track_id'))
                        # The first exact original association is immutable.
                        # A later conflicting row cannot teach a new role.
                        self.roles.setdefault(key,role)
                        need(len(self.roles)<=128,'bounded-original-native-roles')
                    if row.get('type')=='inbound-rtp' and row.get('kind')=='video' and isinstance(tag.get('stream_id'),str):
                        self.receiver_owners.setdefault(tag['stream_id'],set()).add(tag.get('connection_id'))
        if isinstance(self.boundaries,dict):
            for peer in self.boundaries.get('peers',[]):
                for name in ('public_binding','terminal_binding'):
                    tag=(((peer.get(name) or {}).get('bindings') or {}).get('receive') or {}).get('original_track') or {}
                    if isinstance(tag.get('stream_id'),str):
                        self.receiver_owners.setdefault(tag['stream_id'],set()).add(tag.get('connection_id'))

    @staticmethod
    def native_role(row):
        role=row.get('native_rtp_binding')
        if not isinstance(role,dict) or role.get('profile')!='connect/rtc-native-rtp-role/1' or role.get('complete') is not True:return None
        expected_direction='receive' if row.get('type')=='inbound-rtp' else 'send'
        if role.get('connection_id')!=row.get('connection_id') or type(role.get('peer_index')) is not int or not 0<=role['peer_index']<16 or role.get('peer_index')!=row.get('peer_index') or role.get('direction')!=expected_direction or role.get('reason') is not None:return None
        mid,index,track=role.get('native_mid'),role.get('transceiver_index'),role.get('native_track_id')
        if not isinstance(mid,str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,64}',mid) or type(index) is not int or not 0<=index<4 or not isinstance(track,str) or not 0<len(track)<=256:return None
        if ('native_mid' in row and row['native_mid']!=mid) or (row.get('native_track_identifier') is not None and row['native_track_identifier']!=track):return None
        return (mid,index,track)

    @staticmethod
    def tag_matches_role(tag,role):
        return (isinstance(tag,dict) and isinstance(tag.get('stream_id'),str)
                and (tag.get('mid'),tag.get('transceiver_index'),tag.get('track_id'))==role)

    def expected_role(self,owner,direction,stream,track):
        values={value for (o,d,s,t),value in self.roles.items() if o==owner and d==direction
                and (stream is None or s==stream) and (track is None or t==track)}
        return next(iter(values)) if len(values)==1 else None

    def select_rows(self,rows,owner,direction,stream,track):
        expected=self.expected_role(owner,direction,stream,track)
        primary_pts={r['codec']['payload_type'] for r in rows if self.native_role(r)==expected and expected is not None
            and (r.get('codec') or {}).get('mime_type','').lower()=='video/vp8'
            and uint((r.get('codec') or {}).get('payload_type'),7)}
        selected=[]
        for row in rows:
            role=self.native_role(row)
            # Unknown or contradictory rows cannot be fitted to another role.
            if role is None or expected is None or row.get('original_track_refusal'):
                selected.append(row);continue
            tag=row.get('original_track')
            if tag is not None and not self.tag_matches_role(tag,role):
                selected.append(row);continue
            if role!=expected and isinstance(tag,dict) and ((stream is not None and tag.get('stream_id')==stream)
                    or (track is not None and tag.get('track_id')==track)):
                selected.append(row);continue
            if role[0]!=expected[0] and role[1]!=expected[1]:continue
            codec=row.get('codec') or {}
            if role==expected and (row.get('native_rtp_binding') or {}).get('codec_dictionary_type')=='codec' and codec.get('mime_type','').lower()=='video/rtx' and codec.get('clock_rate')==90000 and uint(codec.get('payload_type'),7) and len(primary_pts)==1 and codec.get('associated_payload_type') in primary_pts and codec['payload_type'] not in primary_pts:continue
            selected.append(row)
        return selected

    def rows(self, number, owner, direction, stream=None, track=None):
        rows=[r for group in self.samples[number]['peers'] for r in group if r.get('connection_id')==owner
              and r.get('kind')=='video' and r.get('type')==direction]
        return self.select_rows(rows,owner,direction,stream,track)

    def endpoint(self,number,owner,direction,stream=None,track=None):
        rows=self.rows(number,owner,direction,stream,track)
        return bool(rows) and (len(rows)!=1 or rows[0].get('original_track') is not None)

    def snapshot(self, number, owner, direction, stream, track, ssrc, allow_unbound=False):
        return self.snapshot_rows(self.rows(number,owner,direction,stream,track), owner, direction, stream, track, ssrc, allow_unbound)

    def snapshot_rows(self,rows,owner,direction,stream,track,ssrc,allow_unbound=False):
        need(len(rows)==1,'owner-statistics-ambiguous')
        row=rows[0];original=row.get('original_track')
        need(not row.get('original_track_refusal'),'original-native-role-contradiction')
        if 'native_rtp_binding' in row:
            role=self.native_role(row)
            need(role is not None,'original-native-role-unproved')
            expected=self.expected_role(owner,direction,stream,track)
            need(expected is not None and role==expected,'original-native-role-changed')
            need(original is None or self.tag_matches_role(original,role),'original-native-role-contradiction')
        unbound=allow_unbound and original is None
        if not unbound:
            original=original or {}
            need(original.get('connection_id')==owner and isinstance(original.get('track_id'),str)
                 and isinstance(original.get('stream_id'),str),'original-peer-track-unproved')
            need((stream is None or original['stream_id']==stream) and (track is None or original['track_id']==track),
                 'original-track-or-ssrc-changed')
            if direction=='inbound-rtp':
                need(self.receiver_owners.get(original['stream_id'])=={owner},'ambiguous-observed-target-owner')
        need(ssrc is None or row.get('ssrc')==ssrc,'original-track-or-ssrc-changed')
        path,codec=row.get('selected_path') or {},row.get('codec') or {}
        need(codec.get('mime_type','').lower()=='video/vp8' and codec.get('clock_rate')==90000
             and uint(codec.get('payload_type'),7) and uint(row.get('ssrc'),32),'original-vp8-clock-profile')
        need(path.get('nominated') is True and path.get('succeeded') is True,'selected-path-unproved')
        for end in (path.get('local'),path.get('remote')):
            need(isinstance(end,dict) and end.get('protocol')=='udp' and uint(end.get('port'),16)
                 and end['port']>0,'selected-udp-tuple-unavailable')
            ipaddress.ip_address(end['address'])
        # Selection metadata is diagnostic-only on the ordinary numeric path.
        # Read the SAME raw row explicitly only for the typed privacy fallback.
        path={key:item for key,item in path.items() if key not in
              ('transport_epoch','selected_candidate_pair_changes')}
        return dict(track=original,path=path,ssrc=row['ssrc'],codec=codec,peer_index=row.get('peer_index'))

    def retirement_high(self,owner,direction,first,last,stream,track,ssrc,lo,hi):
        if direction!='inbound-rtp' or self.boundaries is None:return None
        peers=self.boundaries.get('peers',[])
        selected=[peer for peer in peers if peer.get('connection_id')==owner and peer.get('receiver_retirement_statistics') is not None]
        if not selected:return None
        need(len(selected)==1 and len(peers)<=16 and self.boundaries.get('profile')=='connect/rtc-native-binding-boundaries/1'
             and self.boundaries.get('complete') is True,'retirement-statistics-owner-ambiguous')
        record=selected[0]['receiver_retirement_statistics']
        need(isinstance(record,dict) and record.get('profile')=='connect/rtc-receiver-retirement-statistics/1'
             and record.get('connection_id')==owner and record.get('peer_index')==selected[0].get('peer_index'), 'retirement-statistics-original-owner')
        # No data from a pending/rejected/unsupported query can become an endpoint.
        # The unchanged typed native fallback may still provide its own proof.
        if record.get('state')!='fulfilled':return None
        need(len(json.dumps(record,separators=(',',':')).encode())<=8192,'retirement-statistics-byte-bound')
        need(record.get('complete') is True and record.get('native_close_returned') is True and record.get('native_closed') is True,
             'retirement-statistics-incomplete')
        origin=self.boundaries.get('origin_ms');time_origin=record.get('time_origin_ms')
        requested=record.get('requested_at_ms');completed=record.get('completed_at_ms');closed=record.get('native_closed_at_ms')
        need(all(isinstance(v,(int,float)) and not isinstance(v,bool) and math.isfinite(v) and v>=0 for v in (origin,time_origin,requested,completed,closed))
             and requested<=closed<=completed,'retirement-statistics-clock')
        requested_seconds=(requested-origin)/1000
        if requested_seconds<last:return None # Never a public-head or earlier sample.
        need(requested_seconds-last<=2,'owner-statistics-too-distant')
        lows=[i for i in range(max(0,lo+1)) if self.endpoint(i,owner,direction,stream,track)]
        if not lows:return None # A real opposite low endpoint is still mandatory.
        actual=lows[-1];need(first-self.completed[actual]<=2,'owner-statistics-too-distant')
        rows=record.get('rows');need(isinstance(rows,list) and len(rows)==1,'retirement-statistics-ambiguous')
        raw=rows[0];need(raw.get('connection_id')==owner and raw.get('peer_index')==record.get('peer_index')
            and raw.get('type')==direction and raw.get('kind')=='video','retirement-statistics-original-owner')
        value=self.snapshot_rows(rows,owner,direction,stream,track,ssrc)
        tag=value['track'];need(raw.get('native_track_identifier')==tag['track_id'] and raw.get('native_mid')==tag.get('mid'),
                              'retirement-statistics-native-track-contradiction')
        sampled=raw.get('stats_timestamp_ms')
        need(isinstance(sampled,(int,float)) and not isinstance(sampled,bool) and math.isfinite(sampled)
             and time_origin+origin+last*1000<=sampled<=time_origin+completed,'retirement-statistics-native-time-does-not-cover-pair')
        # Preserve EVERY available regular row and every unexplained earlier
        # disappearance in the original missing bracket; no sparse global poll.
        for i in range(actual,max(actual,min(len(self.samples)-1,hi))+1):
            if self.rows(i,owner,direction,stream,track):
                need(self.snapshot(i,owner,direction,stream,track,ssrc)==value,'contradictory-regular-statistics')
            else:need(self.completed[i]>=requested_seconds,'retirement-cannot-explain-earlier-missing-statistics')
        proof=dict(profile=record['profile'],peer_index=record['peer_index'],connection_id=owner,direction=direction,
            requested_at_ms=requested,completed_at_ms=completed,native_closed_at_ms=closed,time_origin_ms=time_origin,
            stats_timestamp_ms=sampled,origin_ms=origin,actual_rtp_snapshot=actual,
            record_sha256=hashlib.sha256(json.dumps(record,separators=(',',':'),sort_keys=True).encode()).hexdigest())
        return {**value,'native_retirement_statistics':proof}

    def get(self,owner,direction,first,last,*,stream=None,track=None,ssrc=None,coordinates=None):
        need(time.monotonic()<self.deadline,'closed-metadata-time-bound')
        lo,hi=bisect_right(self.completed,first)-1,bisect_left(self.requested,last)
        key=(owner,direction,lo,hi,stream,track,ssrc)
        if key in self.cache:
            value=self.cache[key]
            retirement=value.get('native_retirement_statistics') or {}
            if 'native_binding_boundary' not in value and retirement.get('profile')!='connect/rtc-receiver-retirement-statistics/1':
                need(first-self.completed[lo]<=2 and self.requested[hi]-last<=2,'owner-statistics-too-distant')
            self.coordinate_bound(value,first,last,coordinates);return value
        need(len(self.cache)<16384,'original-statistics-cache-bound')
        # The normal actual-snapshot bracket remains preferred and unchanged.
        if 0<=lo<=hi<len(self.samples) and all(self.endpoint(i,owner,direction,stream,track) for i in range(lo,hi+1)):
            need(first-self.completed[lo]<=2 and self.requested[hi]-last<=2,'owner-statistics-too-distant')
            values=[self.snapshot(i,owner,direction,stream,track,ssrc) for i in range(lo,hi+1)]
            need(all(v==values[0] for v in values),'source-or-receiver-state-changed')
            self.cache[key]=values[0];return values[0]
        if direction=='inbound-rtp':
            highs=[i for i in range(hi,len(self.samples)) if self.endpoint(i,owner,direction,stream,track)]
            if not highs:
                retirement=self.retirement_high(owner,direction,first,last,stream,track,ssrc,lo,hi)
                if retirement is not None:self.cache[key]=retirement;return retirement
        need(self.boundaries is not None,'owner-statistics-ambiguous' if 0<=lo<=hi<len(self.samples) else 'owner-statistics-do-not-bracket-pair')
        # A boundary NEVER manufactures both endpoints. One original, complete,
        # actual RTP snapshot must remain on the opposite side of the pair.
        lows=[i for i in range(max(0,lo+1)) if self.endpoint(i,owner,direction,stream,track)]
        highs=[i for i in range(hi,len(self.samples)) if self.endpoint(i,owner,direction,stream,track)]
        have_lo=bool(lows) and first-self.completed[lows[-1]]<=2
        have_hi=bool(highs) and self.requested[highs[0]]-last<=2
        need(have_lo != have_hi,'native-boundary-needs-one-real-rtp-endpoint')
        public=have_hi;actual=highs[0] if public else lows[-1]
        value=self.snapshot(actual,owner,direction,stream,track,ssrc)
        anchor,provenance=self.anchor(owner,direction,first,last,value,public,coordinates,actual)
        captured=(anchor['captured_at_ms']-self.boundaries['origin_ms'])/1000
        # Every available regular row across the original missing bracket is
        # still authoritative. Missing interior rows cannot become a boundary.
        start=min(actual,max(0,lo));end=max(actual,min(len(self.samples)-1,hi))
        for i in range(start,end+1):
            if self.rows(i,owner,direction,stream,track):
                if provenance.get('native_path_kind')=='prflx-address-redacted':
                    need(self.selection_epoch(i,owner,direction,value['peer_index'],stream,track)==provenance['transport_epoch'],
                         'contradictory-regular-selection-epoch')
                observed=self.snapshot(i,owner,direction,stream,track,ssrc,allow_unbound=True)
                if observed['track'] is not None:
                    need(observed==value,'contradictory-regular-statistics');continue
                need(public and direction=='inbound-rtp','null-tag-is-not-a-public-receiver-head')
                # A prepared peer can have actual RTP rows before its exact
                # receive tag is installed at public activation. No identity is
                # invented for that row: all available real fields must agree.
                need(all(observed[key]==value[key] for key in ('path','ssrc','codec','peer_index')),
                     'contradictory-unbound-regular-statistics')
            if public:
                need(self.samples[i]['requested_seconds']<=captured,'public-cannot-explain-later-missing-statistics')
            else:
                need(self.samples[i]['completed_seconds']>=captured,'terminal-cannot-explain-earlier-missing-statistics')
        provenance['actual_rtp_snapshot']=actual
        result={**value,'native_binding_boundary':provenance}
        self.cache[key]=result;return result

    def coordinate_bound(self,value,first,last,coordinates):
        retirement=value.get('native_retirement_statistics')
        if retirement is not None:
            requested=(retirement['requested_at_ms']-retirement['origin_ms'])/1000
            need(last<=requested and requested-last<=2 and first-self.completed[retirement['actual_rtp_snapshot']]<=2,
                 'owner-statistics-too-distant')
            need(retirement['time_origin_ms']+retirement['origin_ms']+last*1000<=retirement['stats_timestamp_ms']<=retirement['time_origin_ms']+retirement['completed_at_ms'],
                 'retirement-statistics-native-time-does-not-cover-pair')
        proof=value.get('native_binding_boundary')
        if proof is None:return
        public=proof['profile']=='connect/rtc-native-public-binding/1'
        actual=proof['actual_rtp_snapshot']
        need((self.requested[actual]-last if public else first-self.completed[actual])<=2,'owner-statistics-too-distant')
        captured=(proof['captured_at_ms']-self.boundaries['origin_ms'])/1000
        done=(proof['captured_done_at_ms']-self.boundaries['origin_ms'])/1000
        need(abs(captured-(first if public else last))<=2,'native-binding-too-distant')
        if public:need(done<=first,'public-binding-after-pair')
        if proof['direction']=='receive':
            need(isinstance(coordinates,(list,tuple)) and len(coordinates)==2 and all(uint(v,32) for v in coordinates),
                 'native-binding-pair-coordinate-missing')
            reported=proof['reported_rtp_timestamp']
            need((delta(coordinates[0],reported)>=0 if public else delta(reported,coordinates[1])>=0),
                 'native-binding-reported-coordinate-does-not-cover-pair')

    def selection_epoch(self,number,owner,direction,peer_index,stream=None,track=None):
        rows=self.rows(number,owner,direction,stream,track)
        need(len(rows)==1,'owner-statistics-ambiguous')
        path=rows[0].get('selected_path') or {}
        epoch=path.get('transport_epoch')
        self.check_selection_epoch(epoch,peer_index)
        count=path.get('selected_candidate_pair_changes')
        need(uint(count,32) and count==epoch['selection_count'],
             'native-selection-statistics-counter-unproved')
        return epoch

    @staticmethod
    def check_selection_epoch(epoch,peer_index):
        keys={'profile','transport_id','armed_before_selection','initial_selection_count',
              'selection_count','state_version'}
        need(isinstance(epoch,dict) and set(epoch)==keys
             and epoch.get('profile')=='connect/rtc-native-selection-epoch/1'
             and isinstance(epoch.get('transport_id'),str)
             and re.fullmatch(r'p'+str(peer_index)+r't[1-4]',epoch['transport_id'])
             and epoch.get('armed_before_selection') is True
             and type(epoch.get('initial_selection_count')) is int and epoch['initial_selection_count']==0
             and uint(epoch.get('selection_count'),32) and uint(epoch.get('state_version'),32)
             and 1<=epoch['selection_count']<=epoch['state_version']<=128, 'native-selection-epoch-unproved')

    def anchor(self,owner,direction,first,last,value,public,coordinates,actual):
        data=self.boundaries
        need(isinstance(data,dict) and data.get('profile')=='connect/rtc-native-binding-boundaries/1'
             and data.get('complete') is True and isinstance(data.get('peers'),list) and len(data['peers'])<=16,
             'native-binding-collection-incomplete')
        if not self.boundary_bytes_checked:
            for peer in data['peers']:
                for name in ('public_binding','terminal_binding'):
                    record=peer.get(name)
                    if record is not None:
                        need(len(json.dumps(record,separators=(',',':'),ensure_ascii=False).encode('utf-8'))<=32768,
                             'native-binding-record-byte-bound')
            need(time.monotonic()<self.deadline,'closed-metadata-time-bound')
            self.boundary_bytes_checked=True
        origin=data.get('origin_ms')
        need(type(origin) in (int,float) and math.isfinite(origin)
             and origin==self.cycle['programme']['source_programme']['originMs'],'native-binding-original-clock-unproved')
        found=[entry for entry in data['peers'] if entry.get('connection_id')==owner]
        need(len(found)==1,'native-binding-original-peer-ambiguous')
        entry=found[0];name='public_binding' if public else 'terminal_binding'
        a=entry.get(name) or {};profile='connect/rtc-native-'+('public' if public else 'terminal')+'-binding/1'
        need(a.get('profile')==profile and a.get('complete') is True and a.get('connection_id')==owner
             and uint(entry.get('peer_index'),8) and entry['peer_index']<16 and a.get('peer_index')==entry['peer_index']
             and value['peer_index']==entry['peer_index'],'native-binding-original-peer-unproved')
        times=[a.get(k) for k in ('captured_at_ms','captured_done_at_ms')]
        need(all(type(v) in (int,float) and math.isfinite(v) for v in times) and 0<=times[0]<=times[1],
             'native-binding-clock-invalid')
        captured,done=((v-origin)/1000 for v in times)
        need(abs(captured-(first if public else last))<=2,'native-binding-too-distant')
        if public:
            need(done<=first,'public-binding-after-pair')
            ledger=[]
            for record in self.cycle['decoded_media'].get('source_frame_public_owners',[]):
                for interval in public_intervals(self.cycle,record['route']):
                    if interval['owner']==owner:ledger.append((record['route'],interval))
            need(len(ledger)==1,'public-binding-owner-ledger-ambiguous')
            route,interval=ledger[0];link=interval.get('native_public_binding') or {}
            need(link.get('profile')=='connect/rtc-validated-public-binding/1' and link.get('complete') is True
                 and link.get('route')==route and link.get('connection_id')==owner and link.get('previous')==interval['previous']
                 and link.get('operation_id')==interval['operation_id'] and link.get('cut_ms')==interval['start_ms']
                 and link.get('basis')==interval['basis'] and link.get('peer_index')==entry['peer_index']
                 and link.get('captured_at_ms')==times[0] and link.get('captured_done_at_ms')==times[1],
                 'validated-public-binding-operation-unproved')
            need(interval['operation_id'] is None or captured>=interval['start_ms']/1000,'public-binding-predates-original-cut')
        else:
            closed=a.get('native_closed_at_ms')
            need(a.get('native_close_observed') is True and type(closed) in (int,float)
                 and math.isfinite(closed) and closed>=times[1],'terminal-original-close-unproved')
        role='receive' if direction=='inbound-rtp' else 'send';binding=(a.get('bindings') or {}).get(role) or {}
        tag=binding.get('original_track') or {}
        need(binding.get('complete') is True and binding.get('track_state') in ('live','ended') and tag==value['track']
             and uint(tag.get('transceiver_index'),8) and isinstance(tag.get('mid'),str)
             and re.fullmatch(r'[A-Za-z0-9_-]{1,64}',tag['mid']),'native-binding-original-tag-or-mid-changed')
        path=binding.get('selected_udp_pair') or {}
        need(path.get('basis')=='RTCIceTransport.getSelectedCandidatePair' and path.get('ice_state') in ('connected','completed')
             and path.get('dtls_state')=='connected','native-binding-selected-path-unproved')
        partial=path.get('kind')=='prflx-address-redacted'
        for side in ('local','remote'):
            aend,bend=path.get(side) or {},value['path'][side]
            need(aend.get('protocol')==bend.get('protocol')=='udp' and aend.get('port')==bend.get('port'),
                 'native-binding-selected-path-changed')
            if partial and side=='remote':
                need(aend.get('address') is None and 'address' in aend
                     and aend.get('type')=='prflx' and aend.get('candidate_empty') is True,
                     'native-binding-redaction-unproved')
            else:
                need(str(ipaddress.ip_address(aend['address']))==str(ipaddress.ip_address(bend['address'])),
                     'native-binding-selected-path-changed')
        privacy={}
        if partial:
            epoch=path.get('transport_epoch')
            self.check_selection_epoch(epoch,entry['peer_index'])
            need(self.selection_epoch(actual,owner,direction,entry['peer_index'],value['track'].get('stream_id'),value['track'].get('track_id'))==epoch,
                 'native-binding-selection-epoch-changed')
            # This is a relation to a real complete Stats endpoint, never a
            # replacement native address or a synthetic getStats sample.
            privacy=dict(native_path_kind='prflx-address-redacted',transport_epoch=epoch)
        codec=value['codec']
        for side in ('local','remote'):
            declarations=(binding.get('codec_declarations') or {}).get(side)
            need(isinstance(declarations,list) and len(declarations)<=32 and len([d for d in declarations
                 if d.get('payload_type')==codec['payload_type'] and d.get('mime_type','').lower()==codec['mime_type'].lower()
                 and d.get('clock_rate')==codec['clock_rate']])==1,'native-binding-codec-declarations-changed')
        if role=='receive':
            sync=binding.get('sync_source') or {}
            need(sync.get('source')==value['ssrc'] and uint(sync.get('rtpTimestamp'),32)
                 and sync.get('timestamp_clock')=='native-uninterpreted' and type(sync.get('timestamp')) in (int,float)
                 and math.isfinite(sync['timestamp']),'native-binding-receiver-source-unproved')
            need(isinstance(coordinates,(list,tuple)) and len(coordinates)==2 and all(uint(v,32) for v in coordinates),
                 'native-binding-pair-coordinate-missing')
            need((delta(coordinates[0],sync['rtpTimestamp'])>=0 if public else delta(sync['rtpTimestamp'],coordinates[1])>=0),
                 'native-binding-reported-coordinate-does-not-cover-pair')
        else:
            need(uint(binding.get('primary_ssrc'),32) and binding['primary_ssrc']==value['ssrc'],
                 'native-binding-sender-primary-changed')
        return a,dict(profile=profile,connection_id=owner,peer_index=entry['peer_index'],direction=role,
                      captured_at_ms=times[0],captured_done_at_ms=times[1],
                      reported_rtp_timestamp=(binding.get('sync_source') or {}).get('rtpTimestamp'),**privacy)


def stable_stats(cycle, owner, direction, first, last, *, stream=None, track=None, ssrc=None):
    return StatsIndex(cycle).get(owner, direction, first, last, stream=stream, track=track, ssrc=ssrc)


def epoch_for_pair(summaries, receiver, first_ts, last_ts):
    found = []
    for summary in summaries:
        epochs = summary["epochs"]
        for i, epoch in enumerate(epochs):
            if epoch.get("target_ssrc") != receiver["ssrc"] or epoch.get("target_logical") != receiver["track"]["stream_id"]:
                continue
            if epoch.get("target_connection") is not None and epoch["target_connection"] != receiver["track"]["connection_id"]:
                continue
            need(summary.get("complete") is True and summary.get("omitted")==summary.get("invalid")==0,"selected-mapping-incomplete")
            need(all(isinstance(epoch.get(key), str) and 0 < len(epoch[key]) <= 256
                     and re.fullmatch(r'[A-Za-z0-9_-]+', epoch[key]) for key in
                     ('source_connection','source_logical','source_transport','target_logical','target_transport')),
                 'mapping-typed-id-invalid')
            need(all(type(epoch.get(k)) is int and 0 < epoch[k] < 2**64 for k in
                     ("source_generation","target_generation","target_connection_generation")),"mapping-generation-invalid")
            need(all(uint(epoch.get(k), bits) for k, bits in (("input_sequence", 16), ("output_sequence", 16),
                 ("input_timestamp", 32), ("output_timestamp", 32), ("sequence_offset", 16), ("timestamp_offset", 32))),
                 "mapping-coordinate-invalid")
            need((epoch['input_sequence'] + epoch['sequence_offset']) % 65536 == epoch['output_sequence']
                 and (epoch['input_timestamp'] + epoch['timestamp_offset']) % 2**32 == epoch['output_timestamp'],
                 'mapping-coordinate-offset-mismatch')
            if delta(first_ts, epoch["output_timestamp"]) < 0:
                continue
            next_epoch = epochs[i + 1] if i + 1 < len(epochs) else None
            if next_epoch is not None and delta(last_ts, next_epoch["output_timestamp"]) >= 0:
                continue
            need(epoch["target_pt"] == receiver["codec"]["payload_type"], "target-profile-mismatch")
            selected = dict(epoch)
            selected["_end_output_timestamp"] = next_epoch["output_timestamp"] if next_epoch is not None else None
            found.append((summary["timeline"], selected))
    need(len(found) == 1, "mapping-missing-or-ambiguous")
    return found[0]


def packet_flow(stats, outbound):
    a, b = stats["path"]["local"], stats["path"]["remote"]
    if not outbound:
        a, b = b, a
    return (str(ipaddress.ip_address(a["address"])), a["port"],
            str(ipaddress.ip_address(b["address"])), b["port"], stats["ssrc"], stats["codec"]["payload_type"])


def capture_headers(path, capture, windows, *, cycles=1, deadline=None, auxiliary_bytes=0):
    need(capture.get("observed") is True and capture.get("original_joined") is True
         and capture.get("exit_code") == 0 and not capture.get("forced")
         and capture.get("kernel_dropped_packets") == 0, "original-capture-not-complete")
    count = 0
    deadline = time.monotonic() + 30 if deadline is None else deadline
    kept = HeaderIndex(200000 * cycles, deadline, auxiliary_bytes)
    flows = {tuple(flow) for flow, _, _ in windows}
    with regular(path, 256 * 1024**2 * cycles) as (source, st):
        need(st.st_size == capture.get("pcap_bytes"), "capture-size-changed")
        digest_state = hashlib.sha256()
        while chunk := source.read(64 * 1024):
            need(time.monotonic() < deadline, "closed-metadata-time-bound")
            digest_state.update(chunk)
        digest = digest_state.hexdigest()
        need(digest == capture.get("pcap_sha256"), "capture-digest-changed")
        source.seek(0)
        head = source.read(24)
        formats = {b"\xd4\xc3\xb2\xa1": ("<", 1000000), b"\xa1\xb2\xc3\xd4": (">", 1000000),
                   b"\x4d\x3c\xb2\xa1": ("<", 1000000000), b"\xa1\xb2\x3c\x4d": (">", 1000000000)}
        need(len(head) == 24 and head[:4] in formats, "classic-pcap-required")
        order, resolution = formats[head[:4]]
        major, minor, _, _, snaplen, link = struct.unpack(order + "HHiIII", head[4:])
        need((major, minor) == (2, 4) and link in (0, 1) and snaplen <= 262144, "capture-format")
        while source.tell() < st.st_size:
            raw = source.read(16)
            need(len(raw) == 16 and count < 200000 * cycles, "capture-record-cap")
            sec, frac, length, original = struct.unpack(order + "IIII", raw)
            count += 1
            if count % 4096 == 0:
                need(time.monotonic() < deadline, "closed-metadata-time-bound")
            need(frac < resolution and 14 <= length == original <= snaplen, "capture-truncated")
            start, end = source.tell(), source.tell() + length
            need(end <= st.st_size, "capture-body-truncated")
            prefix = source.read(14 if link == 1 else 4)
            ip_at = source.tell()
            version = source.read(1)[0] >> 4
            source.seek(ip_at)
            if link == 1 and prefix[12:14] not in (b"\x08\x00", b"\x86\xdd"):
                source.seek(end); continue
            if version == 4:
                ip = source.read(20); ihl = (ip[0] & 15) * 4
                if ip[9] != 17 or int.from_bytes(ip[6:8], "big") & 0x3fff:
                    source.seek(end); continue
                need(20 <= ihl <= 60, "ip-header")
                address_a, address_b = str(ipaddress.ip_address(ip[12:16])), str(ipaddress.ip_address(ip[16:20]))
                source.seek(ip_at + ihl)
            elif version == 6:
                ip = source.read(40)
                if ip[6] != 17:
                    source.seek(end); continue
                address_a, address_b = str(ipaddress.ip_address(ip[8:24])), str(ipaddress.ip_address(ip[24:40]))
            else:
                source.seek(end); continue
            udp = source.read(8)
            need(len(udp) == 8, "udp-header")
            port_a, port_b, udp_len, _ = struct.unpack("!HHHH", udp)
            if udp_len < 20:
                source.seek(end); continue
            rtp_at = source.tell(); header = source.read(12)
            need(len(header) == 12, "rtp-header")
            flags, pt_marker, seq, ts, ssrc = struct.unpack("!BBHII", header)
            if flags >> 6 != 2 or 192 <= pt_marker <= 223:
                source.seek(end); continue
            flow = (address_a, port_a, address_b, port_b, ssrc, pt_marker & 127)
            if flow not in flows:
                source.seek(end); continue
            header_len = 12 + (flags & 15) * 4
            source.seek(rtp_at + header_len)
            if flags & 16:
                extension = source.read(4)
                need(len(extension) == 4, "rtp-extension-header")
                header_len += 4 + int.from_bytes(extension[2:4], "big") * 4
            need(not flags & 32 and header_len < udp_len - 8 and rtp_at + udp_len - 8 <= end,
                 "selected-rtp-unsupported-or-truncated")
            kept.add(flow, seq, ts, bool(pt_marker & 128), udp_len - 8 - header_len,
                     sec * 1000000000 + frac * (1000000000 // resolution), count)
            source.seek(end)
    need(count == capture.get("original_packets_captured"), "original-capture-count")
    return kept.build()


# Compact closed-header index. Seven uint64 columns retain no packet payload.
# The conservative live allocation account includes every owned container/key,
# overcounts shared scalar references, and reserves transient epoch work arrays.
class HeaderIndex:
    LIMIT = 512 * 1024**2
    ALLOCATION_SLACK = 64 * 1024**2

    def __init__(self, maximum=600000, deadline=None, auxiliary_bytes=0):
        self.maximum = maximum
        self.deadline = time.monotonic() + 30 if deadline is None else deadline
        self.columns = [array('Q') for _ in range(7)]
        self.flows, self.groups, self.packets, self.sorted = {}, {}, {}, {}
        self.account = (sys.getsizeof(self) + sys.getsizeof(self.__dict__) + sys.getsizeof(self.columns)
                        + sum(sys.getsizeof(v) for v in self.columns)
                        + sum(sys.getsizeof(v) for v in (self.flows, self.groups, self.packets, self.sorted)))
        self.scratch = 0
        self.work_scratch = 0
        self.account += auxiliary_bytes

    def check(self, scratch=0):
        need(time.monotonic() < self.deadline, 'closed-metadata-time-bound')
        need(self.account + self.work_scratch + scratch + self.ALLOCATION_SLACK <= self.LIMIT, 'closed-header-index-memory-bound')
        self.scratch = max(self.scratch, self.work_scratch + scratch)

    def __len__(self):
        return len(self.columns[0])

    def add(self, flow, sequence, timestamp, marker, body_bytes, unix_ns, record):
        need(len(self) < self.maximum, 'witness-global-header-cap')
        fid = self.flows.get(flow)
        if fid is None:
            need(len(self.flows) < 128 * 32 * 2, 'witness-flow-cap')
            before = sys.getsizeof(self.flows)
            fid = len(self.flows)
            self.flows[flow] = fid
            self.account += (sys.getsizeof(self.flows) - before + sys.getsizeof(flow)
                             + sum(sys.getsizeof(v) for v in flow) + sys.getsizeof(fid))
        at = len(self)
        for column, value in zip(self.columns, (fid, sequence, timestamp, int(marker), body_bytes, unix_ns, record)):
            before = sys.getsizeof(column); column.append(value)
            self.account += sys.getsizeof(column) - before
        key = (fid, timestamp)
        group = self.groups.get(key)
        if group is None:
            before = sys.getsizeof(self.groups)
            group = array('I'); self.groups[key] = group
            self.account += (sys.getsizeof(self.groups) - before + sys.getsizeof(key)
                             + sys.getsizeof(fid) + sys.getsizeof(timestamp) + sys.getsizeof(group))
        before = sys.getsizeof(group); group.append(at)
        self.account += sys.getsizeof(group) - before
        key = (fid, sequence, timestamp, int(marker))
        if key in self.packets:
            self.packets[key] = None  # Selected duplicate original packet is never guessed away.
        else:
            before = sys.getsizeof(self.packets); self.packets[key] = at
            self.account += (sys.getsizeof(self.packets) - before + sys.getsizeof(key)
                             + sum(sys.getsizeof(v) for v in key) + sys.getsizeof(at))
        need(self.account + self.ALLOCATION_SLACK <= self.LIMIT, "closed-header-index-memory-bound")
        if at % 4096 == 0:
            self.check()

    def release_work(self):
        self.work_scratch = 0

    def charge(self, amount):
        self.account += amount
        self.check()

    def radix(self, values):
        # Four fixed byte passes, O(4*frames), including reordered timestamps.
        a = array('I', values)
        for shift in (0, 8, 16, 24):
            self.check()
            counts = [0] * 256
            for value in a: counts[(value >> shift) & 255] += 1
            total = 0
            for i, count in enumerate(counts): counts[i], total = total, total + count
            b = array('I', [0]) * len(a)
            for value in a:
                bucket = (value >> shift) & 255
                b[counts[bucket]] = value; counts[bucket] += 1
            a = b
        return a

    def build(self):
        temporary = {fid: array('I') for fid in self.flows.values()}
        # At most one original timestamp per indexed frame, no copied payload.
        self.check(24 * len(self.groups) + 65536)
        for n, (fid, timestamp) in enumerate(self.groups):
            temporary[fid].append(timestamp)
            if n % 4096 == 0: self.check(24 * len(self.groups) + 65536)
        for fid, timestamps in temporary.items():
            ordered = self.radix(timestamps)
            before = sys.getsizeof(self.sorted); self.sorted[fid] = ordered
            self.account += sys.getsizeof(self.sorted) - before + sys.getsizeof(fid) + sys.getsizeof(ordered)
            self.check(24 * len(self.groups) + 65536)
        self.check()
        return self

    def frame(self, source, target, source_ts, target_ts, epoch):
        source_rows, target_rows = self.groups.get((source, source_ts)), self.groups.get((target, target_ts))
        need(source_rows and target_rows and len(source_rows) == len(target_rows), 'original-frame-group-incomplete')
        _, seqs, timestamps, markers, bodies, times, _ = self.columns
        if target_ts == epoch['output_timestamp']:
            need(source_ts == epoch['input_timestamp']
                 and all(delta(seqs[i], epoch['input_sequence'], 16) >= 0 for i in source_rows)
                 and all(delta(seqs[i], epoch['output_sequence'], 16) >= 0 for i in target_rows),
                 'frame-crosses-source-target-epoch-boundary')
        first = seqs[source_rows[0]]
        offsets = set(); last_offset = None; marked = 0
        # Reserve the worst original-fragment set; it is discarded per frame.
        self.check(160 * len(source_rows) + 65536)
        for number, i in enumerate(source_rows):
            if number % 4096 == 0: self.check(160 * len(source_rows) + 65536)
            key = (source, seqs[i], timestamps[i], markers[i])
            need(self.packets.get(key) == i, 'source-duplicate-packet')
            offset = delta(seqs[i], first, 16)
            need(offset not in offsets, 'original-frame-fragment-gap'); offsets.add(offset)
            last_offset = offset if last_offset is None else max(last_offset, offset)
            if markers[i]: marked += 1
            key = (target, (seqs[i] + epoch['sequence_offset']) % 65536, target_ts, markers[i])
            j = self.packets.get(key)
            need(j is not None and bodies[i] == bodies[j], 'header-mapping-or-packetization-differs')
            need(0 < times[j] - times[i] < 200000000, 'primary-transit-unproved')
        need(marked == 1 and len(offsets) == max(offsets) - min(offsets) + 1, 'original-frame-fragment-gap')
        need(any(markers[i] and delta(seqs[i], first, 16) == last_offset for i in source_rows), 'original-frame-fragment-gap')
        for j in target_rows:
            need(self.packets.get((target, seqs[j], timestamps[j], markers[j])) == j, 'target-duplicate-packet')
        return len(source_rows)

    def timestamp_spans(self, flow, start, end=None):
        ordered = self.sorted[flow]
        finish = (start + (1 << 31)) % 2**32 if end is None else end
        need(uint(start, 32) and uint(finish, 32) and finish != start, 'ambiguous-epoch-coordinate-range')
        if end is not None: need(delta(finish, start) > 0, 'backward-epoch-coordinate-range')
        low, high = bisect_left(ordered, start), bisect_left(ordered, finish)
        return ((low, high),) if finish > start else ((low, len(ordered)), (0, high))

    def timestamps(self, flow, start, end=None):
        ordered = self.sorted[flow]
        for a, b in self.timestamp_spans(flow, start, end):
            for i in range(a, b): yield ordered[i]

    def prove_many(self, source_flow, target_flow, epoch, requests):
        source, target = self.flows.get(source_flow), self.flows.get(target_flow)
        need(source is not None and target is not None, 'primary-window-not-bijective')
        anchor, end = epoch['output_timestamp'], epoch.get('_end_output_timestamp')
        ordered = self.sorted[target]
        epoch_frames = sum(b - a for a, b in self.timestamp_spans(target, anchor, end))
        scratch = 64 * epoch_frames + 768 * len(requests) + 65536
        self.check(scratch)
        self.work_scratch = scratch
        # Two bisected slices at most; unrelated epochs of a reused flow
        # are never scanned. EndNone is the exact nonnegative serial half-range.
        ts = array('I', self.timestamps(target, anchor, end))
        distances = array('I', (delta(value, anchor) for value in ts))
        need(ts, 'primary-window-not-bijective')
        # Fixed coordinate arrays and per-request integer pairs; no headers x aliases scan.
        self.check()
        coverage = array('i', [0]) * (len(ts) + 1)
        spans = []
        for n, (input_ts, output_ts) in enumerate(requests):
            need(all(uint(v, 32) for v in input_ts + output_ts), 'original-frame-coordinate-invalid')
            need(delta(input_ts[1], input_ts[0]) > 0, 'repeated-or-backward-original-source-frame')
            first, last = (bisect_left(distances, delta(value, anchor)) for value in output_ts)
            need(0 < first <= last < len(ts) - 1 and ts[first] == output_ts[0] and ts[last] == output_ts[1], 'original-neighbors-missing')
            lo, hi = first - 1, last + 1
            coverage[lo] += 1; coverage[hi + 1] -= 1; spans.append((lo, hi))
            if n % 4096 == 0: self.check()
        # A source-only complete frame inside a covered mapped span is loss,
        # even though target-driven iteration would never visit its timestamp.
        intervals, active, start = [], 0, None
        for i in range(len(ts)):
            active += coverage[i]
            if active and start is None: start = i
            if start is not None and (not active or i == len(ts) - 1):
                finish = i if active else i - 1
                intervals.append((distances[start], distances[finish])); start = None
        visited = 0
        for low, high in intervals:
            input_start = (anchor + low - epoch['timestamp_offset']) % 2**32
            input_end = (anchor + high - epoch['timestamp_offset'] + 1) % 2**32
            for source_ts in self.timestamps(source, input_start, input_end):
                mapped = (source_ts + epoch['timestamp_offset']) % 2**32
                at = bisect_left(distances, delta(mapped, anchor))
                need(at < len(ts) and ts[at] == mapped, 'source-frame-missing-from-target')
                visited += 1
                if visited % 4096 == 0: self.check()
        prefix = array('Q', [0]); active = 0
        for i, target_ts in enumerate(ts):
            active += coverage[i]; count = 0
            if active:
                count = self.frame(source, target, (target_ts - epoch['timestamp_offset']) % 2**32, target_ts, epoch)
            prefix.append(prefix[-1] + count)
            if i % 4096 == 0: self.check()
        self.check()
        result = [{'matched_packets': prefix[hi + 1] - prefix[lo], 'original_timestamps': original,
                 'target_timestamps': target_ts} for (lo, hi), (original, target_ts) in zip(spans, requests)]
        return result


def prove_headers(rows, source_flow, target_flow, epoch, input_ts, output_ts):
    index = rows
    if not isinstance(index, HeaderIndex):
        index = HeaderIndex()
        for row in rows:
            index.add(tuple(row['flow']), row['seq'], row['ts'], row['marker'], row['body_bytes'], row['unix_ns'], row['record'])
        index.build()
    return index.prove_many(source_flow, target_flow, epoch, [(input_ts, output_ts)])[0]


def metadata_size(*roots, deadline):
    # Invoked before the capture index exists. Count all retained request,
    # grouping, context, and result roots once, including scalar objects.
    seen, stack, total, visits = set(), list(roots), 0, 0
    while stack:
        value = stack.pop(); identity = id(value)
        if identity in seen: continue
        seen.add(identity); total += sys.getsizeof(value); visits += 1
        if isinstance(value, dict): stack.extend(value.keys()); stack.extend(value.values())
        elif isinstance(value, (tuple, list, set)): stack.extend(value)
        if visits % 4096 == 0:
            need(time.monotonic() < deadline, 'closed-metadata-time-bound')
            need(total + sys.getsizeof(seen) + len(seen) * sys.getsizeof(identity) + sys.getsizeof(stack) + HeaderIndex.ALLOCATION_SLACK <= HeaderIndex.LIMIT,
                 'closed-proof-metadata-memory-bound')
    return total


def decode_census(route, deadline=None):
    candidate = route['video']['original_source_frame_witness']
    need(candidate.get('profile') == PROFILE and candidate.get('collection_passed') is True, 'incomplete-or-unrequested-source-candidate')
    text = candidate.get('census_json')
    need(isinstance(text, str), 'census-missing')
    size = len(text.encode('utf-8'))
    need(size == candidate.get('encoded_bytes') and size <= WITNESS_CYCLE_BYTES, 'census-byte-bound')
    if deadline is not None: need(time.monotonic() < deadline, 'closed-metadata-time-bound')
    value = json.loads(text)
    if deadline is not None: need(time.monotonic() < deadline, 'closed-metadata-time-bound')
    descriptors, rows, pairs = value.get('descriptors'), value.get('rows'), value.get('pairs')
    need(isinstance(descriptors, list) and 1 <= len(descriptors) <= 10 and isinstance(rows, list)
         and 1 <= len(rows) <= 10000 and isinstance(pairs, list) and len(pairs) < 10000, 'census-shape')
    for d in descriptors:
        need(isinstance(d, list) and len(d) == 4 and all(isinstance(v, str) and 0 < len(v) <= 256 for v in d[:3])
             and uint(d[3], 53) and d[3] > 0, 'census-original-descriptor')
    seen_indices, seen_rtp, footer = set(), set(), {}
    expected = []
    for at, row in enumerate(rows):
        if deadline is not None and at % 4096 == 0: need(time.monotonic() < deadline, 'closed-metadata-time-bound')
        need(isinstance(row, list) and len(row) == 6 and uint(row[0], 16) and row[0] < 10000
             and uint(row[1], 8) and row[1] < len(descriptors) and type(row[2]) in (int, float)
             and math.isfinite(row[2]) and 1 <= row[2] < 61 and uint(row[3], 53) and row[3] > 0
             and uint(row[4], 32) and uint(row[5], 16) and row[5] < 1860, 'census-original-row')
        need(row[0] not in seen_indices, 'census-repeated-row-reference'); seen_indices.add(row[0])
        descriptor = descriptors[row[1]]
        key = (*descriptor[:3], row[4])
        need(key not in seen_rtp, 'repeated-receiver-rtp-frame'); seen_rtp.add(key)
        if row[5] in footer: expected.append([footer[row[5]], at])
        else: footer[row[5]] = at
    need(pairs == expected and len(pairs) == candidate.get('pair_count') == route['video']['duplicate_frames']
         and len(rows) == candidate.get('rows') and candidate.get('legacy_draw_repeat_frames') == len(pairs), 'incomplete-alias-census')
    return value


def census_point(census, reference):
    rows = census['rows']
    need(uint(reference, 16) and reference < len(rows), 'census-index')
    row = rows[reference]; owner, stream, track, element = census['descriptors'][row[1]]
    return dict(owner=owner, row_index=row[0], callback_seconds=row[2],
                presentation=dict(connection_id=owner, stream_id=stream, track_id=track,
                                  element_id=element, presentedFrames=row[3], rtpTimestamp=row[4]))


def public_intervals(cycle, index):
    records = cycle['decoded_media'].get('source_frame_public_owners', [])
    original = [source for source in cycle['source_startup']['sources'] if source.get('index') == index]
    found = [record for record in records if record.get('route') == index]
    need(len(found) == len(original) == 1 and len(original[0]['attachments']) == 1, 'original-public-owner-ledger-missing')
    ledger = found[0]['intervals']
    moves = sorted((move for move in cycle['programme']['moves'] if move['source_index'] == index), key=lambda move: move['ordinal'])
    need(isinstance(ledger, list) and 1 <= len(ledger) <= 3 and len(ledger) == len(moves) + 1, 'public-owner-ledger-shape')
    first = ledger[0]
    need(first.get('owner') == original[0]['attachments'][0]['connection_id'] and first.get('start_ms') == 0
         and first.get('operation_id') is None and first.get('previous') is None
         and first.get('basis') == 'original-initial-public-render', 'original-public-start-unproved')
    route = next((route for route in cycle['decoded_media']['routes'] if route['route'] == index), None)
    cuts = [] if route is None else route['video']['transition_coverage']['original_public_cuts']
    need(not moves or route is not None, 'public-cut-route-missing')
    owners = set()
    for i, interval in enumerate(ledger):
        need(isinstance(interval.get('owner'), str) and interval['owner'] not in owners
             and isinstance(interval.get('receive_stream_id'), str) and isinstance(interval.get('send_stream_id'), str), 'public-owner-ambiguous')
        owners.add(interval['owner'])
        start, end = interval.get('start_ms'), interval.get('end_ms')
        need(type(start) in (int, float) and math.isfinite(start) and 0 <= start < 62000
             and (end is None if i == len(ledger) - 1 else type(end) in (int, float) and math.isfinite(end) and start < end <= 62000), 'public-interval-clock')
        if i:
            move = moves[i - 1]
            matched = [cut for cut in cuts if cut.get('id') == move['target']]
            need(move['source'] == ledger[i - 1]['owner'] and move['target'] == interval['owner']
                 and interval.get('previous') == move['source'] and interval.get('operation_id') == move['operation_id']
                 and interval.get('basis') == 'validated-receipts-local-public-render-cut'
                 and len(matched) == 1 and matched[0].get('basis') == interval['basis']
                 and matched[0].get('atMs') == start == ledger[i - 1]['end_ms'], 'validated-original-public-cut-unproved')
    return ledger


def within_public(ledger, owner, stream, first, last, direction):
    found = [entry for entry in ledger if entry['owner'] == owner and entry[direction + '_stream_id'] == stream
             and entry['start_ms'] / 1000 <= first <= last
             and (entry['end_ms'] is None or last < entry['end_ms'] / 1000)]
    need(len(found) == 1, 'private-or-cross-public-owner-boundary')
    return found[0]


def capture_attachment(cycle,index,owner):
    sources=[source for source in cycle['programme']['sources']['sources'] if source.get('index')==index]
    need(len(sources)==1,'original-public-source-route-ambiguous')
    found=[entry for entry in sources[0]['attachments'] if entry.get('connection_id')==owner]
    need(len(found)==1 and found[0].get('capture_started') is True,'original-public-capture-attachment-unproved')
    context=found[0].get('public_context') or {}
    need(all(isinstance(context.get(key),str) and re.fullmatch(r'[A-Za-z0-9_-]{1,256}',context[key])
             for key in ('conversation_id','session_id')), 'original-public-capture-context-unproved')
    return context


def publication_source(cycle,route,epoch,summaries,receiver_owner,first,last,stats):
    if stats.publications is None:
        records=getattr(summaries,'source_bindings',())
        need(isinstance(records,(list,tuple)) and 0<len(records)<=48,'publication-source-bridge-missing')
        public_ids,runtime_ids=set(),set()
        process={entry.get('process') for entry in summaries}
        need(len(process)==1,'mapping-original-process-ambiguous')
        by_runtime={}
        for record in records:
            publication_record(record,next(iter(process)))
            need(record['public_connection_id'] not in public_ids and record['runtime_connection_id'] not in runtime_ids,
                 'publication-duplicate-or-ambiguous-namespace')
            public_ids.add(record['public_connection_id']);runtime_ids.add(record['runtime_connection_id'])
            by_runtime[record['runtime_connection_id']]=record
        # Original closed collection is indexed once per cycle, not rescanned
        # and serialized per alias. The actual reader never mutates this input.
        stats.publications={'source':summaries,'by_runtime':by_runtime}
    need(stats.publications['source'] is summaries,'publication-original-collection-changed')
    found=[stats.publications['by_runtime'][epoch['source_connection']]] if epoch['source_connection'] in stats.publications['by_runtime'] else []
    need(len(found)==1,'mapping-source-runtime-bridge-unproved')
    record=found[0];owner=record['public_connection_id']
    source_context=capture_attachment(cycle,route['source_route_index'],owner)
    target_context=capture_attachment(cycle,route['route'],receiver_owner)
    need(record['conversation_id']==source_context['conversation_id']==target_context['conversation_id']
         and record['session_id']==source_context['session_id']==target_context['session_id'],
         'publication-foreign-public-session')
    interval=within_public(public_intervals(cycle,route['source_route_index']),owner,epoch['source_logical'],first,last,'send')
    if interval['operation_id'] is None:
        operations=[op for op in cycle.get('admission',{}).get('operations',[]) if op.get('name')=='offer-'+str(route['source_route_index'])]
        need(record['phase']=='initial-ready' and len(operations)==1 and operations[0].get('state')=='completed'
             and record['publication_operation_id']==operations[0].get('operationId'), 'publication-original-initial-operation-unproved')
    else:
        need(record['phase']=='replacement-published' and record['publication_operation_id']==interval['operation_id'],
             'publication-original-replacement-operation-unproved')
    streams=[stream for stream in record['streams'] if stream['logical_id']==epoch['source_logical']]
    need(len(streams)==1,'publication-original-send-stream-unproved')
    stream=streams[0]
    # ReadyMediaBinding directions are Core-facing: Receive is browser -> Core.
    # The independently validated public capture interval remains client SEND.
    need(stream['medium']=='video' and stream['direction'] in ('receive','both') and stream['source_ready'] is True
         and stream['transport_id']==epoch['source_transport']
         and stream['core_stream_generation']==epoch['source_generation'], 'publication-core-source-port-changed')
    # Public connection generation and Core stream generation are deliberately
    # NOT equated. Each is retained under its own explicit semantic field.
    return owner,dict(profile=PUBLICATION_PROFILE,process=record['process'],phase=record['phase'],
        public_connection_id=owner,runtime_connection_id=record['runtime_connection_id'],conversation_id=record['conversation_id'],
        session_id=record['session_id'],participant_id=record['participant_id'],membership_epoch=record['membership_epoch'],
        public_connection_generation=record['public_connection_generation'],offer_record_id=record['offer_record_id'],
        publication_operation_id=record['publication_operation_id'],stream=stream)


def prepare_pair(cycle, route, pair, summaries, stats=None):
    a, b = pair['first'], pair['current']; pa, pb = a['presentation'], b['presentation']
    need(a['owner'] == b['owner'] and pa['element_id'] == pb['element_id']
         and pa['stream_id'] == pb['stream_id'] and pa['track_id'] == pb['track_id']
         and pb['presentedFrames'] > pa['presentedFrames'], 'cross-owner-or-presentation-replay')
    need(all(uint(p['rtpTimestamp'], 32) for p in (pa, pb)), 'invalid-original-rtp-coordinate')
    need(1 <= a['callback_seconds'] < b['callback_seconds'] < 61, 'outside-public-quality-window')
    within_public(public_intervals(cycle, route['route']), a['owner'], pa['stream_id'], a['callback_seconds'], b['callback_seconds'], 'receive')
    stats = stats or StatsIndex(cycle)
    receiver = stats.get(a['owner'], 'inbound-rtp', a['callback_seconds'], b['callback_seconds'], stream=pa['stream_id'], track=pa['track_id'], coordinates=[pa['rtpTimestamp'],pb['rtpTimestamp']])
    timeline, epoch = epoch_for_pair(summaries, receiver, pa['rtpTimestamp'], pb['rtpTimestamp'])
    source_owner,publication=publication_source(cycle,route,epoch,summaries,a['owner'],a['callback_seconds'],b['callback_seconds'],stats)
    source = stats.get(source_owner, 'outbound-rtp', a['callback_seconds'], b['callback_seconds'], stream=epoch['source_logical'], ssrc=epoch['source_ssrc'])
    source_cipher, receiver_cipher = source['path'].get('srtp_cipher'), receiver['path'].get('srtp_cipher')
    if source_cipher is not None and receiver_cipher is not None: need(source_cipher == receiver_cipher, 'different-srtp-wire-profiles')
    output_ts = [pa['rtpTimestamp'], pb['rtpTimestamp']]
    return dict(timeline=timeline, epoch=epoch, source_flow=packet_flow(source, True), target_flow=packet_flow(receiver, False),
                input_ts=[(value - epoch['timestamp_offset']) % 2**32 for value in output_ts], output_ts=output_ts,
                source_publication_binding=publication,
                native_binding_boundaries=[value['native_binding_boundary'] for value in (source,receiver) if 'native_binding_boundary' in value],
                native_retirement_statistics=[value['native_retirement_statistics'] for value in (source,receiver) if 'native_retirement_statistics' in value])


def prepare_association_control(cycle, summaries, censuses=None, stats=None):
    routes = cycle['decoded_media']['routes']; moved = {move['source_index'] for move in cycle['programme']['moves']}
    eligible = [route for route in routes if route['route'] != route['source_route_index'] and route['route'] not in moved and route['source_route_index'] not in moved]
    need(eligible, 'passive-original-unmoved-edge-missing'); selected = min(eligible, key=lambda route: route['route'])
    censuses = censuses or {route['route']: decode_census(route) for route in routes}
    controls = [(route, censuses[route['route']].get('control')) for route in routes if censuses[route['route']].get('control') is not None]
    need(len(controls) == 1 and controls[0][0]['route'] == selected['route'], 'passive-control-missing-or-wrong-route')
    route, control = controls[0]
    need(control.get('receiver_route_index') == route['route'] and control.get('source_route_index') == route['source_route_index']
         and control.get('window_start_seconds') == 2 and control.get('window_end_seconds') == 3
         and isinstance(control.get('rows'), list) and len(control['rows']) == 2, 'passive-control-profile-or-selection')
    census = censuses[route['route']]
    a, b = (census_point(census, reference) for reference in control['rows'])
    need(2 <= a['callback_seconds'] < b['callback_seconds'] <= 3, 'passive-control-window-or-pair')
    return route, prepare_pair(cycle, route, dict(first=a, current=b), summaries, stats)


def finalize(report, browser_folder, server_log, pcap_path):
    """Return a NEW final observation; original raw files and quality rows stay unchanged."""
    deadline = time.monotonic() + 30
    need(report.get('transport') == 'firefox-wss-webrtc' and report.get('stage') == 'quality', 'firefox-quality-profile-only')
    need(report.get('browser_exit_code') == 0 and report.get('browser_owner_joined') is True, 'original-browser-collection-not-joined0')
    raw = report['browser_observation']
    need(raw.get('source_revision') == report.get('source_revision') and raw.get('transport') == report.get('transport'), 'original-browser-source-or-transport-differs')
    need(raw.get('source_frame_witness_profile') == PROFILE and raw.get('collection_passed') is True, 'not-a-prospective-complete-collection')
    archives = raw['resource_cycles']; need(1 <= len(archives) <= 3, 'original-cycle-count')
    servers = report.get('servers', [])
    need(servers and all(server.get('original_joined') is True and server.get('original_exit_code') == 0 and server.get('original_process_group_joined') is True for server in servers), 'original-server-joins-unproved')
    actual = [server for server in servers if server['name'] == 'materialized-server']; need(len(actual) == 1, 'original-server-owner-ambiguous')
    summaries, log_sha = mapping_summaries(server_log, actual[0]['original_pid'], deadline)
    cycles, contexts, context_ids, requests, windows = [], [], {}, [], []
    cycle_bytes = 0
    statistics = []
    census_bytes = {}
    boundary_used = {}
    retirement_statistics_used = {}
    publication_used = {}
    def request(prepared, cycle_index, route_index, reference, control=False):
        publication=prepared['source_publication_binding']
        publication_used[(cycle_index,publication['runtime_connection_id'],publication['stream']['transport_id'])]=dict(cycle=cycle_index,**publication)
        for binding in prepared.get('native_binding_boundaries', []):
            key=(cycle_index,binding['profile'],binding['peer_index'],binding['direction'])
            boundary_used[key]=dict(cycle=cycle_index,**binding)
        for value in prepared.get('native_retirement_statistics',[]):
            retirement_statistics_used[(cycle_index,value['peer_index'],value['direction'])]=dict(cycle=cycle_index,**value)
        key = (prepared['timeline'], prepared['epoch']['ordinal'], prepared['source_flow'], prepared['target_flow'])
        context = context_ids.get(key)
        if context is None:
            context = len(contexts); context_ids[key] = context; contexts.append(prepared)
            windows.extend(((prepared['source_flow'], 0, 0), (prepared['target_flow'], 0, 0)))
        requests.append((context, tuple(prepared['input_ts']), tuple(prepared['output_ts']), cycle_index, route_index, reference, control))
        if len(requests) % 4096 == 0: need(time.monotonic() < deadline, 'closed-metadata-time-bound')
    for archive in archives:
        path = Path(browser_folder) / 'cycles' / ('cycle-' + str(archive['index'])) / 'cycle-observation.json'
        cycle_bytes += path.stat().st_size; need(cycle_bytes <= CYCLES_JSON_BYTES, 'original-cycle-aggregate-byte-bound')
        cycle, digest = read_json(path, CYCLE_JSON_BYTES)
        need(digest == archive['observation_sha256'] and cycle.get('collection_passed') is True, 'original-cycle-hash-or-collection')
        need(cycle.get('cycle', {}).get('workload_id') == archive['workload_id'], 'original-cycle-owner')
        final_cycle = copy.deepcopy(cycle); censuses = {}; witness_bytes = 0
        stats = StatsIndex(cycle, deadline); statistics.append(stats)
        for route in cycle['decoded_media']['routes']:
            census = decode_census(route, deadline); censuses[route['route']] = census
            witness_bytes += route['video']['original_source_frame_witness']['encoded_bytes']
            need(census['public_intervals'] == public_intervals(cycle, route['route']), 'census-public-ledger-differs')
            for number, row in enumerate(census['rows']):
                descriptor = census['descriptors'][row[1]]
                within_public(census['public_intervals'], descriptor[0], descriptor[1], row[2], row[2], 'receive')
                if number % 4096 == 0: need(time.monotonic() < deadline, 'closed-metadata-time-bound')

            for reference, (first, current) in enumerate(census['pairs']):
                need(len(requests) < MAX_PAIRS + 3, 'global-source-pair-cap')
                pair = dict(first=census_point(census, first), current=census_point(census, current))
                request(prepare_pair(cycle, route, pair, summaries, stats), archive['index'], route['route'], reference)
        need(witness_bytes <= WITNESS_CYCLE_BYTES, 'cycle-census-byte-bound')
        census_bytes[archive['index']] = witness_bytes
        control_route, control = prepare_association_control(cycle, summaries, censuses, stats)
        request(control, archive['index'], control_route['route'], -1, True)
        cycles.append(final_cycle)
    grouped = [[] for _ in contexts]
    for number, value in enumerate(requests): grouped[value[0]].append(number)
    auxiliary = metadata_size(requests, grouped, contexts, context_ids, windows, [value.cache for value in statistics],
                              [value.requested for value in statistics], [value.completed for value in statistics], [value.receiver_owners for value in statistics], [value.roles for value in statistics], [value.boundaries for value in statistics], boundary_used, retirement_statistics_used, summaries.source_bindings, [value.publications for value in statistics], publication_used, deadline=deadline)
    headers = capture_headers(pcap_path, report['packet_capture'], windows, cycles=len(cycles), deadline=deadline, auxiliary_bytes=auxiliary)
    compact, control_facts = [], []
    for context_id, numbers in enumerate(grouped):
        context = contexts[context_id]
        proofs = headers.prove_many(context['source_flow'], context['target_flow'], context['epoch'], [(list(requests[n][1]), list(requests[n][2])) for n in numbers])
        for number, proof in zip(numbers, proofs):
            _, _, _, cycle_index, route_index, reference, control = requests[number]
            if control:
                control_facts.append(dict(profile='connect/rtc-passive-source-association/1', cycle=cycle_index, route=route_index,
                    timeline=context['timeline'], epoch=context['epoch']['ordinal'], **proof))
            else:
                row = [cycle_index, route_index, reference, context_id, proof['matched_packets']]
                before = sys.getsizeof(compact); compact.append(row)
                headers.charge(sys.getsizeof(compact) - before + sys.getsizeof(row) + sum(sys.getsizeof(v) for v in row))
        headers.charge(4096 * sum(requests[n][-1] for n in numbers))
        del proofs
        headers.release_work()
    headers.check(128 * len(compact) + 8192 * len(contexts) + 65536)
    index_accounted, index_scratch = headers.account, headers.scratch
    # No original header/index owners survive into final quality graph copies.
    del headers
    proof_json = json.dumps(compact, separators=(',', ':'))
    need(time.monotonic() < deadline, 'closed-metadata-time-bound')
    # Proof rows reference immutable original census and epoch descriptors; no repeated long IDs.
    need(len(proof_json.encode()) <= WITNESS_CYCLE_BYTES * len(cycles), 'final-proof-byte-bound')
    for cycle_index, size in census_bytes.items():
        selected = [row for row in compact if row[0] == cycle_index]
        used = {row[3] for row in selected}
        for control in control_facts:
            if control['cycle'] == cycle_index:
                used.update(i for i, context in enumerate(contexts) if context['timeline'] == control['timeline'] and context['epoch']['ordinal'] == control['epoch'])
        encoded = json.dumps(dict(proofs=selected, epochs=[dict(timeline=contexts[i]['timeline'], epoch=contexts[i]['epoch']) for i in used],
            controls=[control for control in control_facts if control['cycle'] == cycle_index],
            native_binding_boundaries=[value for value in boundary_used.values() if value['cycle']==cycle_index],
            native_retirement_statistics=[value for value in retirement_statistics_used.values() if value['cycle']==cycle_index],
            source_publication_bindings=[value for value in publication_used.values() if value['cycle']==cycle_index]), separators=(',', ':')).encode()
        need(len(used) <= 10 * 32 and size + len(encoded) <= WITNESS_CYCLE_BYTES, 'final-cycle-witness-byte-bound')
        need(time.monotonic() < deadline, 'closed-metadata-time-bound')

    for cycle in cycles:
        for route in cycle['decoded_media']['routes']:
            video = route['video']; video['raw_draw_duplicate_frames'] = video['duplicate_frames']
            video['actual_source_duplicate_frames'] = 0; video['source_frame_profile'] = PROFILE; video['duplicate_frames'] = 0
            video['passed'] = video['original_source_frame_witness']['non_duplicate_passed'] is True
            route['passed'] = not route['invalid'] and route['audio']['passed'] and video['passed'] and route['sync']['status'] == 'passed'
        cycle['decoded_media']['passed'] = all(route['passed'] for route in cycle['decoded_media']['routes'])
        cycle['programme']['decoded_media'] = copy.deepcopy(cycle['decoded_media'])
        cycle['raw_quality_passed'] = cycle['passed']; cycle['passed'] = cycle['collection_passed'] and cycle['decoded_media']['passed']
        if cycle.get('failure') == dict(stage='decoded-media-quality', name='ProfileBudgetsFailed'): cycle['raw_failure'] = cycle.pop('failure')
        need(len(json.dumps(cycle, separators=(',', ':')).encode()) <= CYCLE_JSON_BYTES, 'final-cycle-byte-bound')
        need(time.monotonic() < deadline, 'closed-metadata-time-bound')
    final = copy.deepcopy(raw); first = next((cycle for cycle in cycles if not cycle['passed']), cycles[0]); final.update(first)
    final['passed'] = all(cycle['passed'] for cycle in cycles)
    final['raw_quality_pending_original_source_witness'] = final.get('quality_pending_original_source_witness', False); final['quality_pending_original_source_witness'] = False
    final['cycles'] = [{**summary, 'raw_passed': summary['passed'], 'passed': cycle['passed']} for summary, cycle in zip(raw['cycles'], cycles)]
    for summary in final['cycles']:
        if summary.get('failure') == dict(stage='decoded-media-quality', name='ProfileBudgetsFailed'): summary['raw_failure'] = summary.pop('failure')
    if final['passed']: final.pop('first_failed_cycle', None)
    final['original_source_frame_adjudication'] = dict(profile=PROFILE, complete=True, passed=final['passed'],
        raw_browser_sha256=report['browser_observation_sha256'], pcap_sha256=report['packet_capture']['pcap_sha256'], server_log_sha256=log_sha,
        source_revision=report['source_revision'], original_server_pid=actual[0]['original_pid'], association_controls=control_facts,
        proof_json=proof_json, proof_rows=len(compact), proof_layout=['cycle','route','original_pair_reference','epoch_descriptor','matched_packets'],
        epochs=[dict(timeline=value['timeline'], epoch=value['epoch']) for value in contexts],
        source_publication_bindings_used=list(publication_used.values()),
        native_binding_boundaries_used=list(boundary_used.values()),
        native_retirement_statistics_used=list(retirement_statistics_used.values()),
        header_index_accounted_bytes=index_accounted, header_index_peak_scratch_bytes=index_scratch,
        scope='Prospective exact public-owner/source-origin predicate only; all original quality rows and historical results unchanged')
    need(len(json.dumps(final, separators=(',', ':')).encode()) <= CYCLES_JSON_BYTES, 'final-private-browser-byte-bound')
    need(time.monotonic() < deadline, 'closed-metadata-time-bound')
    return final, cycles



def compact_proof_bound(value, report):
    try:
        text = value.get('proof_json')
        if not isinstance(text, str) or len(text.encode()) > WITNESS_CYCLE_BYTES * 3: return False
        rows = json.loads(text)
        if not isinstance(rows, list) or len(rows) != value.get('proof_rows') or len(rows) > MAX_PAIRS: return False
        observation = report.get('browser_observation', {})
        archives = observation.get('resource_cycles', [])
        cycles = report.get('cycle_observations') if len(archives) > 1 else [observation]
        if not isinstance(cycles, list) or len(cycles) != len(archives): return False
        expected = {}
        for archive, cycle in zip(archives, cycles):
            for route in cycle.get('decoded_media', {}).get('routes', []):
                candidate = route['video']['original_source_frame_witness']
                count = candidate.get('pair_count')
                if candidate.get('profile') != PROFILE or not uint(count, 16) or count >= 10000: return False
                expected[(archive['index'], route['route'])] = count
        descriptors = value.get('epochs', [])
        if not isinstance(descriptors, list) or len(descriptors) > 128 * 32: return False
        seen = set(); counts = {}
        for row in rows:
            if not isinstance(row, list) or len(row) != 5 or not all(uint(v, 32) for v in row): return False
            cycle, route, reference, epoch, matched = row
            if (cycle, route) not in expected or reference >= expected[(cycle, route)] or epoch >= len(descriptors) or matched == 0: return False
            key = (cycle, route, reference)
            if key in seen: return False
            seen.add(key); counts[(cycle, route)] = counts.get((cycle, route), 0) + 1
        return all(counts.get(key, 0) == count for key, count in expected.items())
    except (KeyError, TypeError, ValueError, OverflowError):
        return False


def report_bound(report):
    """JSON-only grader fence; never treats pending collection as media pass."""
    observation = report.get('browser_observation', {})
    def marked(value):
        if not isinstance(value,dict):return False
        if any(key in value for key in ('source_frame_witness_profile','original_source_frame_adjudication')):return True
        decoded=value.get('decoded_media',{})
        return any(any(key in row.get('video',{}) for key in
            ('source_frame_profile','original_source_frame_witness','actual_source_duplicate_frames','raw_draw_duplicate_frames'))
            for row in decoded.get('routes',[]))
    markers=('source_frame_adjudication' in report or 'raw_browser_observation_sha256' in report
        or marked(observation) or any(marked(c) for c in report.get('cycle_observations',[])))
    enabled=report.get('diagnostic_options', {}).get('rtc_source_frame_witness')
    if enabled is not True:
        return not markers and enabled in (None,False)
    if not markers:return False
    value = report.get('source_frame_adjudication', {})
    return (report.get('transport') == 'firefox-wss-webrtc' and report.get('stage') == 'quality'
            and report.get('browser_exit_code') == 0 and report.get('browser_owner_joined') is True
            and all(report.get(k) is True for k in ('source_inputs_unchanged','sdk_dist_inputs_unchanged','executable_unchanged'))
            and value.get('profile') == PROFILE and value.get('complete') is True and value.get('passed') is True
            and value.get('source_revision') == report.get('source_revision')
            and value.get('pcap_sha256') == report.get('packet_capture', {}).get('pcap_sha256')
            and compact_proof_bound(value, report)
            and uint(value.get('proof_rows'), 32) and value['proof_rows'] <= MAX_PAIRS
            and isinstance(value.get('association_controls'), list)
            and 1 <= len(value['association_controls']) == len(observation.get('resource_cycles', [])) <= 3
            and all(control.get('profile') == 'connect/rtc-passive-source-association/1'
                and type(control.get('matched_packets')) is int and 0 < control['matched_packets'] <= MAX_HEADERS
                and uint(control.get('route'), 16)
                and isinstance(control.get('original_timestamps'), list) and len(control['original_timestamps']) == 2
                and all(uint(timestamp, 32) for timestamp in control['original_timestamps'])
                and control['original_timestamps'][0] != control['original_timestamps'][1]
                for control in value['association_controls'])
            and len({control.get('cycle') for control in value['association_controls']}) == len(value['association_controls'])
            and all(control.get('cycle') == archive.get('index') for control, archive in
                zip(value['association_controls'], observation['resource_cycles']))
            and value.get('raw_browser_sha256') == report.get('raw_browser_observation_sha256')
            and isinstance(value.get('raw_browser_sha256'), str) and len(value['raw_browser_sha256']) == 64
            and all(c in '0123456789abcdef' for c in value['raw_browser_sha256'])
            and observation.get('source_frame_witness_profile') == PROFILE
            and observation.get('original_source_frame_adjudication') == value
            and observation.get('passed') is True)
