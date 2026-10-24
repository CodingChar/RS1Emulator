import init, { Ps1 } from './pkg/ps1_wasm.js';
import * as storage from './storage.js';
import { AudioOutput } from './audio.js';
import { Input, BUTTON_LABELS, keyName } from './input.js';

const $ = (id) => document.getElementById(id);

const BIOS_KEY = 'bios';
const CARD_COUNT = 2;
const CARD_SIZE = 128 * 1024;
/// Disc files are handed to the emulator in pieces of this size.
const DISC_CHUNK = 8 * 1024 * 1024;
/// Never run more than this many frames to catch up after a stall.
const MAX_CATCH_UP_FRAMES = 4;
/// In "maximum speed" mode, the time spent emulating per displayed frame.
const TURBO_BUDGET_MS = 12;
const CARD_SAVE_INTERVAL_MS = 1000;

const screen = $('screen');
const context = screen.getContext('2d');
const statusLine = $('status');
const statsLine = $('stats');
const progress = $('progress');
const buttons = {
    start: $('btn-start'),
    pause: $('btn-pause'),
    reset: $('btn-reset'),
    saveState: $('btn-save-state'),
    loadState: $('btn-load-state'),
    biosForget: $('btn-bios-forget'),
};

const input = new Input();
const audio = new AudioOutput();

let ps1 = null;
let biosLoaded = false;
let discLoaded = false;
let running = false;
let speed = 1;
let imageData = null;
let lastFrameTime = 0;
let accumulator = 0;
let lastCardCheck = 0;
const stats = { frames: 0, since: 0 };

function say(message, isError = false) {
    statusLine.textContent = message;
    statusLine.classList.toggle('error', isError);
}

function setState(id, text, ok = false) {
    const element = $(id);
    element.textContent = text;
    element.classList.toggle('ok', ok);
}

function formatBytes(bytes) {
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${Math.round(bytes / 1024)} KB`;
}

function refreshButtons() {
    buttons.start.disabled = !ps1 || !biosLoaded;
    buttons.start.textContent = running || discLoaded ? 'Reiniciar con el disco' : 'Iniciar sin disco';
    buttons.pause.disabled = !running && !paused();
    buttons.pause.textContent = paused() ? 'Reanudar' : 'Pausar';
    buttons.reset.disabled = !ps1 || !biosLoaded;
    buttons.saveState.disabled = !running && !paused();
    buttons.loadState.disabled = !running && !paused();
    buttons.biosForget.disabled = !biosLoaded;
}

let pausedByUser = false;
function paused() {
    return pausedByUser;
}

// ----- BIOS -----

async function useBios(bytes, name, remember) {
    try {
        ps1.load_bios(bytes);
    } catch (error) {
        say(`BIOS no válida: ${error}`, true);
        return false;
    }
    biosLoaded = true;
    setState('bios-state', `BIOS cargada: ${name} (${formatBytes(bytes.length)})`, true);
    if (remember) {
        await storage.put(BIOS_KEY, bytes);
    }
    refreshButtons();
    return true;
}

$('bios-input').addEventListener('change', async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (await useBios(bytes, file.name, true)) {
        say('BIOS lista. Elige un disco o pulsa Iniciar para abrir el menú de la BIOS.');
    }
});

buttons.biosForget.addEventListener('click', async () => {
    await storage.remove(BIOS_KEY);
    say('BIOS olvidada: se volverá a pedir al recargar la página.');
    setState('bios-state', 'La BIOS cargada sigue en uso hasta que recargues la página.');
});

// ----- Disc -----

async function loadDisc(fileList) {
    const files = [...fileList];
    if (files.length === 0) return;
    const cue = files.find((f) => /\.cue$/i.test(f.name));
    const data = files.filter((f) => !/\.cue$/i.test(f.name));
    if (data.length === 0) {
        say('Falta el archivo .bin (o .iso) del disco.', true);
        return;
    }
    const total = data.reduce((sum, f) => sum + f.size, 0);
    let done = 0;
    progress.classList.add('visible');
    progress.value = 0;
    try {
        running = false;
        for (const file of data) {
            say(`Leyendo ${file.name}…`);
            ps1.begin_disc_file(file.name, file.size);
            for (let offset = 0; offset < file.size; offset += DISC_CHUNK) {
                const piece = new Uint8Array(await file.slice(offset, offset + DISC_CHUNK).arrayBuffer());
                ps1.add_disc_file_chunk(piece);
                done += piece.length;
                progress.value = done / total;
            }
            ps1.end_disc_file();
        }
        ps1.insert_disc_files(cue ? await cue.text() : undefined);
    } catch (error) {
        ps1.discard_disc_files();
        say(`No se pudo cargar el disco: ${error}`, true);
        progress.classList.remove('visible');
        refreshButtons();
        return;
    }
    progress.classList.remove('visible');
    discLoaded = true;
    const names = (cue ? [cue] : data).map((f) => f.name).join(', ');
    setState('disc-state', `Disco: ${names} (${formatBytes(total)})`, true);
    if (biosLoaded) {
        await startConsole();
    } else {
        say('Disco listo. Falta la BIOS.');
        refreshButtons();
    }
}

$('disc-input').addEventListener('change', (event) => loadDisc(event.target.files));

window.addEventListener('dragover', (event) => event.preventDefault());
window.addEventListener('drop', (event) => {
    event.preventDefault();
    const files = [...event.dataTransfer.files];
    // A dropped BIOS (exactly 512 KB, no disc extension) goes to the BIOS slot.
    if (files.length === 1 && files[0].size === 512 * 1024 && !/\.(cue|iso|img)$/i.test(files[0].name)) {
        files[0].arrayBuffer().then((buffer) => useBios(new Uint8Array(buffer), files[0].name, true));
    } else {
        loadDisc(files);
    }
});

// ----- Running -----

async function startConsole() {
    if (!ps1 || !biosLoaded) return;
    try {
        ps1.reset();
    } catch (error) {
        say(`Error al arrancar: ${error}`, true);
        return;
    }
    await audio.start();
    pausedByUser = false;
    running = true;
    accumulator = 0;
    lastFrameTime = performance.now();
    say(discLoaded ? 'En marcha.' : 'En marcha (sin disco: menú de la BIOS).');
    refreshButtons();
}

buttons.start.addEventListener('click', startConsole);
buttons.reset.addEventListener('click', startConsole);

buttons.pause.addEventListener('click', () => {
    if (pausedByUser) {
        pausedByUser = false;
        running = true;
        lastFrameTime = performance.now();
        accumulator = 0;
        say('En marcha.');
    } else {
        pausedByUser = true;
        running = false;
        say('En pausa.');
    }
    refreshButtons();
});

$('btn-fullscreen').addEventListener('click', () => {
    const wrap = $('screen-wrap');
    if (document.fullscreenElement) document.exitFullscreen();
    else wrap.requestFullscreen?.();
});

$('speed').addEventListener('change', (event) => {
    speed = event.target.value === 'max' ? 'max' : Number(event.target.value);
});

$('volume').addEventListener('input', (event) => audio.setVolume(Number(event.target.value) / 100));

$('btn-mute').addEventListener('click', (event) => {
    const muted = event.currentTarget.getAttribute('aria-pressed') !== 'true';
    event.currentTarget.setAttribute('aria-pressed', String(muted));
    event.currentTarget.textContent = muted ? 'Activar sonido' : 'Silenciar';
    audio.setMuted(muted);
});

function stop(error) {
    running = false;
    pausedByUser = false;
    say(`La emulación se detuvo: ${error}`, true);
    refreshButtons();
}

function present() {
    const width = ps1.framebuffer_width();
    const height = ps1.framebuffer_height();
    if (screen.width !== width || screen.height !== height) {
        screen.width = width;
        screen.height = height;
        imageData = null;
    }
    const pixels = ps1.framebuffer();
    if (!imageData) {
        imageData = new ImageData(new Uint8ClampedArray(width * height * 4), width, height);
    }
    imageData.data.set(pixels);
    context.putImageData(imageData, 0, 0);
}

function runFrames(count) {
    for (let i = 0; i < count; i++) {
        ps1.run_frame();
    }
}

function updateStats(now, frames) {
    stats.frames += frames;
    if (now - stats.since >= 1000) {
        const fps = (stats.frames * 1000) / (now - stats.since);
        const full = ps1.frames_per_second();
        statsLine.textContent = `${fps.toFixed(1)} fps (${Math.round((fps / full) * 100)} % de la velocidad real)`;
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

function frame(now) {
    requestAnimationFrame(frame);
    if (!running) {
        lastFrameTime = now;
        return;
    }
    try {
        ps1.set_input(0, input.mask(), 0, 0, 0, 0);
        let frames = 0;
        if (speed === 'max') {
            const start = performance.now();
            do {
                ps1.run_frame();
                frames++;
            } while (performance.now() - start < TURBO_BUDGET_MS);
        } else {
            accumulator += Math.min(now - lastFrameTime, 100) * speed;
            const frameTime = 1000 / ps1.frames_per_second();
            frames = Math.min(Math.floor(accumulator / frameTime), MAX_CATCH_UP_FRAMES);
            accumulator -= frames * frameTime;
            // Too far behind (a slow machine or a stalled tab): give up the backlog.
            if (accumulator > frameTime * MAX_CATCH_UP_FRAMES) accumulator = 0;
            runFrames(frames);
        }
        lastFrameTime = now;
        if (frames > 0) {
            present();
            const samples = ps1.drain_audio();
            // Audio only makes sense at normal speed.
            if (speed === 1) audio.push(samples);
        }
        updateStats(now, frames);
        persistCards(now);
    } catch (error) {
        stop(error);
    }
}

// ----- Memory cards -----

function setupCardControls() {
    const container = $('cards');
    for (let port = 0; port < CARD_COUNT; port++) {
        const label = document.createElement('span');
        label.textContent = `Tarjeta ${port + 1}:`;
        const exportButton = document.createElement('button');
        exportButton.textContent = 'Exportar';
        const importButton = document.createElement('button');
        importButton.textContent = 'Importar';
        const formatButton = document.createElement('button');
        formatButton.textContent = 'Formatear';
        const file = document.createElement('input');
        file.type = 'file';
        file.className = 'hidden-input';
        file.accept = '.mcr,.mcd,.mc,.bin,.srm,.vmp';

        exportButton.addEventListener('click', () => {
            const data = ps1.memory_card_data(port);
            if (!data) return;
            const url = URL.createObjectURL(new Blob([data], { type: 'application/octet-stream' }));
            const link = document.createElement('a');
            link.href = url;
            link.download = `tarjeta${port + 1}.mcr`;
            link.click();
            URL.revokeObjectURL(url);
        });
        importButton.addEventListener('click', () => file.click());
        file.addEventListener('change', async () => {
            const chosen = file.files[0];
            file.value = '';
            if (!chosen) return;
            const bytes = new Uint8Array(await chosen.arrayBuffer());
            if (bytes.length !== CARD_SIZE) {
                say(`Una tarjeta de memoria ocupa 128 KB y ese archivo ocupa ${formatBytes(bytes.length)}.`, true);
                return;
            }
            ps1.insert_memory_card(port, bytes);
            await storage.put(`card${port}`, bytes);
            say(`Tarjeta ${port + 1} importada.`);
        });
        formatButton.addEventListener('click', async () => {
            if (!confirm(`Se borrará todo el contenido de la tarjeta ${port + 1}. ¿Continuar?`)) return;
            ps1.insert_formatted_memory_card(port);
            await storage.put(`card${port}`, ps1.memory_card_data(port));
            say(`Tarjeta ${port + 1} formateada.`);
        });
        container.append(label, exportButton, importButton, formatButton, file);
    }
}

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
    if (!(await storage.isPersistent())) {
        setState('cards-state', 'Este navegador no permite guardar datos: las tarjetas se pierden al recargar. Usa Exportar.');
    }
}

// ----- Save states -----

buttons.saveState.addEventListener('click', async () => {
    try {
        const slot = $('slot').value;
        await storage.put(`state${slot}`, ps1.save_state());
        say(`Estado guardado en la ranura ${slot}.`);
    } catch (error) {
        say(`No se pudo guardar el estado: ${error}`, true);
    }
});

buttons.loadState.addEventListener('click', async () => {
    const slot = $('slot').value;
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
});

// ----- Controls -----

function renderKeys() {
    const container = $('keys');
    container.replaceChildren();
    for (const [button, label] of Object.entries(BUTTON_LABELS)) {
        const element = document.createElement('button');
        const name = document.createElement('span');
        name.textContent = label;
        const key = document.createElement('span');
        key.className = 'key';
        key.textContent = input.bindings[button] ? keyName(input.bindings[button]) : '—';
        element.append(name, key);
        element.addEventListener('click', () => {
            input.cancelCapture();
            renderKeys();
            element.classList.add('waiting');
            key.textContent = 'pulsa una tecla…';
            input.capture(button, renderKeys);
        });
        container.append(element);
    }
}

$('btn-keys-reset').addEventListener('click', () => {
    input.resetBindings();
    renderKeys();
});

window.addEventListener('gamepadconnected', (event) => {
    setState('pad-state', `Mando conectado: ${event.gamepad.id}. También sigue valiendo el teclado.`, true);
});

// ----- Start-up -----

async function main() {
    renderKeys();
    refreshButtons();
    try {
        await init();
    } catch (error) {
        say(`No se pudo cargar el emulador (¿falta compilar web/pkg?): ${error}`, true);
        return;
    }
    ps1 = new Ps1();
    setupCardControls();
    await restoreCards();

    const savedBios = await storage.get(BIOS_KEY);
    if (savedBios && (await useBios(savedBios, 'guardada en este navegador', false))) {
        say('BIOS recuperada. Elige un disco o pulsa Iniciar para abrir el menú de la BIOS.');
    } else {
        say('Elige tu BIOS para empezar.');
    }
    refreshButtons();
    requestAnimationFrame(frame);
}

window.addEventListener('beforeunload', () => {
    if (ps1) persistCards(0, true);
});

main();
