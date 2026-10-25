/** Browser-only Connect surface. This entry point imports no Node modules. */
export {
    BrowserMediaRenderer,
    type BrowserMediaRendererOptions,
} from "./browser_media_renderer.js";
export {
    BrowserMediaCapture,
    browserCaptureSupport,
    type BrowserMediaCaptureOptions,
} from "./browser_media_capture.js";
export {
    BrowserMediaPeer,
    BrowserMediaConnection,
    type BrowserMediaCallbacks,
    type BrowserConnectionOptions,
    type BrowserDataBody,
    type BrowserDataProtocol,
} from "./browser_media.js";
export {
    BrowserClient,
    browserTransportSupport,
    type BrowserClientOptions,
    type RequestOptions,
} from "./browser_client.js";
export type {
    SigningAlgorithm,
    SigningIdentity,
    ServerTrust,
} from "./uctp_signing.js";
export type { Envelope, Scope } from "./uctp_wire.js";
export type { Json, JsonObject } from "./uctp_json.js";
export {
    BrowserOpusCodec,
    BrowserVp8Codec,
    browserCodecSupport,
    RtpClock,
    OPUS_CONFIG,
    VP8_CONFIG,
    type CodecSupport,
    type EncodedMedia,
} from "./browser_codecs.js";
export {
    MediaBindings,
    RtpSender,
    Vp8Assembler,
    MediaSendQueue,
    decodeRtpDatagram,
    MEDIA_LIMITS,
    type MediaBinding,
    type RtpDatagram,
} from "./browser_rtp.js";
export { BrowserHttpClient } from "./browser_http.js";
export {
    BrowserVoiceSession,
    type BrowserVoiceOptions,
} from "./browser_voice.js";
export type { VoiceDiagnostics } from "./browser_voice_stats.js";
export { ConnectError } from "./errors.js";
export type * from "./types.js";

export type * from "./tasks.js";
export { TASK_PROFILE } from "./tasks.js";
export { BrowserVideoSession } from "./browser_video.js";
export type { VideoDiagnostics } from "./browser_voice.js";
