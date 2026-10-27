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

Open `http://127.0.0.1:8088/examples/conversation-client/`. HTTP messaging and
Session controls use the local API fields. Browser media uses the separate
**Signed media configuration** file described in
[canonical WebRTC](../../docs/developer/canonical-webrtc-signaling.md#public-browser-session-api).
It requires enrolled client/server signing keys, an authorized WebRTC Endpoint,
an accepted Session and an explicitly installed canonical WSS media host. Default
server startup does not install that host. The former bearer-only loopback route
cannot be used by this browser API.

Choose **Join voice/video** to establish signaling and owned capture. After both
peers have answered, choose **Receive selected sources** to apply the file's
original explicit subscription. An answer or successful playback call alone does
not prove received or decoded media. **Hang up/Leave video** submits the file's
original Connection end and observes its journal status. Local capture loss/page
cleanup only retires owned resources. It does not invent a durable mutation.
Keys, credentials and SDP stay in memory; keep the configuration outside the
static asset directory. Inspect diagnostics for bounded counters without SDP,
credentials or network addresses. The installed-browser operating profile remains
under G6-051 qualification.

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

Keep the returned Conversation ID. The setup above prepares HTTP/native test
state; it does not provision canonical browser media. Use the separately enrolled
signed media configuration and explicit canonical host for that lane. Serve only
the matching public assets as described above.

Open `http://127.0.0.1:8088/examples/conversation-client/` in two windows.
Privately open the generated client configurations to populate the fields:

| Field | Human window | Agent window |
| --- | --- | --- |
| HTTP API | `http://127.0.0.1:8091` | `http://127.0.0.1:8091` |
| Your identity ID | `human.toml` → `identity_id` | `agent.toml` → `identity_id` |
| Other identity ID | `agent.toml` → `identity_id` | `human.toml` → `identity_id` |
| Credential | `human.toml` → `credential` | `agent.toml` → `credential` |
| Conversation ID | Returned Conversation ID | Same Conversation ID |

Choose **Connect**, then **Refresh timeline** in each window. Select a separate
signed media configuration for each identity before joining browser media.
Send a message and reply before proceeding to the audio/video controls below.
Keep the state directory private and out of the static-server root. When done,
stop only these owned servers with Ctrl-C; retain the state for restart.

## Signed browser audio/video and continuity

Use two permitted identities in two browser windows with the same explicit
Conversation ID. The server must expose its HTTP API and explicitly installed canonical WSS
media host; the selected identities need current Session/media authority. Use
**Invite audio + video** in one window and copy the returned Session ID to the
other. **Reject** leaves devices unopened. For a new invitation, choose **Accept**
before either participant chooses **Join video**. Select available camera and
microphone devices, allow the browser permissions, and verify remote playback
in both windows after explicit receive selection. **Inspect video** reports bounded media diagnostics.

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
