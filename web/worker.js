// The emulator, running in a worker so that it never competes with the page
// for the main thread and so it can read the disc straight from disk.
//
// Talks to app.js with messages; see `onmessage` for the commands and
// `post()` calls for what it reports back.

import init, { Ps1 } from './pkg/ps1_wasm.js';
import * as storage from './storage.js';
import { createPresenter } from './present.js';
import { QualityController } from './quality.js';

const BIOS_KEY = 'bios';
const CARD_COUNT = 2;
const CARD_SIZE = 128 * 1024;
/// Never run more than this many frames at once to catch up after a stall. Catching up in
/// bigger bursts shows as the picture freezing and then jumping.
const MAX_CATCH_UP_FRAMES = 2;
/// In "maximum speed" mode, the time spent emulating before yielding.
const TURBO_BUDGET_MS = 12;
const CARD_SAVE_INTERVAL_MS = 1000;
/// Disc sectors are read from the file in pieces this big.
const DISC_READ_CHUNK = 256 * 1024;
/// The loop wakes up this often to see whether a frame is due.
const TICK_MS = 2;
/// A timer made inside a timer callback waits at least 4 ms once they are nested a few deep, which
/// would make the frames arrive unevenly. The timer fires a message, and the loop runs from the
/// message: the timer it makes there starts the count again.
const tickChannel = new MessageChannel();
tickChannel.port1.onmessage = () => tick();
function scheduleTick() {
    setTimeout(() => tickChannel.port2.postMessage(0), TICK_MS);
}

let wasm = null;
let ps1 = null;
let presenter = null; // draws on the OffscreenCanvas, or null when frames are sent to the page
let audioPort = null;

let biosLoaded = false;
let discLoaded = false;
let running = false;
let paused = false;
let speed = 1;
let pads = [
    { connected: true, mask: 0, lx: 0, ly: 0, rx: 0, ry: 0 },
    { connected: false, mask: 0, lx: 0, ly: 0, rx: 0, ry: 0 },
];
let analog = [false, false];
const appliedAnalog = [null, null];
const appliedConnected = [null, null];
const lastRumble = [0, 0];

let lastFrameTime = 0;
let accumulator = 0;
let lastCardCheck = 0;
let cardSaveFailed = false;
/// What the page's overlay shows, gathered over half a second.
const REPORT_INTERVAL_MS = 500;
const stats = newStats(0);
/// The rendering options in force: the ones the page asked for, or a cheaper rung of them while
/// the machine cannot keep up (see quality.js).
let renderOptions = { scale: 1, trueColor: false, bilinear: false, pgxp: false, pgxpTextures: false, widescreen: false, smooth2d: false };
let adaptiveQuality = true;
const quality = new QualityController();
let shownSize = { width: 0, height: 0 };

function newStats(since) {
    return { since, frames: 0, presented: 0, emuTotal: 0, emuWorst: 0, presentTotal: 0, intervalWorst: 0, lastPresent: 0 };
}

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
    post({ type: 'bios', name, size: bytes.length, info: ps1.bios_description ? ps1.bios_description() : '' });
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

/// A PS-X EXE (homebrew or a test program) instead of a disc.
function loadExe(file) {
    try {
        ps1.load_exe(new Uint8Array(new FileReaderSync().readAsArrayBuffer(file)));
    } catch (error) {
        say(`No se pudo cargar el programa: ${error}`, true);
        return false;
    }
    discLoaded = true;
    post({ type: 'disc', names: [file.name], size: file.size });
    publishState();
    return true;
}

/// Open the disc made of `files`. With `swap` the console keeps running and
/// the game sees the lid being opened; otherwise it is a fresh insertion.
function loadDisc(files, swap = false) {
    if (!swap && files.length === 1 && /\.(exe|psx|ps-exe)$/i.test(files[0].name)) {
        loadExe(files[0]);
        return;
    }
    const cue = files.find((f) => /\.cue$/i.test(f.name));
    const ccd = files.find((f) => /\.ccd$/i.test(f.name));
    const sbi = files.find((f) => /\.sbi$/i.test(f.name));
    // The CloneCD sub-channel dump (.sub) is not used.
    const data = files.filter((f) => !/\.(cue|ccd|sbi|sub)$/i.test(f.name));
    if (data.length === 0) {
        say('Falta el archivo .bin (o .iso) del disco.', true);
        return;
    }
    try {
        for (const file of data) {
            ps1.add_lazy_disc_file(file.name, file.size, makeReader(file));
        }
        const cueText = cue ? new FileReaderSync().readAsText(cue) : undefined;
        if (ccd && !cue) ps1.set_disc_ccd(new FileReaderSync().readAsText(ccd));
        if (sbi) ps1.set_disc_sbi(new Uint8Array(new FileReaderSync().readAsArrayBuffer(sbi)));
        if (swap) ps1.swap_disc_files(cueText);
        else ps1.insert_disc_files(cueText);
    } catch (error) {
        ps1.discard_disc_files();
        say(`No se pudo cargar el disco: ${error}`, true);
        return;
    }
    discLoaded = true;
    const total = data.reduce((sum, f) => sum + f.size, 0);
    post({ type: 'disc', names: (cue || ccd ? [cue || ccd] : data).map((f) => f.name), size: total });
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
    quality.skip();
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
    const width = ps1.frame_width();
    const height = ps1.frame_height();
    if (width !== shownSize.width || height !== shownSize.height) {
        shownSize = { width, height };
        post({ type: 'frame-size', width, height, nativeWidth: ps1.framebuffer_width(), nativeHeight: ps1.framebuffer_height() });
    }
    // A view of the frame inside the module's memory: no copy until it is drawn.
    const pixels = new Uint8Array(wasm.memory.buffer, ps1.frame_ptr(), ps1.frame_len());
    if (presenter) {
        presenter.draw(pixels, width, height);
    } else {
        // No OffscreenCanvas: the page draws it.
        const copy = pixels.slice().buffer;
        post({ type: 'frame', width, height, pixels: copy }, [copy]);
    }
}

function updateStats(now) {
    const elapsed = now - stats.since;
    if (elapsed < REPORT_INTERVAL_MS) return;
    post({
        type: 'stats',
        fps: (stats.frames * 1000) / elapsed,
        full: ps1.frames_per_second(),
        presented: (stats.presented * 1000) / elapsed,
        emuAverage: stats.frames > 0 ? stats.emuTotal / stats.frames : 0,
        emuWorst: stats.emuWorst,
        presentAverage: stats.presented > 0 ? stats.presentTotal / stats.presented : 0,
        intervalWorst: stats.intervalWorst,
        width: shownSize.width,
        height: shownSize.height,
        scale: renderOptions.scale,
        render: renderOptions,
        quality: quality.reduction,
        heap: wasm.memory.buffer.byteLength,
        presenter: presenter ? presenter.kind : 'page',
    });
    const lastPresent = stats.lastPresent;
    Object.assign(stats, newStats(now));
    stats.lastPresent = lastPresent;
}

async function persistCards(now, force = false) {
    if (!force && now - lastCardCheck < CARD_SAVE_INTERVAL_MS) return;
    lastCardCheck = now;
    for (let port = 0; port < CARD_COUNT; port++) {
        if (!ps1.memory_card_dirty(port)) continue;
        const data = ps1.memory_card_data(port);
        ps1.clear_memory_card_dirty(port);
        if (!data) continue;
        try {
            await storage.put(`card${port}`, data);
            cardSaveFailed = false;
        } catch (error) {
            // Keep the card flagged so the next pass tries again, and say so once.
            ps1.mark_memory_card_dirty(port);
            if (!cardSaveFailed) {
                cardSaveFailed = true;
                say(`No se pudo guardar la tarjeta ${port + 1} en el navegador (${error}). Usa Exportar para no perder la partida.`, true);
            }
        }
    }
}

/// Give the pads' state to the console, touching only what changed.
function applyInput() {
    for (let port = 0; port < 2; port++) {
        const pad = pads[port];
        if (appliedConnected[port] !== pad.connected) {
            appliedConnected[port] = pad.connected;
            ps1.set_pad_connected(port, pad.connected);
        }
        if (appliedAnalog[port] !== analog[port]) {
            appliedAnalog[port] = analog[port];
            ps1.set_analog_mode(port, analog[port]);
        }
        ps1.set_input(port, pad.mask, pad.lx, pad.ly, pad.rx, pad.ry);
    }
}

/// Tell the page when the game changes how the pads vibrate.
function reportRumble() {
    for (let port = 0; port < 2; port++) {
        const value = ps1.rumble(port);
        if (value !== lastRumble[port]) {
            lastRumble[port] = value;
            post({ type: 'rumble', port, small: (value >> 8) & 0xFF, large: value & 0xFF });
        }
    }
}

function tick() {
    scheduleTick();
    if (!running) {
        lastFrameTime = performance.now();
        return;
    }
    try {
        const now = performance.now();
        applyInput();
        let frames = 0;
        const emuStart = performance.now();
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
            // Still more than a frame behind after catching up (a slow machine, a stalled tab):
            // give the rest up. Running the debt off in bursts would only freeze the picture and
            // make it jump; a slow machine shows every frame, at the speed it manages.
            if (accumulator > frameTime) accumulator = frameTime;
            for (let i = 0; i < frames; i++) ps1.run_frame();
        }
        lastFrameTime = now;
        if (frames > 0) {
            const presentStart = performance.now();
            present();
            const done = performance.now();
            stats.frames += frames;
            stats.presented++;
            stats.emuTotal += presentStart - emuStart;
            stats.emuWorst = Math.max(stats.emuWorst, (presentStart - emuStart) / frames);
            stats.presentTotal += done - presentStart;
            if (stats.lastPresent > 0) stats.intervalWorst = Math.max(stats.intervalWorst, presentStart - stats.lastPresent);
            stats.lastPresent = presentStart;
            const samples = ps1.drain_audio();
            // Audio only makes sense at normal speed.
            if (audioPort && speed === 1 && samples.length > 0) {
                audioPort.postMessage(samples, [samples.buffer]);
            }
            adaptQuality(frames, done - emuStart, done);
        }
        if (frames > 0) reportRumble();
        updateStats(performance.now());
        persistCards(now);
    } catch (error) {
        stop(error);
    }
}

// ----- Rendering options -----

/// Give the core these options.
function useRender(options) {
    renderOptions = options;
    ps1.set_render_settings(
        options.scale,
        options.trueColor,
        options.bilinear,
        options.pgxp,
        options.pgxpTextures,
        options.widescreen,
        options.smooth2d,
    );
}

/// Watch how much of the time it is allowed each frame takes (emulating it and drawing it) and,
/// when the machine cannot keep up, lower the rendering options a rung; raise them again when
/// there is room. Only at normal speed or slower: a faster speed is asking for all the machine has.
function adaptQuality(frames, spent, now) {
    if (speed === 'max' || speed > 1) {
        quality.skip();
        return;
    }
    const allowed = (frames * 1000) / ps1.frames_per_second() / speed;
    const change = quality.sample(spent / allowed, now);
    if (!change) return;
    const previous = renderOptions;
    useRender(change.options);
    post({ type: 'quality', level: change.level, reason: change.reason, previous, options: change.options });
}

/// Hand the page's choices to the core. A frame already on the screen is drawn
/// again so that a change shows up even while the game is paused.
function applyRender(options, adaptive = adaptiveQuality) {
    renderOptions = options;
    adaptiveQuality = adaptive;
    quality.configure(options, adaptive, performance.now());
    if (!ps1) return;
    useRender(options);
    if (running || paused) {
        try {
            present();
        } catch (error) {
            say(`No se pudo dibujar el cuadro: ${error}`, true);
        }
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
        // A raw 128 KB dump, or a DexDrive .gme or PSP/PS3 .vmp file: the core tells which.
        try {
            ps1.insert_memory_card(port, new Uint8Array(message.bytes));
        } catch (error) {
            say(`Ese archivo no es una tarjeta de memoria (${Math.round(message.bytes.byteLength / 1024)} KB): ${error}`, true);
            return;
        }
        // Store the card as the console now has it, without the file's header.
        await storage.put(`card${port}`, ps1.memory_card_data(port));
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
    applyRender(renderOptions);
    if (message.canvas) {
        presenter = createPresenter(message.canvas, message.prefer2d);
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
        case 'swap-disc':
            loadDisc(message.files, true);
            say('Disco cambiado: el juego tiene que darse cuenta (los de varios discos lo piden).');
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
            quality.skip();
            say('En marcha.');
            return publishState();
        case 'speed': speed = message.value; return;
        case 'render': return applyRender(message.options, message.adaptive !== false);
        case 'input':
            pads = message.pads;
            analog = message.analog;
            return;
        case 'audio-port': audioPort = message.port; return;
        case 'card': return cardCommand(message);
        case 'save-state': return saveState(message.slot);
        case 'load-state': return loadState(message.slot);
        case 'flush-cards': return persistCards(performance.now(), true);
    }
};
