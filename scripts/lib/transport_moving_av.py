"""Decode every physical codec history before measuring six logical receivers.

Ownership intervals are original completed-root observations on the unchanged
programme clock. Decoder state, compressed bytes and presentation timestamps
never cross physical receiver, stream or SSRC boundaries.
"""
import array
import base64
import hashlib
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
        key = (stream, ssrc)
        history = histories.setdefault(key, [])
        if history:
            distance = (timestamp - history[-1]['timestamp']) % 2 ** 32
            if not 0 < distance < 2 ** 31:
                raise ValueError('moving codec history repeated or regressed original RTP clock')
        if medium == 'video' and type(row['key']) is not bool:
            raise ValueError('moving original VP8 keyframe flag')
        history.append(row)
    if len(histories) > 32:
        raise ValueError('moving physical codec history bound')
    return [(stream, ssrc, history) for (stream, ssrc), history in histories.items()]


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
            if (capture['format'] != 'connect/physical-av-segment/1' or capture['program_start_unix_us'] != origin
                    or segment.get('scheduled_start_ms') != scheduled
                    or type(actual) not in (int, float) or not math.isfinite(actual) or not scheduled <= actual <= cut < 62000
                    or cuts and cut <= cuts[-1]
                    or segment['connection_id'] in candidates or segment['reliable_data']['original_data_messages_bound'] != 2):
                raise ValueError('moving original ownership boundary or Data custody')
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
        end = chains[receiver][1][0] if receiver in chains else 62000
        physical = [(f'original-{receiver}', 0, end, captured,
            rows(captured['audio'], 'audio', {streams[receiver]['audio']}, streams[receiver]['audio']),
            rows(captured['video'], 'video', allowed_video))]
        if receiver in chains:
            chain, cuts = chains[receiver]
            for ordinal, segment in enumerate(chain):
                capture = segment['capture']; audio = []; video = []
                for medium, output in [('audio', audio), ('video', video)]:
                    for history in capture[medium]:
                        if history['rows']:
                            if any(row['elapsed_ms'] < cuts[ordinal] for row in history['rows']):
                                raise ValueError('moving pre-completion rows cannot initialize a physical decoder')
                            output.extend(rows(history['rows'], medium,
                                {segment['published_stream_ids']['audio']} if medium == 'audio' else {streams[peer]['video']}))
                physical.append((f'physical-{receiver}-{ordinal}', cuts[ordinal], cuts[ordinal+1] if ordinal + 1 < MOVES_PER_SESSION else 62000,
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
                    bounded = [row for row in group if row['elapsed_ms'] < end]
                    selected = [i for i,row in enumerate(bounded) if max(2000,start) <= row['elapsed_ms'] < min(62000,end)]
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
                wrong_source=classified['wrong_source_pixel_frames']))
    return dict(measurements=measured,undecodable_frames=sum(i<first for i in selected),decoder_started=True,
        ivf_sha256=checksum(ivf),framemd5_sha256=checksum(hashes),pulse_pixels_sha256=checksum(pulses))


def marker_alignment(audio,video):
    observed={}
    for medium,histories in [('audio',audio),('video',video)]:
        edges=[];missing=unproven=0
        for history in histories:
            measurements=history['measurements']
            signal=marker_edges([m['level'] for m in measurements],[m['row']['elapsed_ms']/1000 for m in measurements],
                low=.045 if medium=='audio' else 16,high=.055 if medium=='audio' else 240,max_gap=.05 if medium=='audio' else .1)
            edges.extend(signal['edges_seconds']);missing+=signal['missing_intervals'];unproven+=signal['unproven_onsets']
        observed[medium]=dict(edges_seconds=sorted(edges),missing_intervals=missing,unproven_onsets=unproven,
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
    bins=[sum(second*1000<=m['row']['elapsed_ms']<(second+1)*1000 for m in v) for second in range(2,62)]
    changing=sum(left['hash']!=right['hash'] for left,right in zip(v,v[1:]));wrong=sum(m['wrong_source'] for m in v)
    gaps=[b['row']['elapsed_ms']-a['row']['elapsed_ms'] for a,b in zip(a,a[1:])]
    sync=marker_alignment(audio,video);bad=sum(h['bad_identity_blocks'] for h in audio)
    samples=sum(h['measured_samples'] for h in audio)
    passed=(bool(a) and samples>=59.9*48000 and samples==len(a)*960 and bad==0
        and max(gaps,default=math.inf)<=200 and len(v)>=1440 and changing>=1440 and min(bins)>=24
        and wrong==0 and sync['status']=='passed')
    passed = passed and route['received_clocks']['audio'] > 0 and route['received_clocks']['video'] > 0
    return dict(index=route['index'],peer_index=route['peer_index'],passed=passed,
        audio=dict(decoded_seconds=samples/48000,samples=samples,bad_identity_blocks=bad,
            original_window_packets=len(a),maximum_arrival_gap_ms=max(gaps,default=None)),
        video=dict(decoded_window_frames=len(v),changing_window_frames=changing,changing_fps=changing/60,
            frames_by_observation_second=bins,wrong_source_pixel_frames=wrong,
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
        scope='Independent whole-window receive-content component. Every physical/stream/SSRC history decodes separately with original timestamps; actual root observations delimit ownership. No PLC, gaps filled, retiming, duplicated-frame coverage or inferred device presentation. Impairment/browser/remote cells remain separate.')
    (output/'decoded.json').write_text(json.dumps(report,indent=2)+'\n')
    return report
