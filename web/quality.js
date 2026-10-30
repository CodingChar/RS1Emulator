// Adaptive quality: keeps the game at full speed on a machine that cannot afford everything
// the user switched on.
//
// The rendering options (internal resolution, bilinear textures, PGXP, 24-bit colour) cost
// time and change nothing about what the game does. When the frames take longer than the
// time they are allowed, the controller steps down a ladder of cheaper options, one rung at
// a time, and goes back up one rung when there has been plenty of room for a while. It only
// decides; the worker measures the frames and applies the options. It has no timers and does
// not touch the page, so it can be tested with a made-up clock.

/** The options with the next most expensive thing switched off, or `null` if nothing is left. */
export function cheaper(options) {
    // The bilinear filter is the dearest, then each step of resolution (4 times the pixels
    // for 2×, 9 times for 3×), then PGXP and the 24-bit colour.
    if (options.bilinear) return { ...options, bilinear: false };
    if (options.scale > 1) {
        const scale = options.scale - 1;
        return { ...options, scale, smooth2d: options.smooth2d && scale > 1 };
    }
    // Widescreen needs PGXP to squeeze the 3D geometry; without it the picture would only stretch.
    if (options.pgxp && !options.widescreen) {
        return { ...options, pgxp: false, pgxpTextures: false, widescreen: false };
    }
    if (options.trueColor) return { ...options, trueColor: false };
    return null;
}

/** The options the user chose, followed by each cheaper set down to the cheapest. */
export function qualityLadder(options) {
    const ladder = [{ ...options }];
    for (let next = cheaper(ladder[0]); next; next = cheaper(next)) ladder.push(next);
    return ladder;
}

/**
 * Load is the time a frame took (emulating and drawing it) over the time it is allowed:
 * 1 means the frame used all of its time. The window is how long the load is watched.
 */
const DEGRADE_WINDOW_MS = 2000;
/** A machine so slow that fewer frames than this fit in the window is judged on this many. */
const DEGRADE_MIN_SAMPLES = 8;
/** The average load that cannot be kept up: a game this close to its limit stutters. */
const DEGRADE_MEAN = 0.9;
/**
 * The share of frames over their time that cannot be kept up either, whatever the average.
 * A frame a little over its time costs nothing (the next ones are quicker and the loop makes
 * the time up), so this is only for a machine where half of the frames are late, as measured
 * in the browser at the edge of its speed (44-50 % late, average 0.9); at 0.65 there were 3 %.
 */
const DEGRADE_LATE_SHARE = 0.3;

const RESTORE_WINDOW_MS = 6000;
/** A machine with room to spare makes a frame every 17 ms: 6 s hold about 350. */
const RESTORE_MIN_SAMPLES = 150;
/** Going up a rung costs a quarter to a half more: only with a lot of room to spare. */
const RESTORE_MEAN = 0.5;
const RESTORE_BUSY = 0.8;
const RESTORE_BUSY_SHARE = 0.03;

/** After a change the first frames are not representative (the renderer was rebuilt). */
const SETTLE_MS = 2500;
/** The least time at a lower rung before trying the next one up; doubled each time that fails. */
const RESTORE_AFTER_MS = 15000;
const RESTORE_AFTER_MAX_MS = 120000;
/** Dropping again this soon after going up means the rung above does not fit. */
const BOUNCE_MS = 20000;
/** After this long without a change the machine gets the short waiting time back. */
const CALM_RESET_MS = 90000;
const EVALUATE_EVERY_MS = 250;

export class QualityController {
    constructor() {
        this.configure(null, false);
    }

    /**
     * Start from the options the user chose. `enabled` turns the controller on; `now` is the
     * time of the change, from the same clock as `sample` (the frames right after it count
     * for nothing).
     */
    configure(options, enabled, now = -Infinity) {
        this.ladder = options ? qualityLadder(options) : [];
        this.enabled = Boolean(options) && Boolean(enabled);
        this.level = 0;
        this.samples = [];
        this.changedAt = now;
        this.restoredAt = -Infinity;
        this.restoreAfter = RESTORE_AFTER_MS;
        this.evaluatedAt = -Infinity;
    }

    /** The options in force. */
    get options() {
        return this.ladder[this.level] || null;
    }

    /** How many rungs below the options the user chose (0: those options, as chosen). */
    get reduction() {
        return this.level;
    }

    /** Forget what was measured (the game was paused, or runs at another speed). */
    skip() {
        this.samples = [];
    }

    /**
     * Report the load of a frame at time `now` (milliseconds). Returns `null` while the
     * options stand, or `{ level, previous, options, reason }` when they should change:
     * `reason` is `'slow'` when going down and `'calm'` when going back up.
     */
    sample(load, now) {
        if (!this.enabled || this.ladder.length < 2 || !Number.isFinite(load)) return null;
        this.samples.push({ at: now, load });
        const oldest = now - RESTORE_WINDOW_MS;
        while (this.samples.length > 0 && this.samples[0].at < oldest) this.samples.shift();

        if (now - this.evaluatedAt < EVALUATE_EVERY_MS) return null;
        this.evaluatedAt = now;

        const sinceChange = now - this.changedAt;
        if (this.restoreAfter > RESTORE_AFTER_MS && sinceChange >= CALM_RESET_MS) this.restoreAfter = RESTORE_AFTER_MS;

        if (this.level < this.ladder.length - 1 && sinceChange >= SETTLE_MS) {
            const recent = this.within(now, DEGRADE_WINDOW_MS, DEGRADE_MIN_SAMPLES);
            if (recent.length >= DEGRADE_MIN_SAMPLES && (mean(recent) > DEGRADE_MEAN || share(recent, 1) > DEGRADE_LATE_SHARE)) {
                return this.move(1, now, 'slow');
            }
        }
        if (this.level > 0 && sinceChange >= this.restoreAfter) {
            const calm = this.within(now, RESTORE_WINDOW_MS);
            if (calm.length >= RESTORE_MIN_SAMPLES && mean(calm) < RESTORE_MEAN && share(calm, RESTORE_BUSY) <= RESTORE_BUSY_SHARE) {
                return this.move(-1, now, 'calm');
            }
        }
        return null;
    }

    /**
     * The loads of the last `span` milliseconds, or of the last `least` frames when fewer than
     * that fit in the span (a slow machine makes few frames, and those are the ones to judge).
     */
    within(now, span, least = 0) {
        const inSpan = this.samples.filter((sample) => sample.at >= now - span);
        return (inSpan.length >= least ? inSpan : this.samples.slice(-least)).map((sample) => sample.load);
    }

    move(step, now, reason) {
        const previous = this.level;
        this.level += step;
        if (step > 0) {
            // The rung above was tried a moment ago and did not hold: wait longer next time.
            if (now - this.restoredAt < BOUNCE_MS) this.restoreAfter = Math.min(this.restoreAfter * 2, RESTORE_AFTER_MAX_MS);
        } else {
            this.restoredAt = now;
        }
        this.changedAt = now;
        this.samples = [];
        return { level: this.level, previous, options: this.options, reason };
    }
}

function mean(values) {
    return values.reduce((sum, value) => sum + value, 0) / values.length;
}

/** The share of `values` above `limit`. */
function share(values, limit) {
    return values.filter((value) => value > limit).length / values.length;
}
