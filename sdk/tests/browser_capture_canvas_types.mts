import type { BrowserCaptureInput } from "../typescript/dist/browser.js";
declare const canvas: HTMLCanvasElement;
declare const node: AudioNode;
declare const audio: MediaStream;
const graph: BrowserCaptureInput = { audio: true, audioNode: node, video: "camera", videoCanvas: canvas };
const microphone: BrowserCaptureInput = { audio: true, audioDeviceId: "microphone", video: "camera", videoCanvas: canvas };
const ownedAudio: BrowserCaptureInput = { audio: true, stream: audio, video: "camera", videoCanvas: canvas };
// @ts-expect-error A canvas explicitly selects camera video.
const noVideo: BrowserCaptureInput = { videoCanvas: canvas };
// @ts-expect-error Screen capture remains a separate explicit permission path.
const screen: BrowserCaptureInput = { video: "screen", videoCanvas: canvas };
// @ts-expect-error A canvas cannot claim an unrelated device selection.
const device: BrowserCaptureInput = { video: "camera", videoCanvas: canvas, videoDeviceId: "camera" };
void [graph, microphone, ownedAudio, noVideo, screen, device];
