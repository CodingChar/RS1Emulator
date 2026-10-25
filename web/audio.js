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
        /** Called when the context starts or stops running (browsers suspend it until a gesture). */
        this.onChange = null;
    }

    /** `running`, `suspended` (needs a click or key press) or `off` (not created or unavailable). */
    get status() {
        if (!this.context) return 'off';
        return this.context.state === 'running' ? 'running' : 'suspended';
    }

    /** Create the audio graph. Must run in response to a user gesture. */
    async start(sampleRate = 44100) {
        if (this.context) {
            if (this.context.state !== 'running') await this.context.resume().catch(() => {});
            this.notify();
            return;
        }
        try {
            this.context = new AudioContext({ sampleRate, latencyHint: 'interactive' });
            this.context.onstatechange = () => this.notify();
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
            this.context?.close().catch(() => {});
            this.context = null;
            this.node = null;
        }
        this.notify();
    }

    notify() {
        if (this.onChange) this.onChange();
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

    /**
     * Open a channel that feeds the audio thread directly and return the end
     * for the producer (to be transferred to the worker running the emulator).
     */
    createChannel() {
        if (!this.node) return null;
        const channel = new MessageChannel();
        this.node.port.postMessage({ port: channel.port2 }, [channel.port2]);
        return channel.port1;
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
