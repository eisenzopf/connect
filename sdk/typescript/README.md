# Connect TypeScript SDK

The candidate package is `@rudeless/connect`; its browser entry point is
`@rudeless/connect/browser`. Node.js 22 or later is required.

Native APIs use the `connect` executable from the same candidate on PATH.
The candidate manifest pins its platform, source revision, checksum and helper
protocol versions. Explicit HTTP clients and the browser entry point use their
configured server endpoint directly.

The public Node declarations require the pinned `@types/node` dependency. Its
`undici-types` dependency and both original license files accompany the SDK archive.
Install the candidate and its dependency archives together:

```sh
npm install --offline --ignore-scripts ./dependencies/*.tgz ./rudeless-connect-0.1.0.tgz
```
Use the SDK examples and documentation shipped alongside the matching candidate.
Candidate packaging and complete platform qualification are still in progress.

For finite canonical reads, `await client.raw.read(envelope, options)` uses the
existing Rust owner to validate the full frozen schema and perform the signed
exchange. It accepts `conversation.list`, `conversation.get`, `message.history`,
`operation.get`, and `conversation.subscribe` with explicit `live: false`.
It captures the input before awaiting, enforces the ordinary request deadline,
and creates no synchronous validation subprocess. Each exchange checks current
authorization; reads are neither cached nor retried. Mutations continue to use
`raw.validate` and `raw.request` with their captured original logical intent.

Canonical browser capture accepts explicit caller-owned sources:

```ts
const capture = await connection.capture({
  audio: true,
  audioNode: applicationMixOutput, // running real-time 48 kHz AudioContext
  video: "camera",
  videoCanvas: applicationCanvas, // fixed intrinsic size, at most 640 × 360
}, { timeoutMs: 5000, signal });
capture.close(); // retires only this Connection's derived tracks/graph edges
```

The application keeps its original canvas, draw clock, AudioNode and context.
The canvas cannot be combined with a camera device ID, screen capture or a
supplied video track. Both transport paths retain their existing 30fps, codec,
deadline and capture ownership bounds. Ordinary camera and supplied-track
capture remain separate explicit inputs.

Source and API documentation: https://github.com/eisenzopf/connect

MIT licensed; see LICENSE.
