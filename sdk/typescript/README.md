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

Source and API documentation: https://github.com/eisenzopf/connect

MIT licensed; see LICENSE.
