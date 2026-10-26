// Plays the stereo samples the emulator sends.
//
// The emulator produces audio in bursts, one frame at a time, on its own clock;
// the sound card consumes it on another. Two clocks never agree, so a naive queue
// slowly runs dry (and the sound cuts) or fills up (and the oldest audio is
// thrown away, which clicks). This processor keeps a reserve of audio and keeps
// that reserve steady the way emulators do ("dynamic rate control"): it plays
// slightly faster when the reserve grows and slightly slower when it shrinks
// (never more than 1 %, which cannot be heard), reading the samples with linear
// interpolation. If the reserve does run out anyway, the sound fades out instead
// of stopping dead, waits until the reserve is back and fades in.
//
// The emulator makes 44.1 kHz audio. If the sound card runs at another rate the
// same interpolation resamples it, so the pitch is right on any device.

/** The rate of the audio the emulator makes. */
const SOURCE_RATE = 44100;
const CAPACITY = 1 << 16; // frames kept at most (about 1.5 s)
const MASK = CAPACITY - 1;
const MAX_RATE_ADJUSTMENT = 0.01;
/** How fast the measured reserve follows the real one, per processed block (about 0.2 s). */
const SMOOTHING = 0.015;
const FADE_FRAMES = 128;
const REPORT_INTERVAL_S = 0.25;
/// The automatic reserve starts here, grows after every dropout and shrinks while all goes well.
const AUTO_START_S = 0.08;
const AUTO_MIN_S = 0.05;
const AUTO_MAX_S = 0.3;
const AUTO_CALM_S = 12;

class Ps1AudioProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        this.ring = new Float32Array(CAPACITY * 2);
        this.written = 0; // frames received so far
        this.position = 0; // frames played so far (fractional)
        this.base = SOURCE_RATE / sampleRate; // source frames per output frame at normal speed
        this.target = Math.round(AUTO_START_S * SOURCE_RATE); // the reserve to keep, in source frames
        this.auto = true; // let the reserve follow how the emulator is doing
        this.calm = 0; // seconds since the last dropout
        this.active = true; // the emulator is producing sound (not paused, not at another speed)
        this.smoothed = this.target;
        this.playing = false;
        this.fadeIn = 0; // frames left of a fade in
        this.underruns = 0;
        this.overruns = 0;
        this.rate = this.base;
        this.sinceReport = 0;

        // Samples arrive on this port, or on a channel the page hands over so
        // that a worker can send them straight here.
        const receive = (event) => this.receive(event.data);
        this.port.onmessage = (event) => {
            const message = event.data;
            if (message && message.port) {
                message.port.onmessage = receive;
            } else if (message && message.active !== undefined) {
                this.active = !!message.active;
            } else if (message && message.target !== undefined) {
                // A number of milliseconds, or 'auto'.
                this.auto = message.target === 'auto';
                if (!this.auto) this.target = Math.max(256, Math.round((message.target / 1000) * SOURCE_RATE));
            } else {
                this.receive(message);
            }
        };
    }

    receive(samples) {
        if (!(samples instanceof Float32Array) || samples.length < 2) return;
        const frames = samples.length >> 1;
        let at = this.written;
        for (let i = 0; i < frames; i++, at++) {
            const slot = (at & MASK) << 1;
            this.ring[slot] = samples[2 * i];
            this.ring[slot + 1] = samples[2 * i + 1];
        }
        this.written = at;
        // Far too much in reserve (the emulator ran ahead): drop the oldest, to the target.
        const limit = Math.min(CAPACITY - 1024, this.target * 4);
        if (this.written - this.position > limit) {
            this.position = this.written - this.target * 1.5;
            this.overruns++;
        }
    }

    process(_inputs, outputs) {
        const output = outputs[0];
        const left = output[0];
        const right = output[1] || output[0];
        const length = left.length;
        const reserve = this.written - this.position;

        if (!this.playing) {
            // Waiting for the reserve to build up (at the start, or after running dry).
            if (reserve >= this.target) {
                this.playing = true;
                this.fadeIn = FADE_FRAMES;
                this.smoothed = reserve;
            } else {
                left.fill(0);
                if (right !== left) right.fill(0);
                this.report(length);
                return true;
            }
        }

        // Dynamic rate control: steer the reserve towards the target.
        this.smoothed += (reserve - this.smoothed) * SMOOTHING;
        const error = (this.smoothed - this.target) / this.target;
        this.rate = this.base * (1 + Math.max(-MAX_RATE_ADJUSTMENT, Math.min(MAX_RATE_ADJUSTMENT, error * 0.05)));

        // Not enough for the whole block: play what there is, fading out.
        const needed = length * this.rate + 2;
        const available = reserve - 1;
        const frames = needed <= available ? length : Math.max(0, Math.floor((available - 1) / this.rate));
        const dry = frames < length;

        let position = this.position;
        for (let i = 0; i < frames; i++) {
            const base = Math.floor(position);
            const fraction = position - base;
            const a = (base & MASK) << 1;
            const b = ((base + 1) & MASK) << 1;
            let l = this.ring[a] + (this.ring[b] - this.ring[a]) * fraction;
            let r = this.ring[a + 1] + (this.ring[b + 1] - this.ring[a + 1]) * fraction;
            let gain = 1;
            if (this.fadeIn > 0) {
                gain = 1 - this.fadeIn / FADE_FRAMES;
                this.fadeIn--;
            }
            if (dry) gain *= (frames - i) / (frames + 1);
            left[i] = l * gain;
            right[i] = r * gain;
            position += this.rate;
        }
        this.position = position;
        for (let i = frames; i < length; i++) {
            left[i] = 0;
            right[i] = 0;
        }
        if (dry) {
            this.playing = false;
            // Running dry because the emulator was paused is not a dropout.
            if (this.active) {
                this.underruns++;
                this.calm = 0;
                // The emulator cannot keep up with a reserve this small: keep more.
                if (this.auto) this.target = Math.min(AUTO_MAX_S * SOURCE_RATE, Math.round(this.target * 1.4 + 0.01 * SOURCE_RATE));
            }
        } else if (this.auto) {
            this.calm += length / sampleRate;
            if (this.calm >= AUTO_CALM_S) {
                this.calm = 0;
                this.target = Math.max(AUTO_MIN_S * SOURCE_RATE, Math.round(this.target * 0.9));
            }
        }
        this.report(length);
        return true;
    }

    /** Tell the page how the reserve is doing a few times a second. */
    report(length) {
        this.sinceReport += length / sampleRate;
        if (this.sinceReport < REPORT_INTERVAL_S) return;
        this.sinceReport = 0;
        this.port.postMessage({
            buffered: ((this.written - this.position) / SOURCE_RATE) * 1000,
            target: (this.target / SOURCE_RATE) * 1000,
            auto: this.auto,
            underruns: this.underruns,
            overruns: this.overruns,
            rate: this.rate / this.base,
            playing: this.playing,
        });
    }
}

registerProcessor('ps1-audio', Ps1AudioProcessor);
