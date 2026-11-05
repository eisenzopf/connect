import type { BrowserCaptureInput } from "../typescript/dist/browser.js";
declare const node: AudioNode;
declare const video: MediaStream;
const audio: BrowserCaptureInput = { audio: true, audioNode: node };
const programme: BrowserCaptureInput = { audio: true, audioNode: node, video: "camera", stream: video };
const camera: BrowserCaptureInput = { audio: true, audioNode: node, video: "camera", videoDeviceId: "camera" };
const ordinary: BrowserCaptureInput = { audio: true, audioDeviceId: "microphone" };
// @ts-expect-error Graph capture must explicitly select negotiated audio.
const missingAudio: BrowserCaptureInput = { audioNode: node };
// @ts-expect-error Device and graph audio cannot be selected together.
const ambiguousAudio: BrowserCaptureInput = { audio: true, audioNode: node, audioDeviceId: "microphone" };
void [audio, programme, camera, ordinary, missingAudio, ambiguousAudio];
