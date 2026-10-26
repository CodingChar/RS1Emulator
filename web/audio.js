// Web Audio output. The emulator produces 44.1 kHz stereo; the context runs
// at the same rate so no resampling is needed. The processor in
// audio-worklet.js keeps a steady reserve of samples (see there).

/** The volume slider (0 to 1) as a gain: loudness is perceived roughly as the square root of the amplitude. */
export function gainForVolume(volume) {
    const v = Math.max(0, Math.min(1, volume));
    return v * v;
}

export class AudioOutput {
    constructor() {
        this.context = null;
        this.node = null;
        this.gain = null;
        this.volume = 0.8;
        this.muted = false;
        this.latency = 'auto'; // ms of audio kept in reserve, or 'auto'
        this.active = true;
        this.failed = false;
        /** What the audio thread last reported: reserve in ms, dropouts, ... */
        this.stats = null;
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
            if (this.context.state !== 'running') {
                await this.context.resume().catch(() => {});
                this.notify();
            }
            return;
        }
        try {
            try {
                this.context = new AudioContext({ sampleRate, latencyHint: 'interactive' });
            } catch (_) {
                // A device that cannot run at 44.1 kHz: the player resamples to its own rate.
                this.context = new AudioContext({ latencyHint: 'interactive' });
            }
            this.context.onstatechange = () => this.notify();
            await this.context.audioWorklet.addModule('audio-worklet.js');
            this.node = new AudioWorkletNode(this.context, 'ps1-audio', {
                numberOfInputs: 0,
                outputChannelCount: [2],
            });
            this.node.port.onmessage = (event) => {
                this.stats = event.data;
            };
            this.gain = this.context.createGain();
            this.node.connect(this.gain).connect(this.context.destination);
            this.applyVolume(true);
            this.applyLatency();
            this.node.port.postMessage({ active: this.active });
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

    /** Set the gain; it glides there in a few milliseconds so moving the slider does not click. */
    applyVolume(immediately = false) {
        if (!this.gain) return;
        const value = this.muted ? 0 : gainForVolume(this.volume);
        const param = this.gain.gain;
        if (immediately) {
            param.value = value;
        } else {
            param.setTargetAtTime(value, this.context.currentTime, 0.012);
        }
    }

    /** Volume from 0 to 1. */
    setVolume(value) {
        this.volume = value;
        this.applyVolume();
    }

    setMuted(muted) {
        this.muted = muted;
        this.applyVolume();
    }

    /**
     * Milliseconds of sound kept in reserve, or 'auto' to let the player grow it when the
     * sound runs dry and shrink it when it does not: more is safer on a slow machine,
     * less is more immediate.
     */
    setLatency(milliseconds) {
        this.latency = milliseconds;
        this.applyLatency();
    }

    /** Whether the emulator is producing sound: running at normal speed, not paused. */
    setActive(active) {
        if (active === this.active) return;
        this.active = active;
        if (this.node) this.node.port.postMessage({ active });
    }

    applyLatency() {
        if (this.node) this.node.port.postMessage({ target: this.latency });
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

    get available() {
        return this.node !== null;
    }
}
