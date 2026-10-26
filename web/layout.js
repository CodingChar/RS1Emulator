// Where the console's picture goes inside the window. Pure arithmetic, so it can be
// tested without a browser (see layout.test.mjs).

/** The picture of the console is 4:3 whatever its size in pixels. */
export const CONSOLE_ASPECT = 4 / 3;

/** Proportion of the picture cut from every side, for each level of "Recortar el borde negro". */
export const CROP_FRACTIONS = [0, 0.02, 0.04, 0.07];

/**
 * Size and place of the picture for the aspect option.
 *
 * - `aspect`: `'4:3'`, `'16:9'`, `'fill'`, `'cover'` or `'integer'`;
 * - `crop`: level 0 to 3 of the border trimmed from every side;
 * - `lines`: lines of the console's display (240 or 480), for the whole-number scale;
 * - `ratio`: device pixels per CSS pixel.
 *
 * Returns the `box` that shows the picture (a rectangle in the stage) and the `image`,
 * the whole picture, which is larger than the box when the border is cropped or the
 * picture covers the window; both are `{ x, y, w, h }` in CSS pixels, the image
 * relative to the box.
 */
export function computeLayout({ width, height, aspect, crop = 0, lines = 240, ratio = 1 }) {
    const zoom = 1 / (1 - 2 * (CROP_FRACTIONS[crop] || 0));
    const shape = aspect === '16:9' ? 16 / 9 : CONSOLE_ASPECT;

    // The largest rectangle of the given proportions inside the stage.
    const contain = (proportion) => (width / height > proportion ? { w: height * proportion, h: height } : { w: width, h: width / proportion });
    let box;
    let size;
    switch (aspect) {
        case 'fill':
            box = { w: width, h: height };
            size = { w: width, h: height };
            break;
        case 'cover': {
            // The 4:3 picture, as large as it takes to cover the whole window.
            box = { w: width, h: height };
            const w = Math.max(width, height * shape);
            size = { w, h: w / shape };
            break;
        }
        case 'integer': {
            // A whole number of device pixels for every line of the console's display.
            const k = Math.floor(Math.min((height * ratio) / lines, (width * ratio) / (lines * shape)));
            box = k < 1 ? contain(shape) : { w: (lines * k * shape) / ratio, h: (lines * k) / ratio };
            size = box;
            break;
        }
        default:
            box = contain(shape);
            size = box;
    }
    const image = { w: size.w * zoom, h: size.h * zoom };
    return {
        box: { x: (width - box.w) / 2, y: (height - box.h) / 2, w: box.w, h: box.h },
        image: { x: (box.w - image.w) / 2, y: (box.h - image.h) / 2, w: image.w, h: image.h },
    };
}
