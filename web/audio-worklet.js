// Plays the stereo samples the emulator sends from the main thread.
// A short queue absorbs the jitter between emulated frames and the audio
// clock; if the emulator falls behind the output goes silent instead of
// repeating old samples, and if it runs ahead the oldest audio is dropped.

const MAX_BUFFERED_FRAMES = 44100 * 0.3;

class Ps1AudioProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        this.chunks = [];
        this.offset = 0; // read position (in frames) inside chunks[0]
        this.buffered = 0; // frames queued
        const receive = (event) => {
            const samples = event.data;
            if (!(samples instanceof Float32Array) || samples.length === 0) return;
            this.chunks.push(samples);
            this.buffered += samples.length / 2;
            while (this.buffered > MAX_BUFFERED_FRAMES && this.chunks.length > 1) {
                const dropped = this.chunks.shift();
                this.buffered -= (dropped.length / 2) - this.offset;
                this.offset = 0;
            }
        };
        // Samples arrive on this port, or on a channel the page hands over so
        // that a worker can send them straight here.
        this.port.onmessage = (event) => {
            if (event.data && event.data.port) {
                event.data.port.onmessage = receive;
            } else {
                receive(event);
            }
        };
    }

    process(_inputs, outputs) {
        const output = outputs[0];
        const left = output[0];
        const right = output[1] || output[0];
        for (let i = 0; i < left.length; i++) {
            const chunk = this.chunks[0];
            if (!chunk) {
                left[i] = 0;
                right[i] = 0;
                continue;
            }
            left[i] = chunk[this.offset * 2];
            right[i] = chunk[this.offset * 2 + 1];
            this.offset++;
            this.buffered--;
            if (this.offset * 2 >= chunk.length) {
                this.chunks.shift();
                this.offset = 0;
            }
        }
        return true;
    }
}

registerProcessor('ps1-audio', Ps1AudioProcessor);
