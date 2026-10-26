// Preferences that survive a reload, kept in localStorage.

const STORAGE_KEY = 'rs1emu.settings';

/** What the performance overlay can show; each entry is a switch of its own. */
export const OVERLAY_ITEMS = {
    fps: 'FPS',
    speed: 'Velocidad de emulación',
    frameTime: 'Tiempo por cuadro',
    load: 'Carga del procesador',
    resolution: 'Resolución y renderizado',
    audio: 'Búfer de audio',
    memory: 'Memoria',
};

export const OVERLAY_POSITIONS = [
    ['top-left', 'Arriba a la izquierda'],
    ['top-right', 'Arriba a la derecha'],
    ['bottom-left', 'Abajo a la izquierda'],
    ['bottom-right', 'Abajo a la derecha'],
];

const DEFAULTS = {
    volume: 80,
    muted: false,
    speed: '1',
    slot: 1,
    // Picture: '4:3', '16:9', 'fill', 'cover' or 'integer'; `crop` trims the black
    // border of the console's picture (0 to 3); `filter` is how it is scaled.
    aspect: '4:3',
    crop: 0,
    filter: 'nearest',
    // Audio: how much sound is kept in reserve, in milliseconds, or 'auto'.
    audioLatency: 'auto',
    // Rendering (see `RenderSettings` in the core).
    scale: 1,
    textureFilter: 'nearest',
    trueColor: false,
    pgxp: false,
    pgxpTextures: true,
    widescreen: false,
    smooth2d: false,
    overlay: {
        enabled: false,
        position: 'top-left',
        fps: true,
        speed: true,
        frameTime: false,
        load: false,
        resolution: false,
        audio: false,
        memory: false,
    },
};

const ASPECTS = ['4:3', '16:9', 'fill', 'cover', 'integer'];
const LATENCIES = ['auto', 40, 70, 120, 200];

function number(value, fallback, min, max) {
    const n = Number(value);
    return Number.isFinite(n) ? Math.max(min, Math.min(max, n)) : fallback;
}

/** Saved values with anything unknown or out of range replaced by the default. */
function sanitize(saved) {
    const merged = { ...DEFAULTS, ...saved, overlay: { ...DEFAULTS.overlay, ...(saved && saved.overlay) } };
    merged.volume = number(merged.volume, DEFAULTS.volume, 0, 100);
    merged.slot = number(merged.slot, DEFAULTS.slot, 1, 3);
    merged.crop = Math.round(number(merged.crop, 0, 0, 3));
    merged.scale = Math.round(number(merged.scale, 1, 1, 3));
    if (!ASPECTS.includes(merged.aspect)) merged.aspect = DEFAULTS.aspect;
    if (merged.audioLatency !== 'auto') merged.audioLatency = Number(merged.audioLatency);
    if (!LATENCIES.includes(merged.audioLatency)) merged.audioLatency = DEFAULTS.audioLatency;
    if (!['nearest', 'bilinear'].includes(merged.textureFilter)) merged.textureFilter = 'nearest';
    if (!OVERLAY_POSITIONS.some(([value]) => value === merged.overlay.position)) {
        merged.overlay.position = DEFAULTS.overlay.position;
    }
    return merged;
}

export function loadSettings() {
    try {
        const saved = JSON.parse(localStorage.getItem(STORAGE_KEY));
        if (saved && typeof saved === 'object') return sanitize(saved);
    } catch (_) { /* nothing saved, or storage unavailable */ }
    return sanitize(null);
}

export function saveSettings(settings) {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch (_) { /* the preferences last for this session only */ }
}

/** The options the worker hands to the core's renderer. */
export function renderOptions(settings) {
    return {
        scale: settings.scale,
        trueColor: settings.trueColor,
        bilinear: settings.textureFilter === 'bilinear',
        pgxp: settings.pgxp,
        pgxpTextures: settings.pgxp && settings.pgxpTextures,
        widescreen: settings.pgxp && settings.widescreen,
        smooth2d: settings.smooth2d && settings.scale > 1,
    };
}
