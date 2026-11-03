# Your first Connect conversation

This guide uses the current Gate 6 review kit for **macOS ARM64 or Ubuntu 24.04
ARM64**. The kit contains the runtime, public Node SDK archives and local reference
app. It is an unpublished review artifact; the published v0.1 demo has a different
API. Use one matching kit throughout this guide.

Have these prerequisites installed before starting:

- Node.js 22.16 or newer, including npm.
- A Chromium browser and a writable directory owned by your ordinary user.
- On Ubuntu: `libopus0` and `libssl3t64` (`sudo apt-get install libopus0 libssl3t64`).
- The matching `connect-first-conversation-*.tar.gz` kit and its reviewed SHA-256.

No Rust compiler, source checkout, model account or communications provider is
needed. The participants are local synthetic people; changing perspective lets
you send the reply yourself. Start the first-conversation timer after the listed
prerequisites and archive are available. The target is under ten minutes through
the message and reply; three-developer usability qualification is separate.

## Install the kit

Check the archive against its supplied SHA-256 before extracting it. Choose a
new directory, replacing `/path/to/your-kit.tar.gz` with the downloaded kit:

```sh
mkdir connect-first
cd connect-first
tar -xzf /path/to/your-kit.tar.gz --strip-components=1
node verify.mjs
npm install --offline --ignore-scripts --no-audit --no-fund --no-save --cache .local/npm-cache ./sdk/rudeless-connect-0.1.0.tgz ./sdk/dependencies/*.tgz
export PATH="$PWD/runtime/bin:$PATH"
node app/run.mjs
```

Expect `Verified Connect first-conversation kit for darwin-arm64` or
`linux-arm64`, then a loopback URL after the app starts. Open the URL in your
browser. The command creates scoped local participants and starts its owned
server. Keep the terminal open. Its private state is in `.local/parley`; backend
credentials are kept out of the browser.

## Send a message and reply

1. Keep **Owner** as your perspective. Under **Choose people**, clear
   **Assistant (simulated)** and select **Organizer**. Choose **Create conversation**.
2. Type **Hello Connect** in **Message**, select Organizer as its audience and
   choose **Send message**. Wait for the completed operation and timeline entry.
3. Under **Your perspective**, select **Organizer**, then **Conversation 1**.
   Read the received message, type **Hello back**, select Owner as its audience
   and choose **Send message**.
4. Switch back to **Owner** and select **Conversation 1**. Both messages should
   appear once. There is no model producing the reply.

## Restart and keep history

Press **Ctrl-C** in the terminal and wait for the command to exit. Then repeat
this command in the same directory:

```sh
node app/run.mjs
```

Open or reload the printed URL, select Owner and Conversation 1, and check that
both messages remain. Keep the same runtime and `.local/parley` directory.
The original setup and operation IDs are retained automatically; no IDs need to
be copied. Press Ctrl-C again to join the app, server and SDK helpers.

## Add voice and video

The same app exposes **Join audio**, **Start microphone** and camera controls.
Capture starts only after your explicit action and browser permission. For the
media contract and supported paths, continue with [voice and video](../reference/index.html#voice).
A local preview alone does not prove that another participant received media.

## If setup stops

- **Node or npm is missing:** install Node22.16+ and open a new terminal.
- **Kit verification fails:** select the kit for your ARM operating system and
  compare its supplied checksum. Preserve a failed archive for inspection.
- **Setup stops during `installed-sdk`:** run the offline npm command first and
  export the runtime path in that same terminal. Use the SDK and runtime from
  the same kit; do not add repository-relative dependencies.
- **`private-app-state-required` or a write-permission error:** use an owned,
  writable private `.local` directory. Do not run as root or modify another
  installation's state. Preserve an interrupted setup and its private logs.
- **A pending action appears:** use **Recover original action**. After going
  offline, choose **Reconnect** explicitly; do not resubmit a pending mutation.
- **Microphone or camera is denied:** messages still work. Grant permission in
  your browser only when you want capture, then start it explicitly.

The kit is local and provider-free. It does not send SMS, place phone calls,
contact an AI account or qualify the later full Parley journey.
