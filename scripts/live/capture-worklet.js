/** Bounded chunks of received PCM; analysis occurs on the main thread. */
class Capture extends AudioWorkletProcessor {
    constructor() {
        super();
        this.buffer = new Float32Array(4096);
        this.offset = 0;
    }
    process(inputs) {
        const samples = inputs[0]?.[0];
        if (samples)
            for (const sample of samples) {
                this.buffer[this.offset++] = sample;
                if (this.offset === this.buffer.length) {
                    this.port.postMessage(this.buffer, [this.buffer.buffer]);
                    this.buffer = new Float32Array(4096);
                    this.offset = 0;
                }
            }
        return true;
    }
}
registerProcessor("connect-capture", Capture);
