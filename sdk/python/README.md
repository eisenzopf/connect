# Connect Python SDK

The candidate distribution is `rudeless-connect`; Python imports remain
`connect_sdk`. Python 3.11 or later is required. The unrelated PyPI distribution
named `connect-sdk` is not this SDK.

Native APIs use the `connect` executable from the same candidate on PATH.
The candidate manifest pins its platform, source revision, checksum and helper
protocol versions. Explicit HTTP clients use their configured endpoint directly.

Install the candidate wheel with `python -m pip install --no-index ./rudeless_connect-0.1.0-py3-none-any.whl`.
Use the SDK examples and documentation shipped alongside the matching candidate.
Candidate packaging and complete platform qualification are still in progress.

Source and API documentation: https://github.com/eisenzopf/connect

MIT licensed; see LICENSE.
