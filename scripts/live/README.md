# Explicit live audio qualification

These tools are invoked separately from `check-gate-3.sh`. They use a running,
configured Connect server and real provider service. They can incur provider
usage. They never purchase numbers, change provider accounts or run in CI.

`captured` means audio was collected, **not** that a test passed. Decode the
received recording independently, compare the spoken answers to conversation
facts, and inspect the durable session/context outcomes. Keep credentials,
recordings, transcripts and full provider responses under ignored `.local/`.
Publish only redacted behavior evidence. Preserve every failed attempt.

## Native UCTP/QUIC

Create mono signed 16-bit 8 kHz WAV speech fixtures and a JSON scenario:

```json
{"turns":[{"wav":"question.wav","wait_seconds":10}]}
```

A scenario has one to six turns, waits of zero to twenty seconds, and a maximum
of 120 seconds including initial silence. Optional `wait_for_reply: true` waits
for received speech energy before the following delay, with a twelve-second
deadline included in that bound. This helps establish an actual interruption
instead of sending a second prompt before the assistant starts speaking. On macOS, `say` and `afconvert` can
produce fixtures without microphone input. Run:

```sh
python3 scripts/live/native-voice.py --allow-live-provider \
  --config .local/human.toml --assistant-config .local/assistant.toml \
  --conversation CONVERSATION_ID --assistant ASSISTANT_IDENTITY_ID \
  --scenario .local/speech/scenario.json --output .local/voice-run
```

The public SDK creates and accepts a session, sends PCMU media over native QUIC,
records return media and ends the session. Each run uses an exclusive output
directory and persists logical mutation IDs before submission. Transport
uncertainty never triggers an automatic retry. Inspect receipts before recovery.

## Browser WebRTC

Serve the public SDK's built `dist` tree and `browser-voice.html`,
`browser-voice.js`, `capture-worklet.js`, scenario and WAV fixtures on loopback.
Keep private state outside that document root. The relative SDK import assumes
this repository layout; adapt that import if staging a smaller document root.
Enter the configured WebRTC route credential and a same-origin scenario URL.

The page supplies generated audio through `BrowserVoiceSession.open`'s optional
`inputStream`, negotiates real WebRTC, records received PCM, reports ICE/codecs,
and releases the supplied tracks on hangup. It does not request a microphone.
The normal conversation example continues to use the microphone by default.

Recording upload is optional and goes through `BrowserHttpClient` to the selected
Connect conversation. It uses 32 KiB chunks, SHA-256 and stable upload mutation
IDs. Failed/uncertain uploads retain their intent; changing settings is not a
substitute for inspecting the receipt. Download the recording through the public
artifact API and verify its hash before independent speech recognition.

For interruption tests, allow the assistant to start a deliberately long answer
before speaking the interrupting fixture. Check the recording timestamps rather
than assuming a fixed wait established an overlap. A provider transcript or
nonzero RTP counter alone does not prove intelligible audio or interruption.

Phone routing and provider-initiated hangup tests require their own explicit,
bounded operator rehearsal, with saved settings and verified restoration. See
[the Telnyx SIP runbook](../../examples/agent-presence/telnyx-sip-route.md).
