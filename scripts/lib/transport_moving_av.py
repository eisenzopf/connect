"""Decode every physical codec history before measuring six logical receivers.

Public membership is the original native writer sequence cut on the unchanged
programme clock. Decoder state, compressed bytes and presentation timestamps
never cross physical receiver, stream or SSRC boundaries.
"""
import array
import base64
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import struct

from .transport_av import (FREQUENCIES, checksum, command, marker_edges, marker_profile,
    ogg_page, ordinal_sync, rtp_timing, source_pixels)
from .transport_movement_profile import (MOVE_START_OFFSETS_MS, MOVES_PER_SESSION,
    TOTAL_MOVES, MOVEMENT_PROFILE, MOVEMENT_PROFILE_SHA256)

PEERS = [3, 0, 1, 2, 5, 4]


def original(path, maximum, expected=None):
    path = Path(path)
    if path.resolve() != path or '.local' not in path.parts:
        raise ValueError('moving capture must be an original private path')
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), 'rb') as stream:
        before = os.fstat(stream.fileno())
        if before.st_mode & 0o077 or not 0 < before.st_size <= maximum:
            raise ValueError('moving capture private file bound')
        raw = stream.read(maximum + 1)
        after = os.fstat(stream.fileno())
        if any(getattr(before, key) != getattr(after, key) for key in ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')):
            raise ValueError('moving capture changed during observation')
    if len(raw) != before.st_size or expected is not None and hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('moving capture original digest')
    return json.loads(raw)


def rows(value, medium, allowed_streams, default_stream=None):
    if not isinstance(value, list) or len(value) > (3200 if medium == 'audio' else 2000):
        raise ValueError('moving codec row bound')
    histories = {}
    clocks = {}
    video_boundary = None
    video_group = 0
    last_decoder_history = 0
    previous = -1
    for row in value:
        at, timestamp, ssrc = row['elapsed_ms'], row['timestamp'], row['ssrc']
        stream = row.get('stream_id', default_stream)
        if (type(at) not in (float, int) or not math.isfinite(at) or not previous <= at < 62200
                or type(timestamp) is not int or not 0 <= timestamp < 2 ** 32
                or type(ssrc) is not int or not 0 <= ssrc < 2 ** 32 or stream not in allowed_streams):
            raise ValueError('moving original codec identity or arrival order')
        previous = at
        payload = base64.b64decode(row['payload'], validate=True)
        if not 0 < len(payload) <= (1168 if medium == 'audio' else 256 * 1024):
            raise ValueError('moving original codec payload bound')
        clock_key = (stream, ssrc)
        if clock_key in clocks:
            distance = (timestamp - clocks[clock_key]) % 2 ** 32
            if not 0 < distance < 2 ** 31:
                raise ValueError('moving codec history repeated or regressed original RTP clock')
        clocks[clock_key] = timestamp
        if medium == 'video' and type(row['key']) is not bool:
            raise ValueError('moving original VP8 keyframe flag')
        if medium == 'video':
            decoder_history = row.get('decoder_history')
            if decoder_history is not None:
                if type(decoder_history) is not int or decoder_history < 1 or decoder_history < last_decoder_history:
                    raise ValueError('moving video decoder history regressed')
                last_decoder_history = decoder_history
            # Decode each contiguous physical history separately. Returning to
            # the same stream/SSRC cannot recover a previous decoder's state.
            epoch = row.get('video_epoch')
            if epoch is not None:
                if (not isinstance(epoch, dict) or set(epoch) != {'epoch','incarnation','revision','ssrc','first_sequence','first_timestamp'}
                        or epoch['ssrc'] != ssrc or type(epoch['revision']) is not int or epoch['revision'] <= 0):
                    raise ValueError('moving exact video epoch identity')
                epoch = tuple(epoch[key] for key in ('epoch','incarnation','revision','ssrc','first_sequence','first_timestamp'))
            boundary = (stream, ssrc, decoder_history, epoch, row.get('connection_generation'), row.get('stream_generation'))
            if boundary != video_boundary:
                video_group += 1
                video_boundary = boundary
            key = (stream, ssrc, video_group)
        else:
            key = (stream, ssrc, 0)
        history = histories.setdefault(key, [])
        history.append(row)
    if len(histories) > 32:
        raise ValueError('moving physical codec history bound')
    return [(stream, ssrc, history) for (stream, ssrc, _), history in histories.items()]


def native_cut_membership(capture, row):
    boundary=capture['native_activation']
    if (boundary.get('profile')!='connect/native-public-sequence-cut/1' or
            boundary.get('audience',{}).get('scope')!='public'):
        raise ValueError('native capture has no public writer boundary')
    cuts=boundary['audience']['streams']
    selected=[cut for cut in cuts if cut['stream_id']==row['stream_id']]
    if len(selected)!=1: raise ValueError('native capture cut stream identity')
    cut=selected[0]; seqs=row.get('native_datagram_sequences')
    if (cut.get('sequence_space')!='native-outer-u32' or cut.get('submission_observed') is not False or
            type(cut.get('cursor_ordinal')) is not int or not 0<=cut['cursor_ordinal']<2**31 or
            cut.get('public_sequence_floor')!=cut['cursor_ordinal'] or
            cut.get('stream_generation')!=row.get('stream_generation') or
            cut.get('physical_incarnation')!=row.get('native_physical_incarnation') or
            not isinstance(seqs,list) or not 1<=len(seqs)<=64 or len(set(seqs))!=len(seqs) or
            any(type(v) is not int or not 0<=v<2**31 for v in seqs)):
        raise ValueError('native capture original cursor provenance')
    membership=[v>=cut['public_sequence_floor'] for v in seqs]
    if any(v!=membership[0] for v in membership):raise ValueError('original decoded frame straddles native activation')
    return membership[0]


def original_audio_quantum(row, reports, origin_us):
    """Identity only, never a replacement output timestamp or repaired PCM.

    The original SR maps this exact physical SSRC's RTP samples to the frozen
    source programme. Integer arithmetic preserves NTP precision. The rounded
    20ms source quantum is usable only within the original 62-second source.
    """
    if type(origin_us) is not int or origin_us<=0 or type(row.get('timestamp')) is not int or not 0<=row['timestamp']<2**32:raise ValueError('original source programme identity')
    anchors=[v for v in reports if v.get('ssrc')==row['ssrc']]
    if not anchors:return None
    origin=(origin_us*(1<<32))//1000000+(2208988800<<32)
    identities=set()
    for sr in anchors:
        raw=sr['ntp']
        if not isinstance(raw,str) or not raw.isascii() or not raw.isdecimal() or str(int(raw))!=raw:raise ValueError('original audio SR identity')
        ntp=int(raw);rtp=sr['rtp']
        if not 0<=ntp<2**64 or type(rtp) is not int or not 0<=rtp<2**32:raise ValueError('original audio SR identity')
        delta=(row['timestamp']-rtp+2**31)%2**32-2**31
        if delta==-2**31:raise ValueError('ambiguous original audio RTP distance')
        numerator=(ntp-origin)*48000+delta*(1<<32);denominator=960*(1<<32)
        quantum=(numerator+denominator//2)//denominator
        if 0<=quantum<3100:identities.add(quantum)
    if len(identities)>1:raise ValueError('original source audio quantum has conflicting SR lineage')
    return next(iter(identities),None)

def timeline(directory, metadata):
    bundle = original(directory / 'movement-segments.json', 96 * 1024 * 1024, metadata['sha256'])
    if (bundle['format'] != 'connect/physical-movement-segments/1'
            or bundle['original_move_records_bound'] != TOTAL_MOVES or bundle['reliable_data_exchanges_bound'] != TOTAL_MOVES
            or bundle.get('movement_profile') != MOVEMENT_PROFILE or bundle.get('movement_profile_sha256') != MOVEMENT_PROFILE_SHA256
            or bundle.get('scheduled_start_offsets_ms') != list(MOVE_START_OFFSETS_MS)
            or len(bundle['segments']) != TOTAL_MOVES or len(bundle['original_connection_ids']) != 6
            or len(set(bundle['original_connection_ids'])) != 6 or len(bundle['original_stream_ids']) != 6
            or len(metadata['original_capture_sha256']) != 6):
        raise ValueError('moving complete original custody bundle')
    origin = bundle['program_start_unix_us']
    if type(origin) is not int or origin <= 0:
        raise ValueError('moving original programme origin')
    chains = {}
    candidates = set(bundle['original_connection_ids'])
    for receiver in (0, 4):
        chain = sorted((segment for segment in bundle['segments'] if segment['source_index'] == receiver), key=lambda s:s['ordinal'])
        if [s['ordinal'] for s in chain] != list(range(MOVES_PER_SESSION)):
            raise ValueError('moving original switch-and-return chain')
        cuts = []
        for segment in chain:
            capture = segment['capture']
            cut = capture['root_completed_observed_programme_us'] / 1000
            scheduled = MOVE_START_OFFSETS_MS[segment['ordinal']]
            actual = segment.get('actual_start_ms')
            if (capture['format'] != 'connect/physical-av-segment/3' or capture['program_start_unix_us'] != origin
                    or segment.get('scheduled_start_ms') != scheduled
                    or type(actual) not in (int, float) or not math.isfinite(actual) or not scheduled <= actual <= cut < 62000
                    or cuts and cut <= cuts[-1]
                    or segment['connection_id'] in candidates or segment['reliable_data']['original_data_messages_bound'] != 2):
                raise ValueError('moving original ownership boundary or Data custody')
            activation=capture.get('native_activation',{})
            previous=bundle['original_connection_ids'][receiver] if not cuts else chain[len(cuts)-1]['connection_id']
            if (activation.get('profile')!='connect/native-public-sequence-cut/1' or activation.get('operation_id')!=segment['operation_id']
                    or activation.get('source_connection_id')!=previous or activation.get('target_connection_id')!=segment['connection_id']
                    or activation.get('audience',{}).get('scope')!='public'):
                raise ValueError('moving original public activation lineage')
            candidates.add(segment['connection_id']); cuts.append(cut)
        chains[receiver] = (chain, cuts)
    result = []
    for receiver, peer in enumerate(PEERS):
        captured = original(directory / f'received-{receiver}.json', 32 * 1024 * 1024,
            metadata['original_capture_sha256'][receiver])
        streams = bundle['original_stream_ids']
        if (captured['format'] != 'connect/native-av-received/1' or captured['index'] != receiver
                or captured['peer_index'] != peer or captured['connection_id'] != bundle['original_connection_ids'][receiver]
                or captured['session_index'] != int(receiver >= 4) or captured['generation'] <= 0
                or captured['original_audio_stream_id'] != streams[receiver]['audio']
                or captured['initial_video_stream_id'] != streams[peer]['video'] or captured['unix_start_ms'] * 1000 != origin):
            raise ValueError('moving original receiver binding')
        allowed_video = {streams[peer]['video']}
        if peer in chains:
            allowed_video.update(segment['published_stream_ids']['video'] for segment in chains[peer][0])
        end = 62000
        physical = [(f'original-{receiver}', 0, end, captured,
            rows(captured['audio'], 'audio', {streams[receiver]['audio']}, streams[receiver]['audio']),
            rows(captured['video'], 'video', allowed_video))]
        if receiver in chains:
            chain, cuts = chains[receiver]
            for ordinal, segment in enumerate(chain):
                capture = segment['capture']; audio = []; video = []
                context = capture.get('decoder_context')
                if (not isinstance(context, dict) or context.get('profile') != 'connect/native-retained-private-context/1'
                        or context.get('connection_id') != segment['connection_id']
                        or type(context.get('target_generation')) is not int or context['target_generation'] < 2
                        or context.get('source_generation') != context['target_generation']-1
                        or context.get('audio_stream_id') != segment['published_stream_ids']['audio']
                        or context.get('video_stream_id') != streams[peer]['video']
                        or type(context.get('video_stream_generation')) is not int or context['video_stream_generation'] <= 0
                        or type(context.get('decoder_started_programme_us')) is not int
                        or not 0 <= context['decoder_started_programme_us'] <= cuts[ordinal]*1000):
                    raise ValueError('moving original private decoder context binding')
                for medium, output in [('audio', audio), ('video', video)]:
                    for history in capture[medium]:
                        group = history['rows']
                        normalized = rows(group, medium,
                            {segment['published_stream_ids']['audio']} if medium == 'audio' else {streams[peer]['video']})
                        for row in group:
                            private = not native_cut_membership(capture,row)
                            if (type(row.get('decoder_context_only')) is not bool or row['decoder_context_only'] != private
                                    or row.get('connection_generation') != context['target_generation']
                                    or row['elapsed_ms']*1000 < context['decoder_started_programme_us']):
                                raise ValueError('moving private context cannot claim public packet membership')
                            if medium == 'video' and row.get('stream_generation') != context['video_stream_generation']:
                                raise ValueError('moving private video generation changed')
                            if private and medium == 'audio' and row['ssrc'] != context.get('audio_ssrc'):
                                raise ValueError('moving private audio source context changed')
                            if private and medium == 'video':
                                decoded = row.get('original_decoder_completed_programme_us')
                                encoded = base64.b64decode(row['payload'],validate=True)
                                if (row.get('video_epoch') != context.get('video_epoch') or not isinstance(context.get('video_epoch'), dict)
                                        or row['ssrc'] != context['video_epoch'].get('ssrc')
                                        or type(row.get('original_decoder_ordinal')) is not int or not 0 <= row['original_decoder_ordinal'] < 1860
                                        or row.get('original_decoder_encoded_sha256') != hashlib.sha256(encoded).hexdigest()
                                        or type(decoded) is not int or not row['elapsed_ms']*1000 <= decoded < row['elapsed_ms']*1000+200000):
                                    raise ValueError('moving private video context lacks its exact actual decoder output')
                        output.extend(normalized)
                physical.append((f'physical-{receiver}-{ordinal}', 0, 62000,
                    capture, audio, video))
        histories = []; clocks = {'audio':0,'video':0}
        excluded = {'audio':0, 'video':0}
        for capture_id, start, end, captured, audio, video in physical:
            for medium in clocks:
                clocks[medium] += sum(start <= row['elapsed_ms'] < end for row in captured.get('clock_reports',{}).get(medium,[]))
            for medium, groups in [('audio',audio), ('video',video)]:
                for stream, ssrc, group in groups:
                    # Prior rows in this same codec history may initialize its
                    # decoder. Every counted row belongs to exactly one owner.
                    bounded = [dict(row) for row in group if row['elapsed_ms'] < end]
                    if medium=='audio':
                        for row in bounded:
                            row['original_programme_audio_quantum']=original_audio_quantum(row,captured.get('clock_reports',{}).get('audio',[]),origin)
                    selected = [i for i,row in enumerate(bounded) if not row.get('decoder_context_only', False) and max(2000,start) <= row['elapsed_ms'] < min(62000,end)]
                    excluded[medium] += len(group) - len(selected)
                    if bounded and selected:
                        histories.append(dict(capture_id=capture_id, medium=medium, stream_id=stream, ssrc=ssrc,
                            owner_start_ms=start, owner_end_ms=end, rows=bounded, selected_indices=selected))
        result.append(dict(index=receiver, peer_index=peer, histories=histories, excluded_rows=excluded,received_clocks=clocks))
    return bundle, result


def audio_decode(directory, name, history, expected_hz):
    path = directory / (name + '.ogg'); group = history['rows']
    with path.open('xb') as stream:
        stream.write(ogg_page(struct.pack('<8sBBHIhB',b'OpusHead',1,1,0,48000,0,0),0,0,2))
        stream.write(ogg_page(b'OpusTags'+struct.pack('<II',0,0),1,0))
        for index,row in enumerate(group):
            stream.write(ogg_page(base64.b64decode(row['payload'],validate=True),index+2,(index+1)*960,4 if index == len(group)-1 else 0))
    pcm = directory / (name+'.f32')
    command(['ffmpeg','-nostdin','-hide_banner','-loglevel','warning','-i',str(path),'-ac','1','-ar','48000','-f','f32le',str(pcm)],directory/(name+'.log'))
    values = array.array('f'); values.frombytes(pcm.read_bytes())
    if len(values) != len(group)*960 or any(not math.isfinite(v) for v in values):
        raise ValueError('moving independent Opus original packet correspondence')
    selected = history['selected_indices']; blocks=[]; measured=[]; bad=0
    for index in selected:
        samples=values[index*960:(index+1)*960]
        measured.append(dict(row=group[index],level=math.sqrt(sum(v*v for v in samples)/960)))
    # The same identity thresholds cover every100ms block plus each actual
    # short boundary remainder; no boundary PCM is silently dropped or filled.
    for offset in range(0,len(selected),5):
        block=array.array('f')
        for index in selected[offset:offset+5]: block.extend(values[index*960:(index+1)*960])
        block=block[::8]; energy=sum(v*v for v in block)/len(block); powers=[]
        for hz in FREQUENCIES:
            coefficient=2*math.cos(2*math.pi*hz/6000);a=b=0
            for sample in block: value=sample+coefficient*a-b;b,a=a,value
            powers.append((a*a+b*b-coefficient*a*b)/len(block)**2)
        desired=FREQUENCIES.index(expected_hz)
        bad+=int(energy<=1e-6 or powers[desired]<=8*max(1e-12,max(p for i,p in enumerate(powers) if i!=desired)))
        blocks.append(len(block)*8)
    return dict(measurements=measured,bad_identity_blocks=bad,identity_blocks=len(blocks),
        measured_samples=len(selected)*960,pcm_sha256=checksum(pcm),ogg_sha256=checksum(path))


def video_decode(directory,name,history,peer):
    group=history['rows'];first=next((i for i,row in enumerate(group) if row['key']),None)
    if first is None: return dict(measurements=[],undecodable_frames=len(history['selected_indices']),decoder_started=False)
    ivf=directory/(name+'.ivf');origin=group[first]['timestamp']
    with ivf.open('xb') as stream:
        stream.write(struct.pack('<4sHH4sHHIIII',b'DKIF',0,32,b'VP80',640,360,90000,1,0,0))
        for row in group[first:]:
            body=base64.b64decode(row['payload'],validate=True)
            stream.write(struct.pack('<IQ',len(body),(row['timestamp']-origin)%2**32)+body)
    hashes=directory/(name+'.framemd5');pixels=directory/(name+'.rgb');pulses=directory/(name+'.gray')
    command(['ffmpeg','-nostdin','-hide_banner','-loglevel','error','-copyts','-i',str(ivf),
        '-filter_complex','[0:v]split=3[h][c][p];[c]crop=16:16:8:8,scale=1:1[m];[p]crop=16:16:104:8,scale=1:1[q]',
        '-map','[h]','-an','-enc_time_base','1:90000','-fps_mode','passthrough','-f','framemd5',str(hashes),
        '-map','[m]','-pix_fmt','rgb24','-fps_mode','passthrough','-f','rawvideo',str(pixels),
        '-map','[q]','-pix_fmt','gray','-fps_mode','passthrough','-f','rawvideo',str(pulses)],directory/(name+'.log'))
    lines=hashes.read_text().splitlines()
    if '#dimensions 0: 640x360' not in lines: raise ValueError('moving decoded VP8 dimensions')
    decoded=[line for line in lines if line and not line.startswith('#')];rgb=pixels.read_bytes();gray=pulses.read_bytes()
    if len(decoded)!=len(group)-first or len(rgb)!=len(decoded)*3 or len(gray)!=len(decoded):
        raise ValueError('moving independent VP8 original frame correspondence')
    selected=set(history['selected_indices']);measured=[]
    for at,line in enumerate(decoded):
        row=group[first+at]
        if int(line.split(',')[2]) != (row['timestamp']-origin)%2**32:
            raise ValueError('moving independent VP8 decoder changed original PTS')
        if first+at in selected:
            classified=source_pixels(rgb[at*3:at*3+3],peer)
            measured.append(dict(row=row,level=gray[at],hash=line.split(',')[-1].strip(),
                wrong_source=classified['wrong_source_pixel_frames'],
                other_source=classified['other_identified_source_frames'],
                unclassified=classified['unclassified_pixel_frames'],
                marker_color_counts=classified['source_marker_counts']))
    return dict(measurements=measured,undecodable_frames=sum(i<first for i in selected),decoder_started=True,
        ivf_sha256=checksum(ivf),framemd5_sha256=checksum(hashes),pulse_pixels_sha256=checksum(pulses),
        source_pixels_sha256=hashlib.sha256(rgb).hexdigest())


def arrival_marker_edges(levels, times, *, low, high, max_gap):
    """Interpret simultaneous arrivals without inventing sub-clock ordering.

    RTP uniqueness/order is checked separately by timeline(). A same-time group
    can establish one observed level only if every decoded value agrees on its
    region. Mixed regions break onset custody and require a fresh low rearm.
    These groups do not change decoded sample/frame coverage or timestamps.
    """
    if len(levels) != len(times) or len(levels) > 3200:
        raise ValueError('Decoded native marker observation bound')
    previous = None
    for level, at in zip(levels, times):
        if not math.isfinite(level) or level < 0 or not math.isfinite(at) or (previous is not None and at < previous):
            raise ValueError('Decoded native marker value/order')
        previous = at
    result = dict(edges_seconds=[], missing_intervals=0, unproven_onsets=0,
        tied_arrival_groups=0, ambiguous_arrival_groups=0)
    run_levels = []; run_times = []
    def flush():
        observed = marker_edges(run_levels, run_times, low=low, high=high, max_gap=max_gap)
        result['edges_seconds'].extend(observed['edges_seconds'])
        for key in ('missing_intervals', 'unproven_onsets'):
            result[key] += observed[key]
        run_levels.clear(); run_times.clear()
    for at, grouped in itertools.groupby(zip(times, levels), key=lambda row:row[0]):
        values = [row[1] for row in grouped]
        result['tied_arrival_groups'] += int(len(values) > 1)
        regions = {'low' if value <= low else 'high' if value > high else 'band' for value in values}
        if len(regions) != 1:
            flush()
            result['ambiguous_arrival_groups'] += 1
            result['missing_intervals'] += 1
            result['unproven_onsets'] += int('high' in regions)
            continue
        run_times.append(at); run_levels.append(values[0])
    flush()
    result.update(continuous_low_rearm_seconds=.2, maximum_observation_gap_seconds=max_gap)
    return result


def marker_alignment(audio,video):
    observed={}
    for medium,histories in [('audio',audio),('video',video)]:
        edges=[];missing=unproven=tied=ambiguous=0
        for history in histories:
            measurements=history['measurements']
            signal=arrival_marker_edges([m['level'] for m in measurements],[m['row']['elapsed_ms']/1000 for m in measurements],
                low=.045 if medium=='audio' else 16,high=.055 if medium=='audio' else 240,max_gap=.05 if medium=='audio' else .1)
            edges.extend(signal['edges_seconds']);missing+=signal['missing_intervals'];unproven+=signal['unproven_onsets']
            tied+=signal['tied_arrival_groups'];ambiguous+=signal['ambiguous_arrival_groups']
        observed[medium]=dict(edges_seconds=sorted(edges),missing_intervals=missing,unproven_onsets=unproven,
            tied_arrival_groups=tied,ambiguous_arrival_groups=ambiguous,
            decoder_and_onset_state_crossed_history_boundary=False)
    result=ordinal_sync(observed['video']['edges_seconds'],observed['audio']['edges_seconds'],60)
    result.update(audio_observation=observed['audio'],video_observation=observed['video'],
        physical_device_presentation_measured=False,live_decoder_completion_measured=False,
        time_basis='Independent decoded content on original receive times; state resets at each physical/stream/SSRC history; only uniquely calibrated equal cadence ordinals pair')
    return result


def summarize(route, audio, video):
    a=sorted((m for h in audio for m in h['measurements']),key=lambda m:m['row']['elapsed_ms'])
    v=sorted((m for h in video for m in h['measurements']),key=lambda m:m['row']['elapsed_ms'])
    if len(a)>3200 or len(v)>2000: raise ValueError('moving logical receiver observation bound')
    # The synthetic programme preserves its original frame indices through
    # every move. Repeated frames cannot inflate whole-window frame coverage.
    if len({m['row']['timestamp'] for m in v})!=len(v):
        raise ValueError('moving original programme video frame counted twice')
    quanta=[m['row'].get('original_programme_audio_quantum') for m in a]
    known=[q for q in quanta if q is not None]
    if len(known)!=len(a):raise ValueError('native /3 counted audio lacks original unambiguous source quantum identity')
    if len(known)!=len(set(known)):
        raise ValueError('moving original programme audio quantum counted twice across physical owners')
    bins=[sum(second*1000<=m['row']['elapsed_ms']<(second+1)*1000 for m in v) for second in range(2,62)]
    changing=sum(left['hash']!=right['hash'] for left,right in zip(v,v[1:]));wrong=sum(m['wrong_source'] for m in v)
    # Retain the classifier's distinction without relaxing the existing wrong==0
    # quality limit. Older controlled measurements without these observations
    # remain explicitly unavailable rather than becoming zero other-source data.
    classification=(dict(expected_source_index=route['peer_index'],
        source_marker_counts=[sum(m['marker_color_counts'][index] for m in v) for index in range(6)],
        other_identified_source_frames=sum(m['other_source'] for m in v),
        unclassified_pixel_frames=sum(m['unclassified'] for m in v))
        if v and all('marker_color_counts' in m and 'other_source' in m and 'unclassified' in m for m in v) else None)
    gaps=[b['row']['elapsed_ms']-a['row']['elapsed_ms'] for a,b in zip(a,a[1:])]
    sync=marker_alignment(audio,video);bad=sum(h['bad_identity_blocks'] for h in audio)
    samples=sum(h['measured_samples'] for h in audio)
    passed=(bool(a) and samples>=59.9*48000 and samples==len(a)*960 and bad==0
        and max(gaps,default=math.inf)<=200 and len(v)>=1440 and changing>=1440 and min(bins)>=24
        and wrong==0 and sync['status']=='passed')
    passed = passed and route['received_clocks']['audio'] > 0 and route['received_clocks']['video'] > 0
    return dict(index=route['index'],peer_index=route['peer_index'],passed=passed,
        audio=dict(decoded_seconds=samples/48000,samples=samples,bad_identity_blocks=bad,
            original_window_packets=len(a),source_quantum_identity_available=len(known),source_quantum_identity_unavailable=len(a)-len(known),maximum_arrival_gap_ms=max(gaps,default=None)),
        video=dict(decoded_window_frames=len(v),changing_window_frames=changing,changing_fps=changing/60,
            frames_by_observation_second=bins,wrong_source_pixel_frames=wrong,
            source_marker_classification=classification,
            undecodable_frames=sum(h['undecodable_frames'] for h in video)),
        excluded_rows=route['excluded_rows'],received_clocks=route['received_clocks'],independently_decoded_receive_marker_alignment=sync)


def verify(directory,manifest,metadata):
    directory=Path(directory).resolve()
    if manifest.get('markers')!=marker_profile() or manifest.get('seconds')!=62 or manifest.get('observation_seconds')!=60:
        raise ValueError('moving source programme differs from frozen profile')
    bundle,routes=timeline(directory,metadata)
    output=directory/'moving-decoded';output.mkdir(mode=0o700)
    results=[];number=0;decoders_started=0
    for route in routes:
        decoded={'audio':[],'video':[]};custody=[]
        for history in route['histories']:
            name=f"{history['medium']}-{number:03d}";number+=1
            observed=(audio_decode(output,name,history,manifest['sources'][route['peer_index']]['audio_hz'])
                if history['medium']=='audio' else video_decode(output,name,history,route['peer_index']))
            decoded[history['medium']].append(observed)
            decoders_started += int(history['medium']=='audio' or observed['decoder_started'])
            custody.append({k:v for k,v in history.items() if k not in ('rows','selected_indices') } |
                dict(original_rows=len(history['rows']),selected_original_rows=len(history['selected_indices']),
                    artifacts={k:v for k,v in observed.items() if k!='measurements'}))
        result=summarize(route,decoded['audio'],decoded['video']);result['physical_codec_histories']=custody;results.append(result)
    report=dict(format='connect/moving-native-av-decoded/1',passed=all(r['passed'] for r in results),
        original_moves=TOTAL_MOVES,original_captures=6,physical_captures=TOTAL_MOVES,connections=6,participants=4,sessions=2,
        movement_profile=MOVEMENT_PROFILE,movement_profile_sha256=MOVEMENT_PROFILE_SHA256,
        warmup_seconds=2,observation_seconds=60,source_manifest=manifest,capture_bundle_sha256=metadata['sha256'],
        physical_codec_histories_inspected=number,independently_decoded_codec_histories=decoders_started,routes=results,
        full_timeline_decoded=all(r['video']['undecodable_frames']==0 for r in results),task_complete=False,cell_qualified=False,matrix_qualified=False,whole_gate_qualified=False,
        scope='Independent whole-window receive-content component. Private context is actual same-physical/stream/SSRC/generation/epoch decoder history and never coverage. Public ownership uses the original signed native writer cut independently of delayed root reply arrival. Actual public output from original admitted owners and exact replacement cuts remains on its original receive clock; source-quantum overlaps fail rather than inflate coverage. Every physical/stream/SSRC history decodes separately with original timestamps; the original completion record binds each immutable cut and actual retirement remains required. No PLC, gaps filled, retiming, duplicated-frame coverage or inferred device presentation. Impairment/browser/remote cells remain separate.')
    (output/'decoded.json').write_text(json.dumps(report,indent=2)+'\n')
    return report
