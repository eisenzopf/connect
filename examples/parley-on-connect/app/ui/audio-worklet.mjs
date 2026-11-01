// One 20ms PCM buffer and one transferred message. Receiver grants each credit.
class ParleyCapture extends AudioWorkletProcessor {
    constructor() {
        super(); this.samples = new Float32Array(960); this.offset = 0; this.credit = true; this.stopped = false;
        this.port.onmessage = ({ data }) => {
            if (data === "stop") { this.stopped = true; this.samples.fill(0); }
            if (data === "credit") this.credit = true;
        };
    }
    process(inputs, outputs) {
        if (this.stopped) return false;
        for (const output of outputs) for (const channel of output) channel.fill(0);
        const channels = inputs[0];
        if (!channels?.length) return true;
        if (channels.length > 2) throw Error("Unsupported microphone channel count");
        for (let i = 0; i < channels[0].length; i++) {
            let sample = 0;
            for (const channel of channels) sample += channel[i] / channels.length;
            if (!Number.isFinite(sample)) throw Error("Invalid capture sample");
            this.samples[this.offset++] = sample;
            if (this.offset === 960) {
                if (this.credit) {
                    this.credit = false;
                    const samples = this.samples;
                    this.samples = new Float32Array(960);
                    this.port.postMessage({ samples, firstFrame: currentFrame + i - 959 }, [samples.buffer]);
                }
                this.offset = 0;
            }
        }
        return true;
    }
}
registerProcessor("parley-capture", ParleyCapture);
