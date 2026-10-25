// The page: menus, toolbar, quick menu, input and audio. The emulator itself runs
// in worker.js and reports back with messages.

import { AudioOutput } from './audio.js';
import { Input, BUTTON_LABELS, PAD_ACTIONS, PAD_STICKS, PORT_CHOICES, entryName, keyName } from './input.js';
import { loadSettings, saveSettings } from './settings.js';
import { MenuUI, PadNavigator, Toasts } from './ui.js';

const $ = (id) => document.getElementById(id);

const CARD_COUNT = 2;
const SLOTS = [1, 2, 3];
const SPEEDS = ['1', '2', '4', 'max'];
const SPEED_LABELS = { 1: 'Normal', 2: '×2', 4: '×4', max: 'Máxima' };
const SPEED_CHIPS = { 1: '×1', 2: '×2', 4: '×4', max: 'Máx.' };
const ASPECTS = [['4:3', '4:3 (original)'], ['fill', 'Rellenar']];
const FILTERS = [['nearest', 'Píxeles nítidos'], ['linear', 'Suavizado']];
/// Status texts that only echo a pause or resume the page itself asked for.
const QUIET_STATUS = new Set(['En pausa.', 'En marcha.']);

const screen = $('screen');
const stage = $('stage');
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
const toasts = new Toasts($('osd'));
const menu = new MenuUI($('fs-overlay'));
const padNavigator = new PadNavigator();
const dialog = $('settings-dialog');
audio.onChange = () => refreshAudioChip();

let worker = null;
let ready = false;
let biosLoaded = false;
let discLoaded = false;
let running = false;
let paused = false;
let sentInput = '';
let discName = '';
let biosName = '';
/// Whether the game was running when the menu opened, so closing it resumes it.
let resumeAfterMenu = false;
/// Used only when the browser cannot hand the canvas to the worker.
let fallbackContext = null;

function say(message, isError = false) {
    statusLine.textContent = message;
    statusLine.classList.toggle('error', isError);
    if (!QUIET_STATUS.has(message)) toasts.show(message, isError);
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

function inGame() {
    return running || paused;
}

function refreshButtons() {
    buttons.start.disabled = !ready || !biosLoaded;
    buttons.start.lastChild.textContent = inGame() || discLoaded ? 'Reiniciar con el disco' : 'Iniciar sin disco';
    buttons.pause.disabled = !inGame();
    buttons.pause.lastChild.textContent = paused ? 'Reanudar' : 'Pausar';
    buttons.pause.firstChild.textContent = paused ? '▶' : '⏸';
    buttons.reset.disabled = !ready || !biosLoaded;
    buttons.saveState.disabled = !inGame();
    buttons.loadState.disabled = !inGame();
    buttons.biosForget.disabled = !biosLoaded;
    buttons.swap.disabled = !inGame();

    $('btn-welcome-start').disabled = buttons.start.disabled;
    $('welcome').hidden = inGame();
    $('check-bios').classList.toggle('done', biosLoaded);
    $('check-bios').querySelector('.mark').textContent = biosLoaded ? '✓' : '○';
    $('check-disc').classList.toggle('done', discLoaded);
    $('check-disc').querySelector('.mark').textContent = discLoaded ? '✓' : '○';
    refreshMenus();
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
        case 'bios': {
            biosName = message.name;
            const detail = message.info ? ` · ${message.info}` : '';
            setState('bios-state', `BIOS cargada: ${message.name} (${formatBytes(message.size)})${detail}`, true);
            $('bios-state-short').textContent = message.info || `${message.name} (${formatBytes(message.size)})`;
            break;
        }
        case 'disc': {
            discName = message.names[0] || '';
            const text = `${message.names.join(', ')} (${formatBytes(message.size)})`;
            setState('disc-state', `Disco: ${text}`, true);
            $('disc-state-short').textContent = text;
            break;
        }
        case 'stats': {
            const percent = Math.round((message.fps / message.full) * 100);
            statsLine.textContent = `${message.fps.toFixed(1)} fps · ${percent} %`;
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
    event.target.value = '';
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

$('disc-input').addEventListener('change', (event) => {
    const files = [...event.target.files];
    event.target.value = '';
    loadDisc(files);
});

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

$('swap-input').addEventListener('change', (event) => {
    const files = [...event.target.files];
    event.target.value = '';
    if (files.length > 0) worker.postMessage({ type: 'swap-disc', files });
});

// ----- Actions shared by the toolbar, the menu bar and the quick menu -----

async function startConsole() {
    await startAudio();
    worker.postMessage({ type: 'start' });
}

function togglePause() {
    worker.postMessage({ type: paused ? 'resume' : 'pause' });
}

function saveState(slot = settings.slot) {
    if (inGame()) worker.postMessage({ type: 'save-state', slot });
}

function loadState(slot = settings.slot) {
    if (inGame()) worker.postMessage({ type: 'load-state', slot });
}

function setSlot(slot) {
    settings.slot = slot;
    $('slot').value = String(slot);
    saveSettings(settings);
}

function toggleFullscreen() {
    if (document.fullscreenElement) {
        document.exitFullscreen();
    } else if (stage.requestFullscreen) {
        // Lets Esc reach the page, so it can open the quick menu (hold Esc to leave).
        stage.requestFullscreen().then(() => navigator.keyboard?.lock?.(['Escape'])).catch(() => {});
    }
}

function openSettings(page = 'general') {
    selectSettingsPage(page);
    if (!dialog.open) dialog.showModal();
}

function applySpeed(choice) {
    const value = choice === 'max' ? 'max' : Number(choice);
    worker.postMessage({ type: 'speed', value });
    $('speed-chip').textContent = SPEED_CHIPS[choice] || '×1';
    refreshAudioChip();
}

function setSpeed(choice) {
    settings.speed = choice;
    $('speed').value = choice;
    saveSettings(settings);
    applySpeed(choice);
}

function setVolume(percent) {
    settings.volume = Math.max(0, Math.min(100, percent));
    $('volume').value = settings.volume;
    saveSettings(settings);
    audio.setVolume(settings.volume / 100);
    refreshAudioChip();
}

function setMuted(muted) {
    settings.muted = muted;
    const button = $('btn-mute');
    button.setAttribute('aria-pressed', String(muted));
    button.textContent = muted ? 'Activar sonido' : 'Silenciar';
    saveSettings(settings);
    audio.setMuted(muted);
    refreshAudioChip();
}

/// The speaker button, the status-bar chip and the volume sliders, kept in step.
function refreshAudioChip() {
    const tool = $('btn-mute-tool');
    tool.textContent = settings.muted || settings.volume === 0 ? '🔇' : '🔊';
    tool.setAttribute('aria-pressed', String(settings.muted));
    tool.title = settings.muted ? 'Activar el sonido' : 'Silenciar';
    $('volume-tool').value = settings.volume;

    const chip = $('audio-chip');
    let text = '🔊 Sonido activo';
    let level = 'audio-ok';
    if (settings.muted) {
        text = '🔇 Silenciado';
        level = '';
    } else if (settings.speed !== '1') {
        text = '🔇 Sin sonido a esta velocidad';
        level = 'audio-warn';
    } else if (audio.status !== 'running') {
        text = '⚠ Sonido en espera: haz clic aquí';
        level = 'audio-warn';
    }
    chip.textContent = text;
    chip.className = `chip ${level}`.trim();
    chip.title = level === 'audio-warn'
        ? 'El navegador bloquea el sonido hasta que interactúas con la página, y a ×2, ×4 y máxima no se reproduce'
        : 'Clic para silenciar o activar el sonido';
}

function applyDisplay() {
    stage.classList.toggle('aspect-fill', settings.aspect === 'fill');
    stage.classList.toggle('filter-linear', settings.filter === 'linear');
    $('aspect').value = settings.aspect;
    $('filter').value = settings.filter;
}

function setAspect(value) {
    settings.aspect = value;
    saveSettings(settings);
    applyDisplay();
}

function setFilter(value) {
    settings.filter = value;
    saveSettings(settings);
    applyDisplay();
}

function toggleAnalog() {
    input.toggleAnalog(0);
    toasts.show(input.analog[0] ? 'Modo analógico encendido' : 'Modo analógico apagado');
}

function cycle(list, current, step) {
    const index = list.indexOf(current);
    return list[(index + step + list.length) % list.length];
}

// ----- Toolbar -----

$('btn-open').addEventListener('click', () => $('disc-input').click());
buttons.start.addEventListener('click', startConsole);
buttons.reset.addEventListener('click', startConsole);
buttons.pause.addEventListener('click', togglePause);
buttons.swap.addEventListener('click', () => $('swap-input').click());
buttons.saveState.addEventListener('click', () => saveState());
buttons.loadState.addEventListener('click', () => loadState());
$('slot').addEventListener('change', (event) => setSlot(Number(event.target.value)));
$('btn-quick').addEventListener('click', () => openQuickMenu());
$('btn-fullscreen').addEventListener('click', toggleFullscreen);
$('btn-settings').addEventListener('click', () => openSettings());

$('btn-pick-bios').addEventListener('click', () => $('bios-input').click());
$('btn-pick-disc').addEventListener('click', () => $('disc-input').click());
$('btn-bios-pick').addEventListener('click', () => $('bios-input').click());
$('btn-disc-pick').addEventListener('click', () => $('disc-input').click());
$('btn-welcome-start').addEventListener('click', startConsole);
$('btn-welcome-menu').addEventListener('click', () => openQuickMenu());

// ----- Menu bar -----

const menuBarSpec = {
    system: () => [
        { label: 'Abrir disco…', run: () => $('disc-input').click() },
        { label: 'Elegir BIOS…', run: () => $('bios-input').click() },
        { label: 'Cambiar disco…', disabled: !inGame(), run: () => $('swap-input').click() },
        { rule: true },
        { label: 'Iniciar', disabled: !ready || !biosLoaded, run: startConsole },
        { label: paused ? 'Reanudar' : 'Pausar', disabled: !inGame(), run: togglePause },
        { label: 'Reiniciar consola', disabled: !ready || !biosLoaded, run: startConsole },
        { rule: true },
        ...SLOTS.map((slot) => ({ label: `Guardar estado en la ranura ${slot}`, disabled: !inGame(), run: () => saveState(slot) })),
        ...SLOTS.map((slot) => ({ label: `Cargar estado de la ranura ${slot}`, disabled: !inGame(), run: () => loadState(slot) })),
    ],
    view: () => [
        { label: 'Menú rápido', shortcut: 'Esc', run: () => openQuickMenu() },
        { label: document.fullscreenElement ? 'Salir de pantalla completa' : 'Pantalla completa', shortcut: 'Alt+Intro', run: toggleFullscreen },
        { rule: true },
        ...ASPECTS.map(([value, name]) => ({ label: `${settings.aspect === value ? '● ' : '○ '}Aspecto: ${name}`, run: () => setAspect(value) })),
        ...FILTERS.map(([value, name]) => ({ label: `${settings.filter === value ? '● ' : '○ '}Filtro: ${name}`, run: () => setFilter(value) })),
    ],
    settings: () => [
        { label: 'Configuración…', run: () => openSettings('general') },
        { label: 'Controles…', run: () => openSettings('controls') },
        { label: 'Audio…', run: () => openSettings('audio') },
        { label: 'Tarjetas de memoria…', run: () => openSettings('cards') },
        { rule: true },
        { label: 'Alternar modo analógico', shortcut: 'F9', run: toggleAnalog },
    ],
    help: () => [
        { label: 'Atajos de teclado', run: () => openSettings('general') },
        { label: 'DuckStation (inspiración del diseño)', run: () => window.open('https://github.com/stenzek/duckstation', '_blank', 'noopener') },
    ],
};

function refreshMenus() {
    for (const element of document.querySelectorAll('.menu')) {
        const list = element.querySelector('.menu-list');
        list.replaceChildren(...menuBarSpec[element.dataset.menu]().map((entry) => {
            const item = document.createElement('li');
            if (entry.rule) {
                item.className = 'rule';
                return item;
            }
            item.setAttribute('role', 'menuitem');
            item.classList.toggle('disabled', !!entry.disabled);
            const label = document.createElement('span');
            label.textContent = entry.label;
            item.append(label);
            if (entry.shortcut) {
                const shortcut = document.createElement('span');
                shortcut.className = 'shortcut';
                shortcut.textContent = entry.shortcut;
                item.append(shortcut);
            }
            item.addEventListener('click', () => {
                closeMenuBar();
                entry.run();
            });
            return item;
        }));
    }
}

function closeMenuBar() {
    for (const element of document.querySelectorAll('.menu.open')) element.classList.remove('open');
}

for (const element of document.querySelectorAll('.menu')) {
    element.querySelector('.menu-title').addEventListener('click', (event) => {
        event.stopPropagation();
        const wasOpen = element.classList.contains('open');
        closeMenuBar();
        element.classList.toggle('open', !wasOpen);
    });
    // Once one menu is open, moving over a neighbouring title opens it, like a desktop menu bar.
    element.querySelector('.menu-title').addEventListener('mouseenter', () => {
        if (!document.querySelector('.menu.open') || element.classList.contains('open')) return;
        closeMenuBar();
        element.classList.add('open');
    });
}
document.addEventListener('click', closeMenuBar);

// ----- Quick menu -----

function openQuickMenu() {
    if (menu.isOpen || dialog.open) return;
    resumeAfterMenu = running && !paused;
    if (resumeAfterMenu) worker.postMessage({ type: 'pause' });
    menu.open(rootPage(), () => {
        if (resumeAfterMenu && !dialog.open) worker.postMessage({ type: 'resume' });
        resumeAfterMenu = false;
    });
}

function closeQuickMenu() {
    menu.close();
}

function slotPage(kind) {
    const saving = kind === 'save';
    return {
        title: saving ? 'Guardar estado' : 'Cargar estado',
        subtitle: () => 'Elige una ranura',
        items: () => SLOTS.map((slot) => ({
            icon: saving ? '💾' : '📂',
            label: `Ranura ${slot}`,
            value: () => (settings.slot === slot ? 'Actual' : ''),
            accept: () => {
                setSlot(slot);
                if (saving) saveState(slot);
                else loadState(slot);
                closeQuickMenu();
            },
        })),
    };
}

function displayPage() {
    return {
        title: 'Pantalla y sonido',
        subtitle: () => 'Se recuerdan en este navegador',
        items: () => [
            {
                icon: '▭',
                label: 'Relación de aspecto',
                value: () => ASPECTS.find(([value]) => value === settings.aspect)[1],
                left: () => setAspect(cycle(ASPECTS.map(([value]) => value), settings.aspect, -1)),
                right: () => setAspect(cycle(ASPECTS.map(([value]) => value), settings.aspect, 1)),
            },
            {
                icon: '◫',
                label: 'Filtro de la imagen',
                value: () => FILTERS.find(([value]) => value === settings.filter)[1],
                left: () => setFilter(cycle(FILTERS.map(([value]) => value), settings.filter, -1)),
                right: () => setFilter(cycle(FILTERS.map(([value]) => value), settings.filter, 1)),
            },
            {
                icon: '🔊',
                label: 'Volumen',
                value: () => `${settings.volume} %`,
                left: () => setVolume(settings.volume - 10),
                right: () => setVolume(settings.volume + 10),
            },
            {
                icon: '🔇',
                label: 'Silenciar',
                value: () => (settings.muted ? 'Sí' : 'No'),
                left: () => setMuted(!settings.muted),
                right: () => setMuted(!settings.muted),
            },
        ],
    };
}

function rootPage() {
    const playing = inGame();
    return {
        title: playing ? 'Menú rápido' : 'RS1Emu',
        subtitle: () => (playing ? discName || 'Sin disco' : biosLoaded ? 'Listo para jugar' : 'Falta la BIOS'),
        items: () => {
            const items = [];
            if (playing) {
                items.push({ icon: '▶', label: 'Reanudar', accept: closeQuickMenu });
                items.push({ separator: true, label: 'Estados' });
                items.push({ icon: '💾', label: 'Guardar estado', submenu: true, accept: () => menu.push(slotPage('save')) });
                items.push({ icon: '📂', label: 'Cargar estado', submenu: true, accept: () => menu.push(slotPage('load')) });
                items.push({ separator: true, label: 'Juego' });
                items.push({
                    icon: '⏏',
                    label: 'Cambiar disco…',
                    accept: () => {
                        $('swap-input').click();
                        closeQuickMenu();
                    },
                });
                items.push({
                    icon: '↻',
                    label: 'Reiniciar',
                    accept: () => {
                        closeQuickMenu();
                        startConsole();
                    },
                });
            } else {
                items.push({
                    icon: '▶',
                    label: discLoaded ? 'Iniciar' : 'Iniciar sin disco',
                    disabled: !ready || !biosLoaded,
                    accept: () => {
                        closeQuickMenu();
                        startConsole();
                    },
                });
                items.push({
                    icon: '💿',
                    label: 'Abrir disco…',
                    accept: () => {
                        $('disc-input').click();
                        closeQuickMenu();
                    },
                });
                items.push({
                    icon: '🧩',
                    label: 'Elegir BIOS…',
                    value: () => biosName || 'Sin BIOS',
                    accept: () => {
                        $('bios-input').click();
                        closeQuickMenu();
                    },
                });
            }
            items.push({ separator: true, label: 'Opciones rápidas' });
            items.push({
                icon: '⏩',
                label: 'Velocidad',
                value: () => SPEED_LABELS[settings.speed],
                left: () => setSpeed(cycle(SPEEDS, settings.speed, -1)),
                right: () => setSpeed(cycle(SPEEDS, settings.speed, 1)),
            });
            items.push({
                icon: '🎮',
                label: 'Modo analógico (mando 1)',
                value: () => (input.analog[0] ? 'Encendido' : 'Apagado'),
                left: toggleAnalog,
                right: toggleAnalog,
            });
            items.push({
                icon: '🔊',
                label: 'Volumen',
                value: () => (settings.muted ? 'Silenciado' : `${settings.volume} %`),
                left: () => setVolume(settings.volume - 10),
                right: () => setVolume(settings.volume + 10),
            });
            items.push({ icon: '🖥', label: 'Pantalla y sonido', submenu: true, accept: () => menu.push(displayPage()) });
            items.push({
                icon: '⛶',
                label: document.fullscreenElement ? 'Salir de pantalla completa' : 'Pantalla completa',
                accept: () => {
                    toggleFullscreen();
                    closeQuickMenu();
                },
            });
            items.push({
                icon: '⚙',
                label: 'Configuración…',
                accept: () => openSettings('general'),
            });
            return items;
        },
    };
}

/// Keys that drive the quick menu while it is open (and nothing else sees them).
const MENU_KEYS = {
    ArrowUp: 'up',
    ArrowDown: 'down',
    ArrowLeft: 'left',
    ArrowRight: 'right',
    Enter: 'accept',
    NumpadEnter: 'accept',
    Space: 'accept',
    Escape: 'back',
    Backspace: 'back',
};

window.addEventListener('keydown', (event) => {
    if (dialog.open) return;
    if (menu.isOpen) {
        const kind = MENU_KEYS[event.code];
        if (kind || event.code === 'F1') {
            event.preventDefault();
            event.stopPropagation();
            if (event.code === 'F1') closeQuickMenu();
            else menu.press(kind);
        }
        return;
    }
    if (event.code === 'Escape' || event.code === 'F1') {
        event.preventDefault();
        openQuickMenu();
    } else if (event.code === 'Enter' && event.altKey) {
        event.preventDefault();
        toggleFullscreen();
    } else if (event.code === 'F5') {
        event.preventDefault();
        saveState();
    } else if (event.code === 'F7') {
        event.preventDefault();
        loadState();
    }
}, true);

// ----- Settings dialog -----

function selectSettingsPage(page) {
    for (const tab of document.querySelectorAll('.settings-nav button')) {
        tab.classList.toggle('active', tab.dataset.page === page);
        tab.setAttribute('aria-selected', String(tab.dataset.page === page));
    }
    for (const section of document.querySelectorAll('.settings-pages section')) {
        section.hidden = section.dataset.page !== page;
    }
}

for (const tab of document.querySelectorAll('.settings-nav button')) {
    tab.addEventListener('click', () => selectSettingsPage(tab.dataset.page));
}
$('btn-settings-close').addEventListener('click', () => dialog.close());
dialog.addEventListener('close', () => {
    input.cancelCapture();
    input.cancelGamepadCapture();
    renderPads();
    renderKeys();
    // The quick menu stays open behind the dialog; nothing to resume yet.
    if (!menu.isOpen && resumeAfterMenu) {
        worker.postMessage({ type: 'resume' });
        resumeAfterMenu = false;
    }
});
dialog.addEventListener('click', (event) => {
    if (event.target === dialog) dialog.close();
});

$('speed').addEventListener('change', (event) => setSpeed(event.target.value));
$('volume').addEventListener('input', (event) => setVolume(Number(event.target.value)));
$('btn-mute').addEventListener('click', () => setMuted(!settings.muted));
$('btn-mute-tool').addEventListener('click', () => {
    startAudio();
    setMuted(!settings.muted);
});
$('volume-tool').addEventListener('input', (event) => {
    startAudio();
    setVolume(Number(event.target.value));
});
$('audio-chip').addEventListener('click', async () => {
    // Unblock a waiting context first; otherwise the chip toggles the sound.
    if (!settings.muted && settings.speed === '1' && audio.status !== 'running') {
        await startAudio();
        if (audio.status !== 'running') toasts.show('El navegador no deja activar el sonido todavía', true);
    } else {
        setMuted(!settings.muted);
    }
});

/// Browsers keep the audio suspended until the page has been used: any click or key press unlocks it.
for (const type of ['pointerdown', 'keydown']) {
    window.addEventListener(type, () => {
        if (worker && audio.status !== 'running') startAudio();
    }, { capture: true });
}
document.addEventListener('visibilitychange', () => {
    if (!document.hidden && audio.status === 'suspended') audio.context.resume().catch(() => {});
});
$('aspect').addEventListener('change', (event) => setAspect(event.target.value));
$('filter').addEventListener('change', (event) => setFilter(event.target.value));

/// Show the saved preferences and hand them to the audio output and the worker.
function restoreSettings() {
    $('volume').value = settings.volume;
    $('speed').value = settings.speed;
    $('slot').value = String(settings.slot);
    $('speed-chip').textContent = SPEED_CHIPS[settings.speed] || '×1';
    audio.setVolume(settings.volume / 100);
    setMuted(settings.muted);
    applyDisplay();
    refreshAudioChip();
}

// ----- Input -----

/// The pads' state with every button released, sent while a menu has the controls.
function neutral(state) {
    return {
        ...state,
        pads: state.pads.map((pad) => ({ ...pad, mask: 0, lx: 0, ly: 0, rx: 0, ry: 0 })),
    };
}

/// Send the pads' state to the worker when it changes, and drive the menu with gamepads.
function pollInput(now) {
    requestAnimationFrame(pollInput);
    const state = input.poll(now);
    if (input.homePressed) {
        input.homePressed = false;
        if (menu.isOpen) closeQuickMenu();
        else openQuickMenu();
    }
    const events = padNavigator.poll(input.navState(), now);
    if (menu.isOpen && !dialog.open) {
        for (const event of events) menu.press(event);
    }

    const sent = menu.isOpen || dialog.open ? neutral(state) : state;
    const key = JSON.stringify(sent);
    if (key !== sentInput && worker) {
        sentInput = key;
        worker.postMessage({ type: 'input', ...sent });
    }
    const analog = state.analog[0];
    if (dialog.open) updatePadLive();
    $('analog-chip').textContent = analog ? 'Analógico' : 'Digital';
    $('analog-chip').classList.toggle('on', analog);
    $('analog-state').textContent = analog
        ? 'Modo analógico: encendido (F9 para cambiarlo, también en el menú rápido)'
        : 'Modo analógico: apagado (F9 para cambiarlo, también en el menú rápido)';
}

/// Vibrate the gamepad that drives `port` the way the game asks.
function applyRumble({ port, small, large }) {
    const pad = input.padForPort(port);
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
        const slot = document.createElement('div');
        slot.className = 'card-slot';
        const label = document.createElement('strong');
        label.textContent = `Tarjeta ${port + 1}`;
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
        slot.append(label, exportButton, importButton, formatButton, file);
        container.append(slot);
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
    toasts.show(`Mando conectado: ${shortPadName(event.gamepad.id)}`);
    renderPads();
});
window.addEventListener('gamepaddisconnected', (event) => {
    toasts.show(`Mando desconectado: ${shortPadName(event.gamepad.id)}`, true);
    input.cancelGamepadCapture();
    renderPads();
});

// ----- Gamepads (USB and Bluetooth) -----

/// The name without the vendor and product ids browsers append to it.
function shortPadName(id) {
    return id.replace(/\s*\(.*\)\s*$/, '').trim() || id;
}

const openPadMaps = new Set();

function renderPads() {
    const pads = input.gamepads();
    $('pad-empty').hidden = pads.length > 0;
    $('pad-list').replaceChildren(...pads.map(renderPadCard));
    $('pad-count').textContent = pads.length === 0 ? 'Ninguno' : String(pads.length);
}

function renderPadCard(pad) {
    const profile = input.profile(pad);
    const card = document.createElement('div');
    card.className = 'pad-card';
    card.dataset.index = String(pad.index);

    const head = document.createElement('div');
    head.className = 'pad-head';
    const name = document.createElement('strong');
    name.textContent = shortPadName(pad.id);
    name.title = pad.id;
    const badge = document.createElement('span');
    badge.className = 'chip';
    badge.textContent = pad.mapping === 'standard' ? 'Mapa estándar' : 'Mapa propio: revisa los botones';
    head.append(name, badge);

    const live = document.createElement('div');
    live.className = 'pad-live';
    live.dataset.live = String(pad.index);

    const options = document.createElement('div');
    options.className = 'row';
    const port = document.createElement('select');
    for (const [value, label] of PORT_CHOICES) port.add(new Option(label, value));
    port.value = profile.port;
    port.setAttribute('aria-label', 'Puerto del mando');
    port.addEventListener('change', () => {
        input.updateProfile(pad, { port: port.value });
        renderPads();
    });
    const deadzone = document.createElement('label');
    deadzone.className = 'inline-field';
    deadzone.textContent = 'Zona muerta ';
    const range = document.createElement('input');
    range.type = 'range';
    range.min = '0';
    range.max = '50';
    range.value = String(Math.round(profile.deadzone * 100));
    range.addEventListener('input', () => input.updateProfile(pad, { deadzone: Number(range.value) / 100 }));
    deadzone.append(range);
    const rumble = document.createElement('button');
    rumble.textContent = 'Probar vibración';
    rumble.addEventListener('click', () => testRumble(pad));
    const reset = document.createElement('button');
    reset.textContent = 'Restaurar mapa';
    reset.addEventListener('click', () => {
        input.resetProfile(pad);
        renderPads();
    });
    options.append(port, deadzone, rumble, reset);

    const details = document.createElement('details');
    details.className = 'pad-map';
    details.open = pad.mapping !== 'standard' || openPadMaps.has(pad.id);
    details.addEventListener('toggle', () => {
        if (details.open) openPadMaps.add(pad.id);
        else openPadMaps.delete(pad.id);
    });
    const summary = document.createElement('summary');
    summary.textContent = 'Asignar botones y palancas';
    const grid = document.createElement('div');
    grid.className = 'keys';
    const entries = [
        ...Object.entries(PAD_ACTIONS).map(([target, label]) => [target, label, entryName(profile.map[target])]),
        ...Object.entries(PAD_STICKS).map(([target, label]) => [target, label, `Eje ${profile.axes[target]}`]),
    ];
    for (const [target, label, current] of entries) {
        const button = document.createElement('button');
        const text = document.createElement('span');
        text.textContent = label;
        const value = document.createElement('span');
        value.className = 'key';
        value.textContent = current;
        button.append(text, value);
        button.addEventListener('click', () => {
            input.cancelGamepadCapture();
            button.classList.add('waiting');
            value.textContent = target in PAD_STICKS ? 'mueve la palanca…' : 'pulsa un botón…';
            input.captureGamepad(pad.index, target, (assigned) => {
                if (!assigned) toasts.show('Asignación cancelada: no se detectó ningún control', true);
                renderPads();
            });
        });
        grid.append(button);
    }
    details.append(summary, grid);
    card.append(head, live, options, details);
    return card;
}

function testRumble(pad) {
    const actuator = pad.vibrationActuator;
    if (!actuator) {
        toasts.show('Este mando o este navegador no permite vibración', true);
        return;
    }
    actuator
        .playEffect('dual-rumble', { duration: 400, weakMagnitude: 1, strongMagnitude: 1 })
        .catch(() => toasts.show('No se pudo hacer vibrar el mando', true));
}

/// Shows which controls each pad is holding, so a mapping can be checked by pressing buttons.
function updatePadLive() {
    const pads = input.gamepads();
    for (const element of document.querySelectorAll('[data-live]')) {
        const pad = pads.find((candidate) => String(candidate.index) === element.dataset.live);
        if (!pad) continue;
        const held = input.heldNames(pad).map((key) => PAD_ACTIONS[key]);
        const port = input.portOf(pad);
        const where = port === null ? 'sin puerto' : `puerto ${port + 1}`;
        element.textContent = `${where} · ${held.length ? `pulsando: ${held.join(', ')}` : 'pulsa un botón para probarlo'}`;
    }
}


window.addEventListener('pagehide', () => {
    if (worker) worker.postMessage({ type: 'flush-cards' });
});

// ----- Full screen -----

let cursorTimer = 0;
document.addEventListener('fullscreenchange', () => {
    stage.classList.remove('idle-cursor');
    clearTimeout(cursorTimer);
    refreshMenus();
});
stage.addEventListener('mousemove', () => {
    stage.classList.remove('idle-cursor');
    clearTimeout(cursorTimer);
    if (document.fullscreenElement && !menu.isOpen) {
        cursorTimer = setTimeout(() => stage.classList.add('idle-cursor'), 2500);
    }
});

renderKeys();
renderPads();
refreshButtons();
setupCardControls();
restoreSettings();
startWorker();
requestAnimationFrame(pollInput);
say('Cargando el emulador…');
