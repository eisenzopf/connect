# Connect

**Communication for people, apps, and agents.**

An open communications server and SDK from [Rudeless](https://rudeless.ai). One interface to reach out and be reached.

## Status

Connect is in development. This repository is the home for the server, SDK, and future releases. There is no installable server or SDK release yet. The capabilities below describe what we intend to build.

## What we're building

Connect will let applications and AI agents exchange messages, share files, and communicate over voice and video through a shared API. The server will bring conversations, media, and connectors into one process. The SDK will make those capabilities straightforward to use.

An application will be able to use Connect to communicate outward and expose the same interface so other applications and agents can reach it.

The server and SDK come first. A reference client will let people try conversations, test capabilities, and see the SDK in use.

## A shared protocol

Connect will build on [rvoip](https://github.com/eisenzopf/rvoip) and [UCTP](https://rudeless.ai/uctp), with QUIC as the preferred native transport. Reliable streams will carry control traffic and datagrams will carry live media. UCTP will provide a common conversation model across transports.

Our ambition extends beyond this server: independent developers and service providers should be able to implement UCTP and communicate through the same open protocol.

## Connectors

Vapi, Telnyx, SIP, and WebRTC are the first integration priorities. We also want to connect Twilio, Vonage, WhatsApp, Telegram, RCS, and applications built by the community.

The connector library will provide shared interfaces and examples so developers can contribute integrations. Each connector will expose the capabilities its service supports.

## Help shape Connect

[Open an issue](https://github.com/eisenzopf/connect/issues) with a use case, connector idea, or protocol question. Tell us what you're building and who or what it needs to communicate with.

Connect grows from our work on [rvoip](https://github.com/eisenzopf/rvoip), [Thelve](https://rudeless.ai/thelve), and [Parley](https://github.com/eisenzopf/parley).
