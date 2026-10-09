# Conversation test client

This browser page exercises Connect's public browser SDK. It keeps a durable
conversation across messaging, files, availability, explicit Session control,
and an authenticated WebRTC audio leg. It is a test implementation for people,
not a consumer messenger.

From the repository root, serve the checked-out files on loopback:

```sh
python3 -m http.server 8088 --bind 127.0.0.1
```

Open `http://127.0.0.1:8088/examples/conversation-client/`. Start Connect with
its loopback HTTP and WebRTC listeners, then enter their addresses plus a
communications credential. The credential stays in page memory and is sent to
WebRTC signaling through a private WebSocket subprotocol value.

The configured server-side WebRTC route selects the durable conversation and
assistant. Browser metadata never chooses authority. Microphone denial and
unsupported signaling are reported in the page status area.
Use **Inspect voice path** during a call to see the selected ICE pair, candidate
types, codec, and inbound/outbound audio counters. Network addresses, credentials
and SDP are excluded. Playback startup has a deadline; output failure releases
the microphone leg.

## Native microphone and speaker

`native_voice.py` uses the public Python SDK and Rust helper to carry microphone
audio over UCTP/QUIC datagrams. It needs optional local audio-device support:

```sh
python3 -m venv .local/microphone-venv
.local/microphone-venv/bin/pip install sounddevice
PYTHONPATH=sdk/python .local/microphone-venv/bin/python \
  examples/conversation-client/native_voice.py \
  --config .local/demo/human.toml --conversation CONVERSATION_ID \
  --session ACCEPTED_AUDIO_SESSION_ID --participant ASSISTANT_ID \
  --executable target/debug/connect
```

Use an existing, accepted audio Session. Allow microphone access when macOS
asks, and preferably use headphones. Speak after the connected message;
press Enter to hang up. The example ends the Session while retaining the
Conversation. Capture and playback queues are bounded, and audio is not saved.
Its frame counts show media activity; human confirmation is still required to
establish intelligibility and successful interruption.
