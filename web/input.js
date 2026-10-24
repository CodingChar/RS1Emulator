// Keyboard and gamepad input, turned into the PS1 digital pad button mask.

/** Button masks, as in `ps1_core::controllers::Buttons`. */
export const BUTTONS = {
    SELECT: 0x0001,
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
    ['SELECT', 8], ['START', 9],
    ['UP', 12], ['DOWN', 13], ['LEFT', 14], ['RIGHT', 15],
];
const STICK_THRESHOLD = 0.5;
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

    /** The current button mask from the keyboard and every connected gamepad. */
    mask() {
        let mask = 0;
        for (const [button, code] of Object.entries(this.bindings)) {
            if (code && this.keys.has(code)) mask |= BUTTONS[button];
        }
        const pads = navigator.getGamepads ? navigator.getGamepads() : [];
        for (const pad of pads) {
            if (!pad || !pad.connected) continue;
            for (const [button, index] of GAMEPAD_BUTTONS) {
                if (pad.buttons[index] && pad.buttons[index].pressed) mask |= BUTTONS[button];
            }
            // The left stick doubles as the digital pad.
            const [x = 0, y = 0] = pad.axes;
            if (x < -STICK_THRESHOLD) mask |= BUTTONS.LEFT;
            if (x > STICK_THRESHOLD) mask |= BUTTONS.RIGHT;
            if (y < -STICK_THRESHOLD) mask |= BUTTONS.UP;
            if (y > STICK_THRESHOLD) mask |= BUTTONS.DOWN;
        }
        return mask;
    }

    hasGamepad() {
        const pads = navigator.getGamepads ? navigator.getGamepads() : [];
        return [...pads].some((pad) => pad && pad.connected);
    }
}
