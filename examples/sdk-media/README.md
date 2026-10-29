# App-owned PCMU and VP8 file bridge

These Node and Python examples connect pre-encoded files to the public negotiated
packet API. They packetize PCMU/VP8, receive bounded complete frames and retain
ordinary `.mulaw` and `.ivf` files for an independently chosen decoder. The
Connect helper and server do not launch FFmpeg or capture devices.

The finite example supports mono PCMU at 8 kHz, 160 samples per 20 ms packet,
RTP PT 0; and VP8 at 640×360, a 90 kHz RTP clock and PT 96. Check the original
Connection's negotiation before choosing these utilities. Another negotiated
format needs its own application codec. File input is limited to three seconds,
24,000 PCMU bytes, 90 video frames and a 4 MiB IVF file. Each encoded video frame
is at most 256 KiB; complete RTP packets are at most 1,180 bytes.

Use a new private directory under ignored `.local/`. Input/output files must
remain private; the directory should have mode 0700. Output files must be new.
The caller owns original logical IDs, Connection offers/subscriptions/end
mutations, and Client shutdown. Preserve IDs through uncertain control replies.
Examples use only the public SDK interface; importing a utility grants no media
custody.

For a local Node checkout, build the SDK and install the example dependency:

```sh
npm run build --prefix sdk/typescript
npm install --offline --ignore-scripts --no-package-lock --prefix examples/sdk-media
```

For an external project, install the built SDK package and copy
`packet-codec.mjs` beside your application. With original negotiated handles:

```javascript
import {loadMedia, exchangeMedia} from './packet-codec.mjs';

const input = await loadMedia('.local/media/input.mulaw', '.local/media/input.ivf');
// Each argument is an original Stream.media handle. Start the peer concurrently.
const capture = await exchangeMedia(
    sendingAudio, sendingVideo, receivingAudio, receivingVideo,
    input, '.local/media/received', 0x41560001,
);
// capture reports counts. The .mulaw/.ivf files contain the received media.
```

Install the Python wheel normally and copy `packet_codec.py` beside your
application. The equivalent asynchronous call is:

```python
from packet_codec import load_media, exchange_media

inputs = load_media('.local/media/input.mulaw', '.local/media/input.ivf')
capture = await exchange_media(
    sending_audio, sending_video, receiving_audio, receiving_video,
    inputs, '.local/media/received', 0x41560001,
)
```

Each call sends and receives concurrently and joins its finite work before
returning. Media requests share a four-second application deadline; each
public SDK request uses the remaining budget. The caller closes the original
Client in `finally`/an async context, including after an error. There is no
automatic reconnect or replay.

Choose unique publisher labels and subscribe explicitly to the other
publisher's labels. Refresh `connection.media` after selection to find the actual
receive bindings. Current native audio uses the local negotiated mixed-audio
sink; selected video and reliable data use the matched remote logical Stream.
The [Node](../../sdk/tests/node_sdk_server_consumer.mjs) and
[Python](../../sdk/tests/python_native_server_consumer.py) installed consumers
show the full public offer, selection, both-direction exchange and cleanup.
Do not derive receive custody solely by reconstructing an ID.

VP8 output uses the minimal RFC 7741 descriptor: one partition, with the start
bit on the first fragment and RTP marker on the last. The example receiver
accepts this descriptor subset, preserves packet sequence/timestamp/SSRC and
holds one incomplete frame for at most 200 ms. Missing, out-of-order or expired
fragments discard that frame and suppress dependent frames until a fresh
keyframe. It refuses extended descriptors and a changed SSRC. A new source needs
a new assembler. This finite example does not implement a jitter buffer or
reordering window. Live applications can supply their own bounded assembler.

The receiver validates exact IVF framing, frame lengths, PTS, VP8 keyframe
dimensions and RTP payload types. Gap/clock packets remain distinct SDK results;
this finite file exchange refuses an interrupted capture instead of recording
it as complete. RTP/IVF timestamps are media time, not physical playback time.

FFmpeg is an optional application dependency for encoding and independent
decoding. The local rehearsal uses FFmpeg 9.0.2 with `libvpx`; Node 26.7.0 and
Python 3.14.7 on macOS arm64. Other runtime/platform combinations remain part of
the Gate 6 matrix. For example, create synthetic sources explicitly:

```sh
ffmpeg -nostdin -n -f lavfi -i 'sine=frequency=440:sample_rate=8000:duration=0.6' \
  -ac 1 -ar 8000 -c:a pcm_mulaw -f mulaw .local/media/input.mulaw
ffmpeg -nostdin -n -f lavfi -i 'color=c=red:size=640x360:rate=10' \
  -frames:v 6 -an -c:v libvpx -g 1 -f ivf .local/media/input.ivf
```

The application can independently decode a retained capture:

```sh
ffmpeg -nostdin -n -f mulaw -ar 8000 -ac 1 -i .local/media/received.mulaw \
  -f s16le .local/media/received.s16le
ffmpeg -nostdin -n -f ivf -i .local/media/received.ivf \
  -frames:v 6 -pix_fmt rgb24 -f rawvideo .local/media/received.rgb
```

The rehearsal generator adds a grid to four distinct source colors, ensuring
fragmented video. Independent Rust checks decode the received outputs with
FFmpeg, classify every frame's expected remote color, and identify the remote
audio frequency against the other source frequencies. The observer reuses none
of the Node/Python packetization or assembly code. This proves identified
synthetic decoded output; it does not measure motion, live decoder latency,
physical capture/rendering or audiovisual synchronization.

Run local codec component checks after installing the example dependency:

```sh
python3 sdk/tests/check_packet_codec.py --output .local/media-codec-check
```

The [Gate 6 evidence](../../docs/evidence/gate-6/language-decoded-media.json)
records exact source/package/tool hashes, production listener results and
retained failed attempts. The complete installed SDK/facade/platform matrix and
Gate 6 acceptance remain open.
