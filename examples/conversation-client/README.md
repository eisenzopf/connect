# Conversation test client

This browser page exercises Connect's public browser SDK. It keeps a durable
conversation across messaging, files, availability, explicit Session control,
and an authenticated WebRTC audio leg. It is a test implementation for people,
not a consumer messenger.

For an installed candidate, follow **Prepare the local reference server** below;
serve its matching runtime assets and keep state outside that directory.
Never serve a source repository root: its ignored `.local/` directory can contain
credentials and private logs.

For source development, build the TypeScript SDK first. From the repository root,
copy only the required public browser assets into a new temporary static root:

```sh
npm ci --prefix sdk/typescript
npm run build --prefix sdk/typescript
CONNECT_REFERENCE_ASSETS=$(python3 - <<'PY'
from pathlib import Path
import shutil, tempfile
root = Path.cwd()
public = Path(tempfile.mkdtemp(prefix="connect-reference-assets-"))
for relative in ("examples/conversation-client", "sdk/typescript/dist"):
    source = (root / relative).resolve(strict=True)
    for entry in source.rglob("*"):
        if not entry.is_file() or entry.is_symlink() or entry.suffix not in {".html", ".css", ".js", ".mjs"}:
            continue
        assert entry.resolve().is_relative_to(source), "Public asset escapes its source directory"
        target = public / relative / entry.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(entry, target)
assert (public / "examples/conversation-client/index.html").is_file()
assert (public / "sdk/typescript/dist/browser.js").is_file()
print(public)
PY
)
python3 -m http.server 8088 --bind 127.0.0.1 --directory "$CONNECT_REFERENCE_ASSETS"
```

The static copy preserves the page's relative SDK imports and excludes private
state, logs, source scripts and credentials. Restage after changing browser
sources or rebuilding the SDK. Do not write connection state into this static
root. Use the server configuration and identity mapping below with your selected
normal source-built executable in place of the installed path.

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
Server hangup also releases capture, detaches playback and restores **Join voice**.
The parent conversation remains available for the next call or message.

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

## Offline browser recovery check

`scripts/check-reference-client.mjs` starts a fresh local Connect installation,
serves this real page, and drives Chromium through the public HTTP SDK. It checks
101-message paging, a committed send with a lost acknowledgement followed by
reload and same-ID retry, exactly one resulting message, and voice refusal before
microphone acquisition. It uses no provider account and leaves the existing local
installation alone. Logs and a screenshot stay under ignored `.local/`.

After building `connect` and the TypeScript SDK, provide a Playwright installation
and Chrome executable (the qualified platform is local macOS arm64):

```sh
npm install --prefix .local/browser-tools playwright
CONNECT_PLAYWRIGHT_MODULE="$(pwd)/.local/browser-tools/node_modules/playwright/index.mjs" \
CONNECT_CHROME_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
node scripts/check-reference-client.mjs
```

The account-free workflow fixtures also run through `npm test --prefix sdk/typescript`.
A saved uncertain action retains its original inputs and request ID across page
reload; use **Retry saved action** before submitting a different action in that slot.
A definite first-attempt rejection can be corrected without retaining an uncertain
intent. Storage failure prevents submission rather than losing the retry identity.

## Prepare the local reference server

Use the matching installed runtime and a new state directory **outside** the
files you serve over HTTP. Python 3.11+ reads the generated TOML without an extra
package. Replace the executable path below with your selected candidate. Keep
these same two variables in each terminal used for this walkthrough:

```sh
export CONNECT_REFERENCE_BINARY=/absolute/path/to/runtime/bin/connect
export CONNECT_REFERENCE_STATE="$HOME/connect-reference-state"
umask 077
"$CONNECT_REFERENCE_BINARY" setup init \
  --directory "$CONNECT_REFERENCE_STATE" --bind 127.0.0.1:4465
"$CONNECT_REFERENCE_BINARY" serve \
  --config "$CONNECT_REFERENCE_STATE/server.toml" --http-bind 127.0.0.1:8091
```

Choose a new directory if that path already belongs to another installation.
The setup creates `human.toml` and `agent.toml` with reciprocal communication and
Session authority. The operator configuration is never entered in the page.
The first `serve` stays in the foreground. In a second terminal with the same
variables, create the shared conversation through the public CLI:

```sh
umask 077
python3 - <<'PY' | "$CONNECT_REFERENCE_BINARY" request \
  --config "$CONNECT_REFERENCE_STATE/human.toml" \
  > "$CONNECT_REFERENCE_STATE/reference-conversation.json"
import json, os, pathlib, tomllib
state = pathlib.Path(os.environ["CONNECT_REFERENCE_STATE"])
peer = tomllib.loads((state / "agent.toml").read_text())
print(json.dumps({"op": "create", "request_id": "reference-conversation-1",
                  "participants": [peer["identity_id"]]}))
PY
```

Stop your foreground server with Ctrl-C before changing its configuration.
Add the loopback signaling listener and route using the generated public IDs.
The route's `credential_id` is an identifier, not the secret `credential` value.
This command refuses to add a second WebRTC section and never prints a token:

```sh
python3 - <<'PY'
import json, os, pathlib, tomllib
state = pathlib.Path(os.environ["CONNECT_REFERENCE_STATE"])
human = tomllib.loads((state / "human.toml").read_text())
peer = tomllib.loads((state / "agent.toml").read_text())
reply = json.loads((state / "reference-conversation.json").read_text())
assert reply["kind"] == "conversation", "Inspect the saved create result before proceeding"
conversation = reply["data"]["id"]
server = state / "server.toml"
text = server.read_text()
assert "webrtc" not in tomllib.loads(text), "Existing WebRTC configuration preserved"
text += ('\n[webrtc]\nbind = "127.0.0.1:8092"\n[webrtc.route]\n'
         + 'credential_id = ' + json.dumps(human["credential_id"]) + '\n'
         + 'conversation_id = ' + json.dumps(conversation) + '\n'
         + 'assistant_identity_id = ' + json.dumps(peer["identity_id"]) + '\n')
server.write_text(text)
print("Reference Conversation ID:", conversation)
PY
"$CONNECT_REFERENCE_BINARY" serve \
  --config "$CONNECT_REFERENCE_STATE/server.toml" --http-bind 127.0.0.1:8091
```

The route deliberately omits `session_id`: each video join supplies the explicit
accepted Session, and the server rechecks its membership and current authority.
Keep the returned Conversation ID. Serve the matching runtime root in another
terminal, so both `examples/` and `sdk/` remain reachable by relative imports:

```sh
cd /absolute/path/to/runtime
python3 -m http.server 8088 --bind 127.0.0.1
```

Open `http://127.0.0.1:8088/examples/conversation-client/` in two windows.
Privately open the generated client configurations to populate the fields:

| Field | Human window | Agent window |
| --- | --- | --- |
| HTTP API | `http://127.0.0.1:8091` | `http://127.0.0.1:8091` |
| Your identity ID | `human.toml` → `identity_id` | `agent.toml` → `identity_id` |
| Other identity ID | `agent.toml` → `identity_id` | `human.toml` → `identity_id` |
| Credential | `human.toml` → `credential` | `agent.toml` → `credential` |
| Conversation ID | Returned Conversation ID | Same Conversation ID |

Choose **Connect**, then **Refresh timeline** in each window. Signaling is read
from the server before joining; do not substitute a browser-supplied route.
Send a message and reply before proceeding to the audio/video controls below.
Keep the state directory private and out of the static-server root. When done,
stop only these owned servers with Ctrl-C; retain the state for restart.

## Gate 5 local audio/video and continuity

Use two permitted identities in two browser windows with the same explicit
Conversation ID. The server must expose its configured loopback HTTP and WebRTC
listeners and the selected identities must have session/media authority. Use
**Invite audio + video** in one window and copy the returned Session ID to the
other. **Reject** leaves devices unopened. For a new invitation, choose **Accept**
before either participant chooses **Join video**. Select available camera and
microphone devices, allow the browser permissions, and verify remote playback
in both windows. **Inspect video** reports bounded media diagnostics.

**Mute camera** and **Mute microphone** change the local tracks. **Leave video**
releases the current media leg; **End** ends the Session for both participants.
The Conversation remains available. To call again, create and explicitly accept
a new Session. Device denial, capture loss and signaling failure should restore
the controls and release capture; record the displayed error if they do not.

For a continuity walkthrough, upload a small launch brief with a unique fact,
have the peer **Download and verify** it, and record a text decision during the
accepted call. End the Session, restart only the owned Connect server with the
same configuration/state, reconnect, and inspect the retained context and file.
Send a follow-up citing the actual decision and brief in that same Conversation.
A different Conversation must not acquire those sources. Live video does not
produce speech recognition, a transcript, a derived summary or model vision.

The public reference checker also observes this sequence with generated devices,
including attributed source/decision IDs, artifact bytes and SHA-256 after
restart, exact logical-ID replay, and one final follow-up. Its screenshots need a
separate visual review; generated capture does not qualify physical devices.
The [independent walkthrough protocol](../../docs/connect-first-ten-minutes.html#gate5-walkthrough)
records human timing, assistance, failures and visual observations separately.
