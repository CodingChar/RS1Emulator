// The performance panel drawn over the picture: frames per second and the other
// figures the user switched on. It only reads what the emulator and the audio
// thread report; hiding it costs nothing.

const MB = 1024 * 1024;

export class PerfOverlay {
    constructor(element) {
        this.element = element;
        this.options = null;
        this.lines = [];
    }

    /** Apply the user's choices (`settings.overlay`). */
    configure(options) {
        this.options = options;
        this.element.hidden = !options.enabled;
        this.element.setAttribute('aria-hidden', String(!options.enabled));
        for (const position of ['top-left', 'top-right', 'bottom-left', 'bottom-right']) {
            this.element.classList.toggle(position, options.position === position);
        }
        if (!options.enabled) this.element.replaceChildren();
    }

    get enabled() {
        return !!(this.options && this.options.enabled);
    }

    /**
     * Redraw. `info` has what the worker reports (`fps`, `full`, `emuAverage`, ...), what
     * the audio thread reports (`audio`), the emulation `speed` as a number (or null at
     * maximum speed) and the rendering options in force (`render`).
     */
    update(info) {
        if (!this.enabled) return;
        const lines = [];
        const options = this.options;
        const worker = info.worker;
        const running = !!worker && info.running;

        if (options.fps) {
            const fps = running ? worker.fps : 0;
            lines.push({ text: `${fps.toFixed(1)} FPS`, level: running && worker.fps < worker.full * 0.9 * (info.speed || 1) ? 'warn' : '' });
        }
        if (options.speed) {
            const percent = running ? Math.round((worker.fps / worker.full) * 100) : 0;
            lines.push({ text: `Velocidad ${percent} %`, level: running && info.speed && percent < 95 * info.speed ? 'warn' : '' });
        }
        if (options.frameTime) {
            if (running) {
                const total = worker.emuAverage + worker.presentAverage;
                lines.push({
                    text: `Cuadro ${total.toFixed(1)} ms (emu ${worker.emuAverage.toFixed(1)} + imagen ${worker.presentAverage.toFixed(1)}) · pico ${worker.intervalWorst.toFixed(0)} ms`,
                    level: '',
                });
            } else {
                lines.push({ text: 'Cuadro —', level: '' });
            }
        }
        if (options.load) {
            if (running && info.speed) {
                const budget = 1000 / worker.full / info.speed;
                const load = Math.round(((worker.emuAverage + worker.presentAverage) / budget) * 100);
                lines.push({ text: `Carga ${load} %`, level: load > 95 ? 'bad' : load > 75 ? 'warn' : '' });
            } else {
                lines.push({ text: 'Carga —', level: '' });
            }
        }
        if (options.resolution) {
            if (worker && worker.width > 0) {
                const render = info.render;
                const tags = [];
                if (render.scale > 1) tags.push(`${render.scale}×`);
                if (render.pgxp) tags.push(render.pgxpTextures ? 'PGXP + perspectiva' : 'PGXP');
                if (render.bilinear) tags.push('bilineal');
                if (render.trueColor) tags.push('24 bits');
                if (render.widescreen) tags.push('ancho');
                if (render.smooth2d) tags.push('suavizado 2D');
                if (worker.quality > 0) tags.push('calidad adaptada');
                const native = info.native ? `${info.native.width}×${info.native.height}` : '';
                const out = `${worker.width}×${worker.height}`;
                lines.push({ text: `Imagen ${native && native !== out ? `${native} → ${out}` : out}${tags.length ? ` · ${tags.join(' · ')}` : ''}`, level: '' });
            } else {
                lines.push({ text: 'Imagen —', level: '' });
            }
        }
        if (options.audio) {
            const audio = info.audio;
            if (info.speed !== 1) {
                lines.push({ text: 'Audio sin sonido a esta velocidad', level: '' });
            } else if (audio) {
                const state = audio.playing ? `${audio.buffered.toFixed(0)} ms` : 'llenando…';
                lines.push({
                    text: `Audio ${state} (meta ${audio.target.toFixed(0)}${audio.auto ? ' auto' : ''}) · ${audio.underruns} cortes${audio.overruns ? ` · ${audio.overruns} saltos` : ''}`,
                    level: audio.playing ? '' : 'warn',
                });
            } else {
                lines.push({ text: 'Audio desactivado', level: '' });
            }
        }
        if (options.memory) {
            lines.push({ text: worker ? `Memoria ${(worker.heap / MB).toFixed(0)} MB` : 'Memoria —', level: '' });
        }
        this.draw(lines);
    }

    draw(lines) {
        const previous = this.lines;
        const same = previous.length === lines.length && previous.every((line, i) => line.text === lines[i].text && line.level === lines[i].level);
        if (same) return;
        this.lines = lines;
        this.element.replaceChildren(
            ...lines.map((line) => {
                const row = document.createElement('div');
                row.className = line.level ? `perf-line ${line.level}` : 'perf-line';
                row.textContent = line.text;
                return row;
            }),
        );
    }
}

