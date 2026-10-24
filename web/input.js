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

/** Standard-mapping gamepad button index → PS1 button. */
const GAMEPAD_BUTTONS = [
    ['CROSS', 0], ['CIRCLE', 1], ['SQUARE', 2], ['TRIANGLE', 3],
    ['L1', 4], ['R1', 5], ['L2', 6], ['R2', 7],
    ['SELECT', 8], ['START', 9], ['L3', 10], ['R3', 11],
    ['UP', 12], ['DOWN', 13], ['LEFT', 14], ['RIGHT', 15],
];
const STICK_THRESHOLD = 0.5;
/** Stick positions closer to the centre than this count as centred. */
const STICK_DEADZONE = 0.12;
/** The "Home" button of a standard gamepad toggles the analog mode. */
const GAMEPAD_ANALOG_BUTTON = 16;
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
        this.analogButtonWasDown = [false, false];
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
            if (down && !event.repeat) this.analog[0] = !this.analog[0];
            return;
        }
        const used = Object.values(this.bindings).includes(event.code);
        if (!used) return;
        event.preventDefault();
        if (down) this.keys.add(event.code);
        else this.keys.delete(event.code);
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

    /**
     * The state of both pads: the keyboard and the first gamepad drive pad 1,
     * the second gamepad drives pad 2. Each has the button mask, the sticks
     * (-128..127) and whether anything is plugged in. Also toggles the analog
     * mode when the Home button of a gamepad is pressed.
     */
    poll() {
        const pads = this.gamepads();
        const states = [0, 1].map(() => ({ connected: false, mask: 0, lx: 0, ly: 0, rx: 0, ry: 0 }));
        states[0].connected = true; // pad 1 is always there: it is the keyboard's

        for (const [button, code] of Object.entries(this.bindings)) {
            if (code && this.keys.has(code)) states[0].mask |= BUTTONS[button];
        }
        const scale = (value) => {
            if (Math.abs(value) < STICK_DEADZONE) return 0;
            return Math.max(-128, Math.min(127, Math.round(value * 127)));
        };
        pads.slice(0, 2).forEach((pad, port) => {
            const state = states[port];
            state.connected = true;
            for (const [button, index] of GAMEPAD_BUTTONS) {
                if (pad.buttons[index] && pad.buttons[index].pressed) state.mask |= BUTTONS[button];
            }
            const [x = 0, y = 0, rx = 0, ry = 0] = pad.axes;
            if (this.analog[port]) {
                state.lx = scale(x);
                state.ly = scale(y);
                state.rx = scale(rx);
                state.ry = scale(ry);
            } else {
                // In digital mode the left stick doubles as the D-pad.
                if (x < -STICK_THRESHOLD) state.mask |= BUTTONS.LEFT;
                if (x > STICK_THRESHOLD) state.mask |= BUTTONS.RIGHT;
                if (y < -STICK_THRESHOLD) state.mask |= BUTTONS.UP;
                if (y > STICK_THRESHOLD) state.mask |= BUTTONS.DOWN;
            }
            const home = !!(pad.buttons[GAMEPAD_ANALOG_BUTTON] && pad.buttons[GAMEPAD_ANALOG_BUTTON].pressed);
            if (home && !this.analogButtonWasDown[port]) this.analog[port] = !this.analog[port];
            this.analogButtonWasDown[port] = home;
        });
        return { pads: states, analog: [...this.analog] };
    }
}
