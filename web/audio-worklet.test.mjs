// Tests of the audio player's logic (reserve, rate control, fades), run without a
// browser by giving the worklet file the few globals it expects.
// Run with `node --test web/audio-worklet.test.mjs`.
import assert from 'node:assert/strict';
import test from 'node:test';

const RATE = 44100;
const BLOCK = 128;

let ProcessorClass = null;
globalThis.sampleRate = RATE;
globalThis.AudioWorkletProcessor = class {
    constructor() {
        this.port = { posted: [], onmessage: null, postMessage(message) { this.posted.push(message); } };
    }
};
globalThis.registerProcessor = (_name, cls) => {
    ProcessorClass = cls;
};
await import('./audio-worklet.js');

/** A processor, a sine source that keeps its phase, and a clock that runs both. */
function rig({ producerRate = 1, jitter = 0, active = true, target, contextRate = RATE } = {}) {
    globalThis.sampleRate = contextRate;
    const processor = new ProcessorClass();
    if (target !== undefined) processor.port.onmessage({ data: { target } });
    if (!active) processor.port.onmessage({ data: { active: false } });
    let phase = 0;
    const sine = (frames) => {
        const out = new Float32Array(frames * 2);
        for (let i = 0; i < frames; i++, phase++) {
            const value = 0.5 * Math.sin((2 * Math.PI * 440 * phase) / RATE);
            out[2 * i] = value;
            out[2 * i + 1] = value;
        }
        return out;
    };
    let seed = 12345;
    const random = () => {
        seed = (seed * 1103515245 + 12345) & 0x7fffffff;
        return seed / 0x7fffffff;
    };
    const left = new Float32Array(BLOCK);
    const right = new Float32Array(BLOCK);
    const output = [];
    let produced = 0; // frames the emulator has made
    let owed = 0; // frames due from the emulator
    const state = { stalled: false, maxJump: 0, last: 0, jumps: [] };

    /** Run `seconds` of audio-clock time: the page consumes blocks while the emulator makes frames. */
    function run(seconds) {
        const blocks = Math.round((seconds * contextRate) / BLOCK);
        for (let b = 0; b < blocks; b++) {
            // The emulator makes 60 frames a second, a little early or late.
            owed += (BLOCK / contextRate) * 60 * producerRate;
            while (owed >= 1 && !state.stalled) {
                owed -= 1;
                const frames = 735 + (jitter ? Math.round((random() - 0.5) * jitter) : 0);
                processor.receive(sine(frames));
                produced += frames;
            }
            if (state.stalled) owed = 0;
            processor.process([], [[left, right]]);
            for (let i = 0; i < BLOCK; i++) {
                const jump = Math.abs(left[i] - state.last);
                if (jump > state.maxJump) state.maxJump = jump;
                if (jump > 0.06) state.jumps.push(jump);
                state.last = left[i];
                if (state.recorded) state.recorded.push(left[i]);
            }
            output.push(left[0]);
        }
    }
    return { processor, run, state, produced: () => produced, output };
}

test('a steady stream plays without dropouts and the reserve settles near the target', () => {
    const { processor, run, state } = rig({ target: 70 });
    run(6);
    const dropouts = processor.underruns;
    state.maxJump = 0;
    run(10);
    assert.equal(processor.underruns, dropouts, 'no dropouts once settled');
    assert.equal(processor.overruns, 0);
    const reserve = (processor.written - processor.position) / RATE * 1000;
    assert.ok(reserve > 35 && reserve < 110, `reserve ${reserve.toFixed(0)} ms`);
    // A 440 Hz sine of amplitude 0.5 moves by at most 0.032 per sample; rate control adds 1 %.
    assert.ok(state.maxJump < 0.04, `jump ${state.maxJump}`);
});

test('a producer slightly slower than the sound card (clock drift) does not run dry', () => {
    const { processor, run } = rig({ producerRate: 0.997, jitter: 120 });
    run(8);
    const dropouts = processor.underruns;
    run(40);
    assert.equal(processor.underruns, dropouts, 'rate control slows the playback instead');
});

test('a producer slightly faster than the sound card does not overflow', () => {
    const { processor, run } = rig({ producerRate: 1.003, jitter: 120 });
    run(8);
    const skips = processor.overruns;
    run(40);
    assert.equal(processor.overruns, skips, 'rate control speeds the playback up instead');
    const reserve = (processor.written - processor.position) / RATE * 1000;
    assert.ok(reserve < 200, `reserve ${reserve.toFixed(0)} ms`);
});

test('a stall fades out instead of cutting, waits for the reserve and comes back', () => {
    const { processor, run, state } = rig({ target: 70 });
    run(4);
    const before = processor.underruns;
    state.stalled = true;
    state.jumps.length = 0;
    run(0.5);
    assert.equal(processor.underruns, before + 1, 'exactly one dropout');
    assert.equal(processor.playing, false);
    assert.equal(state.jumps.length, 0, 'the fade has no click');
    state.stalled = false;
    run(1);
    assert.equal(processor.playing, true, 'playing again');
    assert.equal(state.jumps.length, 0, 'and the fade in has no click either');
});

test('in automatic mode the reserve grows after a dropout and shrinks again when all is well', () => {
    const { processor, run, state } = rig();
    run(3);
    const start = processor.target;
    state.stalled = true;
    run(0.4);
    state.stalled = false;
    run(1);
    assert.ok(processor.target > start, `${processor.target} <= ${start}`);
    const grown = processor.target;
    run(40);
    assert.ok(processor.target < grown, 'it comes back down while nothing goes wrong');
    assert.ok(processor.target >= 0.05 * RATE, 'but not below the minimum');
});

test('a fixed reserve does not move after a dropout', () => {
    const { processor, run, state } = rig({ target: 70 });
    run(3);
    const target = processor.target;
    state.stalled = true;
    run(0.4);
    state.stalled = false;
    run(2);
    assert.equal(processor.target, target);
});

test('running dry while the emulator is paused is not a dropout', () => {
    const { processor, run, state } = rig({ target: 70 });
    run(3);
    processor.port.onmessage({ data: { active: false } });
    const before = processor.underruns;
    state.stalled = true;
    run(1);
    assert.equal(processor.underruns, before, 'not counted');
    assert.equal(processor.playing, false);
});

test('the reserve is reported to the page a few times a second', () => {
    const { processor, run } = rig({ target: 70 });
    run(2);
    const reports = processor.port.posted;
    assert.ok(reports.length >= 6 && reports.length <= 9, `${reports.length} reports in 2 s`);
    const last = reports.at(-1);
    assert.equal(last.auto, false);
    assert.ok(last.playing && last.buffered > 0 && last.target > 60 && last.target < 80, JSON.stringify(last));
});

test('a sound card at another rate gets the sound resampled, with the pitch intact', () => {
    for (const contextRate of [48000, 22050]) {
        const { run, state } = rig({ target: 70, contextRate });
        run(2);
        state.recorded = [];
        run(2);
        // Rising zero crossings of the 440 Hz sine over the 2 seconds recorded.
        let crossings = 0;
        for (let i = 1; i < state.recorded.length; i++) {
            if (state.recorded[i - 1] < 0 && state.recorded[i] >= 0) crossings++;
        }
        assert.ok(Math.abs(crossings / 2 - 440) < 8, `${contextRate} Hz card: ${crossings / 2} Hz`);
    }
});
