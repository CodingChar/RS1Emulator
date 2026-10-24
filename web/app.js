// The page: file pickers, buttons, input and audio. The emulator itself runs
// in worker.js and reports back with messages.

import { AudioOutput } from './audio.js';
import { Input, BUTTON_LABELS, keyName } from './input.js';
import { loadSettings, saveSettings } from './settings.js';

const $ = (id) => document.getElementById(id);

const CARD_COUNT = 2;

const screen = $('screen');
const statusLine = $('status');
const statsLine = $('stats');
const buttons = {
    start: $('btn-start'),
    pause: $('btn-pause'),
    reset: $('btn-reset'),
    saveState: $('btn-save-state'),
    loadState: $('btn-load-state'),
    biosForget: $('btn-bios-forget'),
    swap: $('btn-swap'),
};
const settings = loadSettings();

const input = new Input();
const audio = new AudioOutput();

let worker = null;
let ready = false;
let biosLoaded = false;
let discLoaded = false;
let running = false;
let paused = false;
let sentInput = '';
/// Used only when the browser cannot hand the canvas to the worker.
let fallbackContext = null;

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
    buttons.start.disabled = !ready || !biosLoaded;
    buttons.start.textContent = running || discLoaded ? 'Reiniciar con el disco' : 'Iniciar sin disco';
    buttons.pause.disabled = !running && !paused;
    buttons.pause.textContent = paused ? 'Reanudar' : 'Pausar';
    buttons.reset.disabled = !ready || !biosLoaded;
    buttons.saveState.disabled = !running && !paused;
    buttons.loadState.disabled = !running && !paused;
    buttons.biosForget.disabled = !biosLoaded;
    buttons.swap.disabled = !running && !paused;
}

// ----- Talking to the worker -----

/// Hand the audio output to the worker: a channel straight from the worker
/// to the audio thread, so samples never pass through this thread.
function connectAudio() {
    const port = audio.createChannel();
    if (port) worker.postMessage({ type: 'audio-port', port }, [port]);
}

async function startAudio() {
    const first = !audio.available;
    await audio.start();
    if (first && audio.available) connectAudio();
}

function onWorkerMessage(event) {
    const message = event.data;
    switch (message.type) {
        case 'ready':
            ready = true;
            applySpeed(settings.speed);
            if (!message.persistent) {
                setState('cards-state', 'Este navegador no permite guardar datos: las tarjetas se pierden al recargar. Usa Exportar.');
            }
            refreshButtons();
            break;
        case 'status':
            say(message.text, message.isError);
            break;
        case 'state':
            ({ biosLoaded, discLoaded, running, paused } = message);
            refreshButtons();
            break;
        case 'bios':
            setState('bios-state', `BIOS cargada: ${message.name} (${formatBytes(message.size)})`, true);
            break;
        case 'disc':
            setState('disc-state', `Disco: ${message.names.join(', ')} (${formatBytes(message.size)})`, true);
            break;
        case 'stats': {
            const percent = Math.round((message.fps / message.full) * 100);
            statsLine.textContent = `${message.fps.toFixed(1)} fps (${percent} % de la velocidad real)`;
            break;
        }
        case 'frame':
            drawFallbackFrame(message);
            break;
        case 'rumble':
            applyRumble(message);
            break;
        case 'card-export': {
            const url = URL.createObjectURL(new Blob([message.data], { type: 'application/octet-stream' }));
            const link = document.createElement('a');
            link.href = url;
            link.download = `tarjeta${message.port + 1}.mcr`;
            link.click();
            URL.revokeObjectURL(url);
            break;
        }
    }
}

function drawFallbackFrame({ width, height, pixels }) {
    if (!fallbackContext) fallbackContext = screen.getContext('2d');
    if (screen.width !== width || screen.height !== height) {
        screen.width = width;
        screen.height = height;
    }
    fallbackContext.putImageData(new ImageData(new Uint8ClampedArray(pixels), width, height), 0, 0);
}

function startWorker() {
    worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
    worker.onmessage = onWorkerMessage;
    worker.onerror = (event) => say(`Error en el emulador: ${event.message}`, true);
    if ('transferControlToOffscreen' in screen) {
        const offscreen = screen.transferControlToOffscreen();
        worker.postMessage({ type: 'start-worker', canvas: offscreen }, [offscreen]);
    } else {
        worker.postMessage({ type: 'start-worker', canvas: null });
    }
}

// ----- BIOS and disc -----

$('bios-input').addEventListener('change', (event) => {
    const file = event.target.files[0];
    if (file) worker.postMessage({ type: 'bios', file });
});

buttons.biosForget.addEventListener('click', () => {
    worker.postMessage({ type: 'forget-bios' });
    say('BIOS olvidada: se volverá a pedir al recargar la página.');
    setState('bios-state', 'La BIOS cargada sigue en uso hasta que recargues la página.');
});

async function loadDisc(files) {
    files = [...files];
    if (files.length === 0 || !ready) return;
    await startAudio();
    say('Abriendo el disco…');
    worker.postMessage({ type: 'disc', files });
}

$('disc-input').addEventListener('change', (event) => loadDisc(event.target.files));

window.addEventListener('dragover', (event) => event.preventDefault());
window.addEventListener('drop', (event) => {
    event.preventDefault();
    const files = [...event.dataTransfer.files];
    // A dropped BIOS (exactly 512 KB, no disc extension) goes to the BIOS slot.
    if (files.length === 1 && files[0].size === 512 * 1024 && !/\.(cue|iso|img)$/i.test(files[0].name)) {
        worker.postMessage({ type: 'bios', file: files[0] });
    } else {
        loadDisc(files);
    }
});

// ----- Controls -----

async function startConsole() {
    await startAudio();
    worker.postMessage({ type: 'start' });
}

buttons.start.addEventListener('click', startConsole);
buttons.reset.addEventListener('click', startConsole);
buttons.pause.addEventListener('click', () => {
    worker.postMessage({ type: paused ? 'resume' : 'pause' });
});

$('btn-fullscreen').addEventListener('click', () => {
    const wrap = $('screen-wrap');
    if (document.fullscreenElement) document.exitFullscreen();
    else wrap.requestFullscreen?.();
});

function applySpeed(choice) {
    const value = choice === 'max' ? 'max' : Number(choice);
    worker.postMessage({ type: 'speed', value });
}

function applyMuted(muted) {
    const button = $('btn-mute');
    button.setAttribute('aria-pressed', String(muted));
    button.textContent = muted ? 'Activar sonido' : 'Silenciar';
    audio.setMuted(muted);
}

$('speed').addEventListener('change', (event) => {
    settings.speed = event.target.value;
    saveSettings(settings);
    applySpeed(settings.speed);
});

$('volume').addEventListener('input', (event) => {
    settings.volume = Number(event.target.value);
    saveSettings(settings);
    audio.setVolume(settings.volume / 100);
});

$('btn-mute').addEventListener('click', (event) => {
    settings.muted = event.currentTarget.getAttribute('aria-pressed') !== 'true';
    saveSettings(settings);
    applyMuted(settings.muted);
});

buttons.swap.addEventListener('click', () => $('swap-input').click());
$('swap-input').addEventListener('change', (event) => {
    const files = [...event.target.files];
    event.target.value = '';
    if (files.length > 0) worker.postMessage({ type: 'swap-disc', files });
});

/// Show the saved preferences and hand them to the audio output and the worker.
function restoreSettings() {
    $('volume').value = settings.volume;
    $('speed').value = settings.speed;
    audio.setVolume(settings.volume / 100);
    applyMuted(settings.muted);
}

buttons.saveState.addEventListener('click', () => {
    worker.postMessage({ type: 'save-state', slot: Number($('slot').value) });
});

buttons.loadState.addEventListener('click', () => {
    worker.postMessage({ type: 'load-state', slot: Number($('slot').value) });
});

/// Send the pads' state to the worker when it changes, and show the analog LED.
function pollInput() {
    requestAnimationFrame(pollInput);
    const state = input.poll();
    const key = JSON.stringify(state);
    if (key !== sentInput && worker) {
        sentInput = key;
        worker.postMessage({ type: 'input', ...state });
        $('analog-state').textContent = state.analog[0]
            ? 'Modo analógico: encendido (F9 o botón Home del mando para cambiarlo)'
            : 'Modo analógico: apagado (F9 o botón Home del mando para cambiarlo)';
    }
}

/// Vibrate the gamepad that drives `port` the way the game asks.
function applyRumble({ port, small, large }) {
    const pad = input.gamepads()[port];
    const actuator = pad && pad.vibrationActuator;
    if (!actuator) return;
    if (small === 0 && large === 0) {
        actuator.reset?.();
    } else {
        actuator
            .playEffect('dual-rumble', {
                duration: 5000,
                weakMagnitude: small / 255,
                strongMagnitude: large / 255,
            })
            .catch(() => {});
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

        exportButton.addEventListener('click', () => worker.postMessage({ type: 'card', op: 'export', port }));
        importButton.addEventListener('click', () => file.click());
        file.addEventListener('change', async () => {
            const chosen = file.files[0];
            file.value = '';
            if (!chosen) return;
            const bytes = await chosen.arrayBuffer();
            worker.postMessage({ type: 'card', op: 'import', port, bytes }, [bytes]);
        });
        formatButton.addEventListener('click', () => {
            if (!confirm(`Se borrará todo el contenido de la tarjeta ${port + 1}. ¿Continuar?`)) return;
            worker.postMessage({ type: 'card', op: 'format', port });
        });
        container.append(label, exportButton, importButton, formatButton, file);
    }
}

// ----- Key bindings -----

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

window.addEventListener('pagehide', () => {
    if (worker) worker.postMessage({ type: 'flush-cards' });
});

renderKeys();
refreshButtons();
setupCardControls();
restoreSettings();
startWorker();
pollInput();
say('Cargando el emulador…');
