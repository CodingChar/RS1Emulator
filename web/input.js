// Keyboard and gamepad input, turned into the PS1 digital pad button mask.

/** Button masks, as in `ps1_core::controllers::Buttons`. */
export const BUTTONS = {
    SELECT: 0x0001,
    L3: 0x0002,
    R3: 0x0004,
    START: 0x0008,
    UP: 0x0010,
    RIGHT: 0x0020,
    DOWN: 0x0040,
    LEFT: 0x0080,
    L2: 0x0100,
    R2: 0x0200,
    L1: 0x0400,
    R1: 0x0800,
    TRIANGLE: 0x1000,
    CIRCLE: 0x2000,
    CROSS: 0x4000,
    SQUARE: 0x8000,
};

export const BUTTON_LABELS = {
    UP: 'Arriba',
    DOWN: 'Abajo',
    LEFT: 'Izquierda',
    RIGHT: 'Derecha',
    CROSS: '✕ Cruz',
    CIRCLE: '○ Círculo',
    SQUARE: '□ Cuadrado',
    TRIANGLE: '△ Triángulo',
    L1: 'L1',
    R1: 'R1',
    L2: 'L2',
    R2: 'R2',
    START: 'Start',
    SELECT: 'Select',
};

export const DEFAULT_KEYS = {
    UP: 'ArrowUp',
    DOWN: 'ArrowDown',
    LEFT: 'ArrowLeft',
    RIGHT: 'ArrowRight',
    CROSS: 'KeyZ',
    CIRCLE: 'KeyX',
    SQUARE: 'KeyA',
    TRIANGLE: 'KeyS',
    L1: 'KeyQ',
    R1: 'KeyW',
    L2: 'KeyE',
    R2: 'KeyR',
    START: 'Enter',
    SELECT: 'ShiftRight',
};

/** Standard-mapping gamepad button index → PS1 button (the default map of every pad). */
const GAMEPAD_BUTTONS = [
    ['CROSS', 0], ['CIRCLE', 1], ['SQUARE', 2], ['TRIANGLE', 3],
    ['L1', 4], ['R1', 5], ['L2', 6], ['R2', 7],
    ['SELECT', 8], ['START', 9], ['L3', 10], ['R3', 11],
    ['UP', 12], ['DOWN', 13], ['LEFT', 14], ['RIGHT', 15],
];
/** The "Home" button of a standard gamepad opens the quick menu. */
const GAMEPAD_HOME_BUTTON = 16;
const STICK_THRESHOLD = 0.5;
/** Stick positions closer to the centre than this count as centred (unless the pad's profile says otherwise). */
const STICK_DEADZONE = 0.12;
const AXIS_CAPTURE_THRESHOLD = 0.6;
const PAD_CAPTURE_TIMEOUT_MS = 8000;
const PADS_STORAGE_KEY = 'rs1emu.pads';

/** What a gamepad can be told to do, in the order the settings list them. */
export const PAD_ACTIONS = {
    UP: 'Arriba', DOWN: 'Abajo', LEFT: 'Izquierda', RIGHT: 'Derecha',
    CROSS: '✕ Cruz', CIRCLE: '○ Círculo', SQUARE: '□ Cuadrado', TRIANGLE: '△ Triángulo',
    L1: 'L1', R1: 'R1', L2: 'L2', R2: 'R2', L3: 'L3', R3: 'R3',
    START: 'Start', SELECT: 'Select', HOME: 'Menú rápido',
};
export const PAD_STICKS = {
    LX: 'Palanca izq. ↔', LY: 'Palanca izq. ↕', RX: 'Palanca der. ↔', RY: 'Palanca der. ↕',
};
export const PORT_CHOICES = [['auto', 'Automático'], ['0', 'Puerto 1'], ['1', 'Puerto 2'], ['off', 'Desactivado']];

function defaultProfile() {
    const map = {};
    for (const [name, index] of GAMEPAD_BUTTONS) map[name] = { b: index };
    map.HOME = { b: GAMEPAD_HOME_BUTTON };
    return { port: 'auto', deadzone: STICK_DEADZONE, map, axes: { LX: 0, LY: 1, RX: 2, RY: 3 } };
}

/** Whether a mapped control (`{b}` button or `{a, d}` axis direction) is held on `pad`. */
function entryDown(pad, entry) {
    if (!entry) return false;
    if ('b' in entry) return !!(pad.buttons[entry.b] && pad.buttons[entry.b].pressed);
    const value = pad.axes[entry.a];
    return value !== undefined && value * entry.d > STICK_THRESHOLD;
}

/** How a mapped control reads in the settings. */
export function entryName(entry) {
    if (!entry) return '—';
    if ('b' in entry) return `Botón ${entry.b}`;
    return `Eje ${entry.a} ${entry.d > 0 ? '+' : '−'}`;
}

function loadProfiles() {
    try {
        const saved = JSON.parse(localStorage.getItem(PADS_STORAGE_KEY));
        if (saved && typeof saved === 'object') return saved;
    } catch (_) { /* no saved profiles */ }
    return {};
}

const KEY_ANALOG = 'F9';
const STORAGE_KEY = 'rs1emu.keys';

export function loadBindings() {
    try {
        const saved = JSON.parse(localStorage.getItem(STORAGE_KEY));
        if (saved && typeof saved === 'object') {
            return { ...DEFAULT_KEYS, ...saved };
        }
    } catch (_) { /* no saved bindings */ }
    return { ...DEFAULT_KEYS };
}

export function saveBindings(bindings) {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(bindings));
    } catch (_) { /* storage unavailable: bindings last for this session */ }
}

const KEY_NAMES = {
    ArrowUp: '↑',
    ArrowDown: '↓',
    ArrowLeft: '←',
    ArrowRight: '→',
    ShiftRight: 'Shift der.',
    ShiftLeft: 'Shift izq.',
    ControlRight: 'Ctrl der.',
    ControlLeft: 'Ctrl izq.',
    Space: 'Espacio',
    Enter: 'Intro',
    Backspace: 'Retroceso',
    Tab: 'Tab',
};

/** Names the key a binding stores, for display. */
export function keyName(code) {
    return KEY_NAMES[code] || code.replace(/^(Key|Digit)/, '');
}

export class Input {
    constructor() {
        this.bindings = loadBindings();
        this.keys = new Set();
        /** Set while the settings UI waits for a key to bind. */
        this.captureHandler = null;
        /** The analog mode (red LED) of each of the two pads. */
        this.analog = [false, false];
        this.homeWasDown = new Map();
        this.profiles = loadProfiles();
        /** The gamepad (and its profile) that drives each port, from the last poll. */
        this.assigned = [null, null];
        /** Set while the settings UI waits for a gamepad control to bind. */
        this.padCapture = null;
        /** Set when a gamepad's Home button goes down; the page opens its quick menu and clears it. */
        this.homePressed = false;
        window.addEventListener('keydown', (e) => this.onKey(e, true));
        window.addEventListener('keyup', (e) => this.onKey(e, false));
        window.addEventListener('blur', () => this.keys.clear());
    }

    onKey(event, down) {
        if (down && this.captureHandler) {
            event.preventDefault();
            const handler = this.captureHandler;
            this.captureHandler = null;
            handler(event.code);
            return;
        }
        const target = event.target;
        if (target instanceof HTMLElement && /^(INPUT|SELECT|TEXTAREA)$/.test(target.tagName)) {
            return;
        }
        if (event.code === KEY_ANALOG) {
            event.preventDefault();
            if (down && !event.repeat) this.toggleAnalog(0);
            return;
        }
        const used = Object.values(this.bindings).includes(event.code);
        if (!used) return;
        event.preventDefault();
        if (down) this.keys.add(event.code);
        else this.keys.delete(event.code);
    }

    toggleAnalog(port) {
        this.analog[port] = !this.analog[port];
    }

    /** Wait for the next key press and bind it to `button`. */
    capture(button, done) {
        this.captureHandler = (code) => {
            if (code !== 'Escape') {
                // A key drives one button only.
                for (const other of Object.keys(this.bindings)) {
                    if (this.bindings[other] === code) this.bindings[other] = '';
                }
                this.bindings[button] = code;
                saveBindings(this.bindings);
            }
            done();
        };
    }

    cancelCapture() {
        this.captureHandler = null;
    }

    resetBindings() {
        this.bindings = { ...DEFAULT_KEYS };
        saveBindings(this.bindings);
    }

    /** Gamepads that are plugged in, in the order the browser lists them. */
    gamepads() {
        const all = navigator.getGamepads ? navigator.getGamepads() : [];
        return [...all].filter((pad) => pad && pad.connected);
    }

    hasGamepad() {
        return this.gamepads().length > 0;
    }

    /** The profile of `pad`: its saved one, or the default (standard layout). */
    profile(pad) {
        const saved = this.profiles[pad.id] || {};
        const base = defaultProfile();
        return {
            ...base,
            ...saved,
            map: { ...base.map, ...saved.map },
            axes: { ...base.axes, ...saved.axes },
        };
    }

    updateProfile(pad, change) {
        this.profiles[pad.id] = { ...this.profile(pad), ...change };
        this.saveProfiles();
    }

    resetProfile(pad) {
        delete this.profiles[pad.id];
        this.saveProfiles();
    }

    saveProfiles() {
        try {
            localStorage.setItem(PADS_STORAGE_KEY, JSON.stringify(this.profiles));
        } catch (_) { /* profiles last for this session */ }
    }

    /** Which pad drives which port: explicit choices first, then the rest in order. */
    assignPorts(pads) {
        const slots = [null, null];
        const entries = pads.map((pad) => ({ pad, profile: this.profile(pad), placed: false }));
        for (const entry of entries) {
            const choice = entry.profile.port;
            if ((choice === '0' || choice === '1') && !slots[Number(choice)]) {
                slots[Number(choice)] = entry;
                entry.placed = true;
            }
        }
        for (const entry of entries) {
            if (entry.placed || entry.profile.port !== 'auto') continue;
            const free = slots.indexOf(null);
            if (free >= 0) slots[free] = entry;
        }
        return slots;
    }

    /** The pad that drives `port` right now, or `undefined`. */
    padForPort(port) {
        const entry = this.assigned[port];
        return entry ? entry.pad : undefined;
    }

    /** The port a pad drives right now (`null` when it is off or there is no room). */
    portOf(pad) {
        const index = this.assigned.findIndex((entry) => entry && entry.pad.index === pad.index);
        return index >= 0 ? index : null;
    }

    /** Names of the controls held on `pad`, for the settings' live test. */
    heldNames(pad) {
        const profile = this.profile(pad);
        return Object.keys(PAD_ACTIONS).filter((name) => entryDown(pad, profile.map[name]));
    }

    /** Wait for a control of the pad at `index` and bind it to `target` (an action or a stick axis). */
    captureGamepad(index, target, done) {
        const pad = this.gamepads().find((candidate) => candidate.index === index);
        if (!pad) return false;
        this.padCapture = {
            index,
            target,
            done,
            started: performance.now(),
            baseAxes: [...pad.axes],
            baseButtons: pad.buttons.map((button) => button.pressed),
        };
        return true;
    }

    cancelGamepadCapture() {
        this.padCapture = null;
    }

    /** Look for the control the user moves while a capture is waiting. */
    pollCapture(pads, now) {
        const capture = this.padCapture;
        if (!capture) return;
        const pad = pads.find((candidate) => candidate.index === capture.index);
        if (!pad || now - capture.started > PAD_CAPTURE_TIMEOUT_MS) {
            this.padCapture = null;
            capture.done(false);
            return;
        }
        const isStick = capture.target in PAD_STICKS;
        let entry = null;
        if (!isStick) {
            const button = pad.buttons.findIndex((state, i) => state.pressed && !capture.baseButtons[i]);
            if (button >= 0) entry = { b: button };
        }
        if (!entry) {
            pad.axes.forEach((value, i) => {
                const moved = value - (capture.baseAxes[i] || 0);
                if (!entry && Math.abs(moved) > AXIS_CAPTURE_THRESHOLD) {
                    entry = isStick ? { a: i } : { a: i, d: Math.sign(moved) };
                }
            });
        }
        // A button held when the capture started does not count until it is released.
        capture.baseButtons = capture.baseButtons.map((was, i) => was && !!(pad.buttons[i] && pad.buttons[i].pressed));
        if (!entry) return;
        this.padCapture = null;
        if (isStick) {
            this.updateProfile(pad, { axes: { ...this.profile(pad).axes, [capture.target]: entry.a } });
        } else {
            this.updateProfile(pad, { map: { ...this.profile(pad).map, [capture.target]: entry } });
        }
        capture.done(true);
    }

    /** Menu navigation from every pad, through each pad's own button map. */
    navState() {
        const state = { up: false, down: false, left: false, right: false, accept: false, back: false };
        for (const pad of this.gamepads()) {
            const { map, axes } = this.profile(pad);
            const x = pad.axes[axes.LX] || 0;
            const y = pad.axes[axes.LY] || 0;
            state.up ||= entryDown(pad, map.UP) || y < -0.6;
            state.down ||= entryDown(pad, map.DOWN) || y > 0.6;
            state.left ||= entryDown(pad, map.LEFT) || x < -0.6;
            state.right ||= entryDown(pad, map.RIGHT) || x > 0.6;
            state.accept ||= entryDown(pad, map.CROSS);
            state.back ||= entryDown(pad, map.CIRCLE);
        }
        return state;
    }

    /**
     * The state of both pads: the keyboard and the first gamepad drive pad 1,
     * the second gamepad drives pad 2 (or whatever the pads' profiles say). Each
     * has the button mask, the sticks (-128..127) and whether anything is
     * plugged in. Also raises `homePressed` when a gamepad's menu button goes down.
     */
    poll(now = performance.now()) {
        const pads = this.gamepads();
        this.pollCapture(pads, now);
        const states = [0, 1].map(() => ({ connected: false, mask: 0, lx: 0, ly: 0, rx: 0, ry: 0 }));
        states[0].connected = true; // pad 1 is always there: it is the keyboard's

        for (const [button, code] of Object.entries(this.bindings)) {
            if (code && this.keys.has(code)) states[0].mask |= BUTTONS[button];
        }
        this.assigned = this.assignPorts(pads);

        for (const pad of pads) {
            const home = entryDown(pad, this.profile(pad).map.HOME);
            if (home && !this.homeWasDown.get(pad.index)) this.homePressed = true;
            this.homeWasDown.set(pad.index, home);
        }

        this.assigned.forEach((entry, port) => {
            if (!entry) return;
            const { pad, profile } = entry;
            const state = states[port];
            state.connected = true;
            for (const name of Object.keys(BUTTONS)) {
                if (entryDown(pad, profile.map[name])) state.mask |= BUTTONS[name];
            }
            const scale = (value = 0) => {
                if (Math.abs(value) < profile.deadzone) return 0;
                return Math.max(-128, Math.min(127, Math.round(value * 127)));
            };
            const x = pad.axes[profile.axes.LX] || 0;
            const y = pad.axes[profile.axes.LY] || 0;
            if (this.analog[port]) {
                state.lx = scale(x);
                state.ly = scale(y);
                state.rx = scale(pad.axes[profile.axes.RX]);
                state.ry = scale(pad.axes[profile.axes.RY]);
            } else {
                // In digital mode the left stick doubles as the D-pad.
                if (x < -STICK_THRESHOLD) state.mask |= BUTTONS.LEFT;
                if (x > STICK_THRESHOLD) state.mask |= BUTTONS.RIGHT;
                if (y < -STICK_THRESHOLD) state.mask |= BUTTONS.UP;
                if (y > STICK_THRESHOLD) state.mask |= BUTTONS.DOWN;
            }
        });
        return { pads: states, analog: [...this.analog] };
    }
}
