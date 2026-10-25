// Widgets of the page: the full-screen style menu (list of rows driven by
// keyboard, gamepad or mouse), the gamepad navigation and the on-screen messages.

const TOAST_LIFETIME_MS = 3200;
const MAX_TOASTS = 4;

/** Short messages shown over the picture, like an emulator's on-screen display. */
export class Toasts {
    constructor(container) {
        this.container = container;
    }

    show(text, isError = false) {
        const toast = document.createElement('div');
        toast.className = isError ? 'toast error' : 'toast';
        toast.textContent = text;
        this.container.append(toast);
        while (this.container.children.length > MAX_TOASTS) this.container.firstChild.remove();
        setTimeout(() => {
            toast.classList.add('leaving');
            setTimeout(() => toast.remove(), 300);
        }, TOAST_LIFETIME_MS);
    }
}

/**
 * A stack of pages. A page is `{ title, subtitle?, items() }` and `items()` returns
 * rows `{ label, icon?, value?(), disabled?, accept?(), left?(), right?(), separator? }`.
 * It is rebuilt after every action, so rows always show the current state.
 */
export class MenuUI {
    constructor(root) {
        this.root = root;
        this.stack = [];
        this.index = 0;
        this.onClose = null;

        root.innerHTML = `
            <div class="fs-panel" role="dialog" aria-modal="true">
                <header class="fs-header">
                    <div class="fs-title"></div>
                    <div class="fs-subtitle"></div>
                </header>
                <ul class="fs-list" role="menu"></ul>
                <footer class="fs-footer"></footer>
            </div>`;
        this.titleElement = root.querySelector('.fs-title');
        this.subtitleElement = root.querySelector('.fs-subtitle');
        this.list = root.querySelector('.fs-list');
        this.footer = root.querySelector('.fs-footer');
        // A click on the dimmed background closes the menu.
        root.addEventListener('mousedown', (event) => {
            if (event.target === root) this.back(true);
        });
    }

    get isOpen() {
        return this.stack.length > 0;
    }

    get page() {
        return this.stack[this.stack.length - 1];
    }

    open(page, onClose) {
        this.stack = [page];
        this.onClose = onClose;
        this.index = this.nextEnabled(-1, 1);
        this.root.hidden = false;
        this.render();
    }

    push(page) {
        this.stack.push(page);
        this.index = this.nextEnabled(-1, 1);
        this.render();
    }

    /** Go up one page; from the first page, close the menu. */
    back(forceClose = false) {
        if (this.stack.length > 1 && !forceClose) {
            this.stack.pop();
            this.index = 0;
            this.index = this.nextEnabled(-1, 1);
            this.render();
        } else {
            this.close();
        }
    }

    close() {
        if (!this.isOpen) return;
        this.stack = [];
        this.root.hidden = true;
        const callback = this.onClose;
        this.onClose = null;
        if (callback) callback();
    }

    items() {
        return this.page ? this.page.items() : [];
    }

    selectable(item) {
        return !item.separator && !item.disabled;
    }

    /** Index of the next selectable row after `from` in direction `step` (wraps around). */
    nextEnabled(from, step) {
        const items = this.items();
        if (items.length === 0) return 0;
        let index = from;
        for (let tries = 0; tries < items.length; tries++) {
            index = (index + step + items.length) % items.length;
            if (this.selectable(items[index])) return index;
        }
        return Math.max(0, from);
    }

    /** React to a navigation event: up, down, left, right, accept or back. */
    press(kind) {
        if (!this.isOpen) return;
        const item = this.items()[this.index];
        switch (kind) {
            case 'up': this.index = this.nextEnabled(this.index, -1); break;
            case 'down': this.index = this.nextEnabled(this.index, 1); break;
            case 'left': if (item && item.left && !item.disabled) item.left(); break;
            case 'right': if (item && item.right && !item.disabled) item.right(); break;
            case 'accept':
                if (item && this.selectable(item)) {
                    if (item.accept) item.accept();
                    else if (item.right) item.right();
                }
                break;
            case 'back': this.back(); break;
        }
        if (this.isOpen) this.render();
    }

    render() {
        const page = this.page;
        if (!page) return;
        this.titleElement.textContent = page.title;
        this.subtitleElement.textContent = page.subtitle ? page.subtitle() : '';
        const items = this.items();
        if (this.index >= items.length || !this.selectable(items[this.index])) {
            this.index = this.nextEnabled(-1, 1);
        }
        this.list.replaceChildren(...items.map((item, index) => this.renderRow(item, index)));
        const selected = this.list.children[this.index];
        if (selected && selected.scrollIntoView) selected.scrollIntoView({ block: 'nearest' });
        const back = this.stack.length > 1 ? 'Atrás' : 'Cerrar';
        this.footer.innerHTML = `<span><kbd>↑↓</kbd> Mover</span><span><kbd>←→</kbd> Cambiar</span>` +
            `<span><kbd>Intro</kbd> / <kbd>✕</kbd> Elegir</span><span><kbd>Esc</kbd> / <kbd>○</kbd> ${back}</span>`;
    }

    renderRow(item, index) {
        const row = document.createElement('li');
        if (item.separator) {
            row.className = 'fs-separator';
            row.textContent = item.label || '';
            row.setAttribute('role', 'separator');
            return row;
        }
        row.className = 'fs-row';
        row.setAttribute('role', 'menuitem');
        if (item.disabled) row.classList.add('disabled');
        if (index === this.index) row.classList.add('selected');

        const icon = document.createElement('span');
        icon.className = 'fs-icon';
        icon.textContent = item.icon || '';
        const label = document.createElement('span');
        label.className = 'fs-label';
        label.textContent = item.label;
        row.append(icon, label);

        if (item.value || item.left || item.right) {
            const value = document.createElement('span');
            value.className = 'fs-value';
            if (item.left) value.append(this.arrow('◀', item, 'left'));
            const text = document.createElement('span');
            text.className = 'fs-value-text';
            text.textContent = item.value ? item.value() : '';
            value.append(text);
            if (item.right) value.append(this.arrow('▶', item, 'right'));
            row.append(value);
        } else if (item.submenu) {
            const chevron = document.createElement('span');
            chevron.className = 'fs-value';
            chevron.textContent = '›';
            row.append(chevron);
        }

        row.addEventListener('mousemove', () => {
            if (this.index === index || item.disabled) return;
            this.list.children[this.index]?.classList.remove('selected');
            this.index = index;
            row.classList.add('selected');
        });
        row.addEventListener('click', () => {
            this.index = index;
            this.press('accept');
        });
        return row;
    }

    arrow(glyph, item, side) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'fs-arrow';
        button.tabIndex = -1;
        button.textContent = glyph;
        button.addEventListener('click', (event) => {
            event.stopPropagation();
            this.index = this.items().indexOf(item);
            this.press(side);
        });
        return button;
    }
}

const NAV_REPEAT_DELAY_MS = 380;
const NAV_REPEAT_INTERVAL_MS = 110;

/** Turns the gamepads' merged navigation state into menu events (with key repeat for directions). */
export class PadNavigator {
    constructor() {
        this.held = new Map();
    }

    /** Events (`up`, `down`, `left`, `right`, `accept`, `back`) produced since the last call. */
    poll(pressed, now) {
        const events = [];
        for (const [name, down] of Object.entries(pressed)) {
            const state = this.held.get(name);
            if (!down) {
                this.held.delete(name);
            } else if (!state) {
                this.held.set(name, { next: now + NAV_REPEAT_DELAY_MS });
                events.push(name);
            } else if (now >= state.next && name !== 'accept' && name !== 'back') {
                state.next = now + NAV_REPEAT_INTERVAL_MS;
                events.push(name);
            }
        }
        return events;
    }
}
