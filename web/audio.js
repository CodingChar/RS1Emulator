// Web Audio output. The emulator produces 44.1 kHz stereo; the context runs
// at the same rate so no resampling is needed.

export class AudioOutput {
    constructor() {
        this.context = null;
        this.node = null;
        this.gain = null;
        this.volume = 0.8;
        this.muted = false;
        this.failed = false;
    }

    /** Create the audio graph. Must run in response to a user gesture. */
    async start(sampleRate = 44100) {
        if (this.context || this.failed) {
            if (this.context && this.context.state === 'suspended') await this.context.resume();
            return;
        }
        try {
            this.context = new AudioContext({ sampleRate, latencyHint: 'interactive' });
            await this.context.audioWorklet.addModule('audio-worklet.js');
            this.node = new AudioWorkletNode(this.context, 'ps1-audio', {
                numberOfInputs: 0,
                outputChannelCount: [2],
            });
            this.gain = this.context.createGain();
            this.node.connect(this.gain).connect(this.context.destination);
            this.applyVolume();
            await this.context.resume();
        } catch (error) {
            console.warn('Audio unavailable:', error);
            this.failed = true;
            this.context = null;
        }
    }

    applyVolume() {
        if (this.gain) this.gain.gain.value = this.muted ? 0 : this.volume;
    }

    setVolume(value) {
        this.volume = value;
        this.applyVolume();
    }

    setMuted(muted) {
        this.muted = muted;
        this.applyVolume();
    }

    /** Queue interleaved stereo samples (a Float32Array the callee takes over). */
    push(samples) {
        if (this.node && samples.length > 0) {
            this.node.port.postMessage(samples, [samples.buffer]);
        }
    }

    get available() {
        return this.node !== null;
    }
}
