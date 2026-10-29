// Run with `node --test web/`.
import assert from 'node:assert/strict';
import test from 'node:test';

import { computeLayout } from './layout.js';
import { loadSettings, renderOptions } from './settings.js';

const near = (a, b, message) => assert.ok(Math.abs(a - b) < 0.01, `${message}: ${a} vs ${b}`);

test('4:3 is centred with bars at the sides of a wide window', () => {
    const { box, image } = computeLayout({ width: 1600, height: 900, aspect: '4:3' });
    near(box.h, 900, 'full height');
    near(box.w, 1200, '4:3 width');
    near(box.x, 200, 'bar at the left');
    near(image.w, box.w, 'no crop: the image is the box');
});

test('16:9 fills a 16:9 window and bars the top of a taller one', () => {
    const wide = computeLayout({ width: 1920, height: 1080, aspect: '16:9' });
    near(wide.box.w, 1920, 'width');
    near(wide.box.h, 1080, 'height');
    const tall = computeLayout({ width: 1000, height: 1000, aspect: '16:9' });
    near(tall.box.w, 1000, 'width');
    near(tall.box.h, 562.5, 'height');
    near(tall.box.y, 218.75, 'bar above');
});

test('fill stretches to the window whatever its shape', () => {
    const { box } = computeLayout({ width: 1000, height: 300, aspect: 'fill' });
    assert.deepEqual([box.w, box.h, box.x, box.y], [1000, 300, 0, 0]);
});

test('cover keeps the 4:3 proportions and crops what does not fit', () => {
    const { box, image } = computeLayout({ width: 1920, height: 1080, aspect: 'cover' });
    assert.deepEqual([box.w, box.h], [1920, 1080]);
    near(image.w, 1920, 'as wide as the window');
    near(image.h, 1440, '4:3 of that');
    near(image.y, -180, 'centred, a third of the overflow above');
    near(image.w / image.h, 4 / 3, 'not deformed');
    // A window taller than 4:3 is covered by height instead.
    const tall = computeLayout({ width: 600, height: 1000, aspect: 'cover' });
    near(tall.image.h, 1000, 'as tall as the window');
    near(tall.image.w, 1333.33, '4:3 of that');
});

test('cropping the border enlarges the picture without changing its proportions', () => {
    const plain = computeLayout({ width: 1200, height: 900, aspect: '4:3' });
    const cropped = computeLayout({ width: 1200, height: 900, aspect: '4:3', crop: 2 });
    near(cropped.box.w, plain.box.w, 'the box does not change');
    near(cropped.image.w, 1200 / 0.92, 'the image is larger');
    near(cropped.image.w / cropped.image.h, 4 / 3, 'same proportions');
    // The image is centred on the box, so the same amount is cut from every side.
    near(cropped.image.x, (cropped.box.w - cropped.image.w) / 2, 'centred');
});

test('whole-number scale uses a multiple of the console lines', () => {
    const { box } = computeLayout({ width: 1920, height: 1080, aspect: 'integer', lines: 240 });
    near(box.h, 960, 'four times 240 lines');
    near(box.w, 1280, '4:3 of that');
    // With a device pixel ratio of 2 the same window holds twice as many device pixels.
    const dense = computeLayout({ width: 1920, height: 1080, aspect: 'integer', lines: 240, ratio: 2 });
    near(dense.box.h, 1080, 'nine device pixels per line fill the window');
    // Too small for one line per pixel: falls back to fitting.
    const small = computeLayout({ width: 300, height: 200, aspect: 'integer', lines: 480 });
    near(small.box.h, 200, 'fits');
});

test('saved settings are cleaned up when loaded', () => {
    // No localStorage in Node: the defaults come back, and the options derive from them.
    const settings = loadSettings();
    assert.equal(settings.aspect, '4:3');
    assert.equal(settings.scale, 1);
    assert.equal(settings.audioLatency, 'auto');
    assert.deepEqual(renderOptions(settings), { scale: 1, trueColor: false, bilinear: false, pgxp: false, pgxpTextures: false, widescreen: false, smooth2d: false });
});

test('perspective textures and widescreen only count with PGXP', () => {
    const options = renderOptions({ scale: 2, trueColor: true, textureFilter: 'bilinear', pgxp: false, pgxpTextures: true, widescreen: true });
    assert.deepEqual(options, { scale: 2, trueColor: true, bilinear: true, pgxp: false, pgxpTextures: false, widescreen: false, smooth2d: false });
    const withPgxp = renderOptions({ scale: 3, trueColor: false, textureFilter: 'nearest', pgxp: true, pgxpTextures: true, widescreen: true });
    assert.deepEqual(withPgxp, { scale: 3, trueColor: false, bilinear: false, pgxp: true, pgxpTextures: true, widescreen: true, smooth2d: false });
});

test('2D smoothing only counts when the picture is scaled', () => {
    const base = { trueColor: false, textureFilter: 'nearest', pgxp: false, pgxpTextures: false, widescreen: false };
    assert.equal(renderOptions({ ...base, scale: 1, smooth2d: true }).smooth2d, false);
    assert.equal(renderOptions({ ...base, scale: 2, smooth2d: true }).smooth2d, true);
    assert.equal(renderOptions({ ...base, scale: 2, smooth2d: false }).smooth2d, false);
});
