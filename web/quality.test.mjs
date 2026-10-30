// Run with: node --test web/quality.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { QualityController, cheaper, qualityLadder } from './quality.js';

const FULL = { scale: 3, trueColor: true, bilinear: true, pgxp: true, pgxpTextures: true, widescreen: false, smooth2d: true };
const PLAIN = { scale: 1, trueColor: false, bilinear: false, pgxp: false, pgxpTextures: false, widescreen: false, smooth2d: false };

/** A short name for a set of options: scale, then b(ilinear), p(gxp), w(idescreen), t(rue colour). */
function name(o) {
    return `${o.scale}${o.bilinear ? 'b' : ''}${o.pgxp ? 'p' : ''}${o.widescreen ? 'w' : ''}${o.trueColor ? 't' : ''}`;
}

/**
 * Report `load` (a number or a function of the time) every `step` ms from `from`; returns the
 * changes, each with the time it happened at. With `stop` the clock stops at the first change,
 * so the test can carry on from there.
 */
function feed(controller, from, ms, load, { step = 16, stop = false } = {}) {
    const changes = [];
    for (let t = from; t < from + ms; t += step) {
        const change = controller.sample(typeof load === 'function' ? load(t) : load, t);
        if (change) {
            changes.push({ ...change, at: t });
            if (stop) break;
        }
    }
    return changes;
}

function controllerFor(options, enabled = true, now = 0) {
    const controller = new QualityController();
    controller.configure(options, enabled, now);
    return controller;
}

// ----- The ladder -----

test('the ladder drops the dearest thing first and ends with the console as it is', () => {
    const ladder = qualityLadder(FULL);
    assert.deepEqual(ladder.map(name), ['3bpt', '3pt', '2pt', '1pt', '1t', '1']);
    assert.deepEqual(ladder[0], FULL, 'the first rung is what the user chose');
    assert.deepEqual(ladder.at(-1), PLAIN);
});

test('options that cost nothing have a ladder of one rung', () => {
    assert.equal(qualityLadder(PLAIN).length, 1);
    assert.equal(cheaper(PLAIN), null);
});

test('smoothing the 2D picture only makes sense above 1x and goes with the resolution', () => {
    const ladder = qualityLadder({ ...PLAIN, scale: 2, smooth2d: true });
    assert.deepEqual(ladder.map((o) => [o.scale, o.smooth2d]), [[2, true], [1, false]]);
});

test('widescreen keeps PGXP, which it needs to squeeze the geometry, to the end', () => {
    const ladder = qualityLadder({ ...FULL, scale: 2, widescreen: true });
    assert.deepEqual(ladder.map(name), ['2bpwt', '2pwt', '1pwt', '1pw']);
    assert.ok(ladder.every((o) => o.pgxp && o.widescreen));
});

test('PGXP takes its perspective textures with it', () => {
    const rung = qualityLadder({ ...PLAIN, pgxp: true, pgxpTextures: true }).at(-1);
    assert.equal(rung.pgxp, false);
    assert.equal(rung.pgxpTextures, false);
});

test('building the ladder leaves the options it was given alone', () => {
    const options = { ...FULL };
    qualityLadder(options);
    assert.deepEqual(options, FULL);
});

// ----- Going down -----

test('a machine with room to spare keeps what the user chose', () => {
    const controller = controllerFor(FULL);
    assert.deepEqual(feed(controller, 0, 30000, 0.5), []);
    assert.equal(controller.reduction, 0);
    assert.deepEqual(controller.options, FULL);
});

test('frames over their time make it step down, once the options have settled', () => {
    const controller = controllerFor(FULL);
    const [first, ...rest] = feed(controller, 0, 3500, 1.3);
    assert.equal(first.reason, 'slow');
    assert.equal(first.level, 1);
    assert.equal(name(first.options), '3pt');
    assert.ok(first.at >= 2500, `waited for the settling time (${first.at})`);
    assert.deepEqual(rest, [], 'one rung at a time');
});

test('it keeps stepping down while the machine cannot keep up, and stops at the bottom', () => {
    const controller = controllerFor(FULL);
    const changes = feed(controller, 0, 60000, 1.5);
    assert.deepEqual(changes.map((c) => name(c.options)), ['3pt', '2pt', '1pt', '1t', '1']);
    for (let i = 1; i < changes.length; i++) assert.ok(changes[i].at - changes[i - 1].at >= 2500);
    assert.equal(controller.reduction, 5);
});

test('a machine so slow that it makes a few frames a second is judged on the few it makes', () => {
    // Four frames a second, each taking three times its time: 8 frames fit in the 2 s window.
    const controller = controllerFor(FULL);
    const changes = feed(controller, 0, 12000, 3, { step: 250 });
    assert.equal(changes[0]?.reason, 'slow');
    assert.ok(changes[0].at <= 3500, `${changes[0].at} ms`);
});

test('a single frame is not enough to decide', () => {
    const controller = controllerFor(FULL, true, 0);
    assert.equal(controller.sample(5, 3000), null);
});

test('a short stall does not count', () => {
    const controller = controllerFor(FULL);
    const changes = feed(controller, 0, 20000, (t) => (t >= 8000 && t < 8100 ? 3 : 0.5));
    assert.deepEqual(changes, []);
});

test('an average too close to the limit steps down, one with some room does not', () => {
    assert.equal(feed(controllerFor(FULL), 0, 6000, 0.95).length > 0, true);
    assert.deepEqual(feed(controllerFor(FULL), 0, 20000, 0.85), []);
});

test('a lot of late frames step down even when the average looks fine', () => {
    const controller = controllerFor(FULL);
    // Half of the frames take 1.2 times their time, the rest 0.4: an average of 0.8.
    const changes = feed(controller, 0, 6000, (t) => (Math.floor(t / 16) % 2 ? 1.2 : 0.4));
    assert.equal(changes[0]?.reason, 'slow');
});

test('frames a little over their time, with room in the rest, are left alone', () => {
    // As seen in the browser: a quarter of the frames at 1.1 and the rest at 0.5 (an average of 0.65).
    const controller = controllerFor(FULL);
    assert.deepEqual(feed(controller, 0, 30000, (t) => (Math.floor(t / 16) % 4 === 0 ? 1.1 : 0.5)), []);
});

test('it does nothing when it is off, or when there is nothing to lower', () => {
    assert.deepEqual(feed(controllerFor(FULL, false), 0, 20000, 2), []);
    assert.deepEqual(feed(controllerFor(PLAIN), 0, 20000, 2), []);
    assert.deepEqual(feed(new QualityController(), 0, 20000, 2), []);
});

test('nonsense loads are ignored', () => {
    const controller = controllerFor(FULL);
    assert.deepEqual(feed(controller, 0, 10000, NaN), []);
    assert.deepEqual(feed(controller, 10000, 10000, Infinity), []);
});

test('skipping forgets what was measured', () => {
    const controller = controllerFor(FULL);
    // Two seconds of overload, forgotten, then calm: nothing was ever decided.
    feed(controller, 0, 2400, 1.5);
    controller.skip();
    assert.deepEqual(feed(controller, 2400, 10000, 0.5), []);
});

test('the frames right after configuring do not count', () => {
    const controller = controllerFor(FULL, true, 5000);
    const [first] = feed(controller, 5000, 6000, 1.5);
    assert.ok(first.at >= 5000 + 2500);
});

// ----- Coming back up -----

/** Overload until the first step down; returns the time of it. */
function stepDown(controller, from = 0) {
    const [change] = feed(controller, from, 8000, 1.5, { stop: true });
    assert.ok(change && change.reason === 'slow');
    return change.at;
}

/** Calm until the first step up; returns the change. */
function stepUp(controller, from, ms = 200000) {
    const [change] = feed(controller, from, ms, 0.3, { stop: true });
    assert.ok(change && change.reason === 'calm', 'it went back up');
    return change;
}

test('after plenty of room for a while it goes back up one rung', () => {
    const controller = controllerFor(FULL);
    const down = stepDown(controller);
    const changes = feed(controller, down + 16, 40000, 0.3);
    assert.equal(changes.length, 1);
    assert.equal(changes[0].reason, 'calm');
    assert.equal(changes[0].level, 0);
    assert.deepEqual(changes[0].options, FULL);
    assert.ok(changes[0].at - down >= 15000, `not before 15 s (${changes[0].at - down} ms)`);
});

test('a machine that is neither struggling nor roomy stays where it is', () => {
    const controller = controllerFor(FULL);
    const down = stepDown(controller);
    assert.deepEqual(feed(controller, down + 16, 120000, 0.7), []);
    assert.equal(controller.reduction, 1);
});

test('one busy moment is enough to wait longer before going up', () => {
    const controller = controllerFor(FULL);
    const down = stepDown(controller);
    // Every tenth frame at 0.9: the average is low but the busy ones are too many.
    const changes = feed(controller, down + 16, 60000, (t) => (Math.floor(t / 16) % 10 === 0 ? 0.9 : 0.3));
    assert.deepEqual(changes, []);
});

test('going back up one rung at a time until the user\'s options are back', () => {
    const controller = controllerFor(FULL);
    let now = 0;
    for (let i = 0; i < 3; i++) now = stepDown(controller, now + 16);
    assert.equal(controller.reduction, 3);
    const changes = feed(controller, now + 16, 200000, 0.2);
    assert.deepEqual(changes.map((c) => c.level), [2, 1, 0]);
    assert.deepEqual(controller.options, FULL);
});

test('dropping again soon after going up makes the next try wait twice as long', () => {
    const controller = controllerFor(FULL);
    const down = stepDown(controller);
    const up = stepUp(controller, down + 16, 40000);
    assert.equal(up.level, 0);
    // The rung above does not fit.
    const again = stepDown(controller, up.at + 16);
    assert.ok(again - up.at < 20000);
    const second = stepUp(controller, again + 16, 80000);
    assert.ok(second.at - again >= 30000, `waited twice as long (${second.at - again} ms)`);
    assert.ok(second.at - again < 40000);
});

test('the wait to go back up never grows past two minutes, and starts again after a long calm', () => {
    const controller = controllerFor(FULL);
    let now = stepDown(controller);
    for (let i = 0; i < 6; i++) {
        const up = stepUp(controller, now + 16);
        now = stepDown(controller, up.at + 16);
    }
    assert.equal(controller.restoreAfter, 120000);
    // Ninety quiet seconds at a lower rung bring the short wait back.
    feed(controller, now + 16, 95000, 0.7);
    assert.equal(controller.restoreAfter, 15000);
});
