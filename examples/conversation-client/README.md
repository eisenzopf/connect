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
