class ConnectPcmCapture extends AudioWorkletProcessor {
    buffer = new Float32Array(960);
    offset = 0;
    firstFrame = 0;
    pending = 0;
    epoch = 0;
    origin = 0;
    active = true;
    constructor() {
        super();
        if (sampleRate !== 48000)
            throw new Error("Capture requires 48 kHz");
        this.port.onmessage = (event) => {
            const data = event.data;
            if (data?.type === "stop")
                this.active = false;
            else if (data?.type === "credit" && data.epoch === this.epoch)
                this.pending = Math.max(0, this.pending - 1);
            else if (data?.type === "reset" &&
                Number.isSafeInteger(data.epoch) &&
                Number.isFinite(data.originUs)) {
                this.epoch = data.epoch;
                this.origin = data.originUs;
                this.offset = this.pending = 0;
            }
        };
    }
    process(inputs) {
        if (!this.active)
            return false;
        const channels = inputs[0];
        if (!channels?.length)
            return true;
        for (let i = 0; i < channels[0].length; i++) {
            if (!this.offset)
                this.firstFrame = currentFrame + i;
            this.buffer[this.offset++] =
                channels.length > 1
                    ? (channels[0][i] + channels[1][i]) / 2
                    : channels[0][i];
            if (this.offset !== 960)
                continue;
            if (this.pending < 10) {
                this.port.postMessage({
                    epoch: this.epoch,
                    timestampUs: Math.round(this.origin +
                        (this.firstFrame * 1000000) / sampleRate),
                    samples: this.buffer,
                }, [this.buffer.buffer]);
                this.buffer = new Float32Array(960);
                this.pending++;
            }
            this.offset = 0;
        }
        return true;
    }
}
registerProcessor("connect-pcm-48k", ConnectPcmCapture);
export {};
