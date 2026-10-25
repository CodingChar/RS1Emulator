// Preferences that survive a reload, kept in localStorage.

const STORAGE_KEY = 'rs1emu.settings';

const DEFAULTS = { volume: 80, muted: false, speed: '1', slot: 1, aspect: '4:3', filter: 'nearest' };

export function loadSettings() {
    try {
        const saved = JSON.parse(localStorage.getItem(STORAGE_KEY));
        if (saved && typeof saved === 'object') return { ...DEFAULTS, ...saved };
    } catch (_) { /* nothing saved, or storage unavailable */ }
    return { ...DEFAULTS };
}

export function saveSettings(settings) {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch (_) { /* the preferences last for this session only */ }
}
