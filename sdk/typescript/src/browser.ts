/** Browser-only Connect surface. This entry point imports no Node modules. */
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
