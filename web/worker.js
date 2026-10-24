// The emulator, running in a worker so that it never competes with the page
// for the main thread and so it can read the disc straight from disk.
//
// Talks to app.js with messages; see `onmessage` for the commands and
// `post()` calls for what it reports back.

import init, { Ps1 } from './pkg/ps1_wasm.js';
import * as storage from './storage.js';

const BIOS_KEY = 'bios';
const CARD_COUNT = 2;
const CARD_SIZE = 128 * 1024;
/// Never run more than this many frames to catch up after a stall.
const MAX_CATCH_UP_FRAMES = 4;
/// In "maximum speed" mode, the time spent emulating before yielding.
const TURBO_BUDGET_MS = 12;
const CARD_SAVE_INTERVAL_MS = 1000;
/// Disc sectors are read from the file in pieces this big.
const DISC_READ_CHUNK = 256 * 1024;
/// The loop wakes up this often to see whether a frame is due.
const TICK_MS = 2;

let wasm = null;
let ps1 = null;
let canvas = null; // an OffscreenCanvas, or null when frames are sent to the page
let context = null;
let audioPort = null;

let biosLoaded = false;
let discLoaded = false;
let running = false;
let paused = false;
let speed = 1;
let inputMask = 0;

let lastFrameTime = 0;
let accumulator = 0;
let lastCardCheck = 0;
const stats = { frames: 0, since: 0 };

function post(message, transfer = []) {
    postMessage(message, transfer);
}

function say(text, isError = false) {
    post({ type: 'status', text, isError });
}

function publishState() {
    post({ type: 'state', biosLoaded, discLoaded, running, paused });
}

// ----- BIOS -----

function useBios(bytes, name) {
    try {
        ps1.load_bios(bytes);
    } catch (error) {
        say(`BIOS no válida: ${error}`, true);
        return false;
    }
    biosLoaded = true;
    post({ type: 'bios', name, size: bytes.length });
    publishState();
    return true;
}

async function loadBios(file) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (useBios(bytes, file.name)) {
        await storage.put(BIOS_KEY, bytes);
        say('BIOS lista. Elige un disco o pulsa Iniciar para abrir el menú de la BIOS.');
    }
}

// ----- Disc -----

/// A function that returns `length` bytes of `file` from `offset`, reading
/// the file synchronously a chunk at a time (this only works in a worker).
function makeReader(file) {
    const reader = new FileReaderSync();
    let start = -1;
    let chunk = null;
    return (offset, length) => {
        const end = offset + length;
        if (chunk === null || offset < start || end > start + chunk.length) {
            start = Math.floor(offset / DISC_READ_CHUNK) * DISC_READ_CHUNK;
            const stop = Math.min(file.size, Math.max(start + DISC_READ_CHUNK, end));
            chunk = new Uint8Array(reader.readAsArrayBuffer(file.slice(start, stop)));
        }
        return chunk.subarray(offset - start, offset - start + length);
    };
}

function loadDisc(files) {
    const cue = files.find((f) => /\.cue$/i.test(f.name));
    const data = files.filter((f) => !/\.cue$/i.test(f.name));
    if (data.length === 0) {
        say('Falta el archivo .bin (o .iso) del disco.', true);
        return;
    }
    try {
        for (const file of data) {
            ps1.add_lazy_disc_file(file.name, file.size, makeReader(file));
        }
        ps1.insert_disc_files(cue ? new FileReaderSync().readAsText(cue) : undefined);
    } catch (error) {
        ps1.discard_disc_files();
        say(`No se pudo cargar el disco: ${error}`, true);
        return;
    }
    discLoaded = true;
    const total = data.reduce((sum, f) => sum + f.size, 0);
    post({ type: 'disc', names: (cue ? [cue] : data).map((f) => f.name), size: total });
    publishState();
}

// ----- Running -----

function startConsole() {
    if (!ps1 || !biosLoaded) return;
    try {
        ps1.reset();
    } catch (error) {
        say(`Error al arrancar: ${error}`, true);
        return;
    }
    paused = false;
    running = true;
    accumulator = 0;
    lastFrameTime = performance.now();
    say(discLoaded ? 'En marcha.' : 'En marcha (sin disco: menú de la BIOS).');
    publishState();
}

function stop(error) {
    running = false;
    paused = false;
    say(`La emulación se detuvo: ${error}`, true);
    publishState();
}

function present() {
    ps1.render_frame();
    const width = ps1.framebuffer_width();
    const height = ps1.framebuffer_height();
    // A view of the frame inside the module's memory: no copy until it is drawn.
    const pixels = new Uint8ClampedArray(wasm.memory.buffer, ps1.frame_ptr(), ps1.frame_len());
    if (context) {
        if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width;
            canvas.height = height;
        }
        context.putImageData(new ImageData(pixels, width, height), 0, 0);
    } else {
        // No OffscreenCanvas: the page draws it.
        const copy = pixels.slice().buffer;
        post({ type: 'frame', width, height, pixels: copy }, [copy]);
    }
}

function updateStats(now, frames) {
    stats.frames += frames;
    if (now - stats.since >= 1000) {
        const fps = (stats.frames * 1000) / (now - stats.since);
        post({ type: 'stats', fps, full: ps1.frames_per_second() });
        stats.frames = 0;
        stats.since = now;
    }
}

async function persistCards(now, force = false) {
    if (!force && now - lastCardCheck < CARD_SAVE_INTERVAL_MS) return;
    lastCardCheck = now;
    for (let port = 0; port < CARD_COUNT; port++) {
        if (ps1.memory_card_dirty(port)) {
            const data = ps1.memory_card_data(port);
            ps1.clear_memory_card_dirty(port);
            if (data) await storage.put(`card${port}`, data);
        }
    }
}

function tick() {
    setTimeout(tick, TICK_MS);
    if (!running) {
        lastFrameTime = performance.now();
        return;
    }
    try {
        const now = performance.now();
        ps1.set_input(0, inputMask, 0, 0, 0, 0);
        let frames = 0;
        if (speed === 'max') {
            do {
                ps1.run_frame();
                frames++;
            } while (performance.now() - now < TURBO_BUDGET_MS);
        } else {
            accumulator += Math.min(now - lastFrameTime, 100) * speed;
            const frameTime = 1000 / ps1.frames_per_second();
            frames = Math.min(Math.floor(accumulator / frameTime), MAX_CATCH_UP_FRAMES);
            accumulator -= frames * frameTime;
            // Too far behind (a slow machine or a stalled tab): give up the backlog.
            if (accumulator > frameTime * MAX_CATCH_UP_FRAMES) accumulator = 0;
            for (let i = 0; i < frames; i++) ps1.run_frame();
        }
        lastFrameTime = now;
        if (frames > 0) {
            present();
            const samples = ps1.drain_audio();
            // Audio only makes sense at normal speed.
            if (audioPort && speed === 1 && samples.length > 0) {
                audioPort.postMessage(samples, [samples.buffer]);
            }
        }
        updateStats(now, frames);
        persistCards(now);
    } catch (error) {
        stop(error);
    }
}

// ----- Memory cards and states -----

async function restoreCards() {
    for (let port = 0; port < CARD_COUNT; port++) {
        const saved = await storage.get(`card${port}`);
        if (saved && saved.length === CARD_SIZE) {
            ps1.insert_memory_card(port, saved);
        } else {
            ps1.insert_formatted_memory_card(port);
            await storage.put(`card${port}`, ps1.memory_card_data(port));
        }
        ps1.clear_memory_card_dirty(port);
    }
}

async function cardCommand(message) {
    const { port, op } = message;
    if (op === 'export') {
        const data = ps1.memory_card_data(port);
        if (data) post({ type: 'card-export', port, data }, [data.buffer]);
    } else if (op === 'import') {
        const bytes = new Uint8Array(message.bytes);
        if (bytes.length !== CARD_SIZE) {
            say(`Una tarjeta de memoria ocupa 128 KB y ese archivo ocupa ${Math.round(bytes.length / 1024)} KB.`, true);
            return;
        }
        ps1.insert_memory_card(port, bytes);
        await storage.put(`card${port}`, bytes);
        say(`Tarjeta ${port + 1} importada.`);
    } else if (op === 'format') {
        ps1.insert_formatted_memory_card(port);
        await storage.put(`card${port}`, ps1.memory_card_data(port));
        say(`Tarjeta ${port + 1} formateada.`);
    }
}

async function saveState(slot) {
    try {
        await storage.put(`state${slot}`, ps1.save_state());
        say(`Estado guardado en la ranura ${slot}.`);
    } catch (error) {
        say(`No se pudo guardar el estado: ${error}`, true);
    }
}

async function loadState(slot) {
    const bytes = await storage.get(`state${slot}`);
    if (!bytes) {
        say(`La ranura ${slot} está vacía.`, true);
        return;
    }
    try {
        ps1.load_state(bytes);
        say(`Estado de la ranura ${slot} cargado.`);
    } catch (error) {
        say(`No se pudo cargar el estado: ${error}`, true);
    }
}

// ----- Messages from the page -----

async function start(message) {
    try {
        wasm = await init();
    } catch (error) {
        say(`No se pudo cargar el emulador (¿falta compilar web/pkg?): ${error}`, true);
        return;
    }
    ps1 = new Ps1();
    if (message.canvas) {
        canvas = message.canvas;
        context = canvas.getContext('2d');
    }
    await restoreCards();
    post({ type: 'ready', persistent: await storage.isPersistent() });

    const saved = await storage.get(BIOS_KEY);
    if (saved && useBios(saved, 'guardada en este navegador')) {
        say('BIOS recuperada. Elige un disco o pulsa Iniciar para abrir el menú de la BIOS.');
    } else {
        say('Elige tu BIOS para empezar.');
    }
    publishState();
    stats.since = performance.now();
    tick();
}

onmessage = async (event) => {
    const message = event.data;
    if (message.type === 'start-worker') return start(message);
    if (!ps1) return;
    switch (message.type) {
        case 'bios': return loadBios(message.file);
        case 'forget-bios': return storage.remove(BIOS_KEY);
        case 'disc':
            running = false;
            loadDisc(message.files);
            if (discLoaded && biosLoaded) startConsole();
            else publishState();
            return;
        case 'start': return startConsole();
        case 'pause':
            paused = true;
            running = false;
            say('En pausa.');
            return publishState();
        case 'resume':
            paused = false;
            running = true;
            lastFrameTime = performance.now();
            accumulator = 0;
            say('En marcha.');
            return publishState();
        case 'speed': speed = message.value; return;
        case 'input': inputMask = message.mask; return;
        case 'audio-port': audioPort = message.port; return;
        case 'card': return cardCommand(message);
        case 'save-state': return saveState(message.slot);
        case 'load-state': return loadState(message.slot);
        case 'flush-cards': return persistCards(performance.now(), true);
    }
};
