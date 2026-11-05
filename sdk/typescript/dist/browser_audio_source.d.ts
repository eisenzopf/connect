/** A graph source is borrowed; its context and unrelated edges remain caller-owned. */
export declare function captureAudioNode(input: {
    audioNode?: AudioNode;
    audioDeviceId?: string;
    stream?: MediaStream;
}, audio: boolean): AudioNode | undefined;
export declare function captureWorkletModule(context: AudioContext, signal: AbortSignal): Promise<void>;
