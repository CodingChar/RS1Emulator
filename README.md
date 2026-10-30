# RS1Emu — PlayStation 1 Emulator in Rust + WebAssembly

An open-source Sony PlayStation (PS1) emulator written in Rust, compiled to WebAssembly
so it runs directly in a web browser (and natively, through the same core).

**Try it:** <https://codingchar.github.io/RS1Emulator/> — bring your own BIOS and games;
nothing leaves your browser.

[![CI](https://github.com/CodingChar/RS1Emulator/actions/workflows/ci.yml/badge.svg)](https://github.com/CodingChar/RS1Emulator/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-e8955c.svg)](LICENSE)

**Architecture reference:** [DuckStation](https://github.com/stenzek/duckstation)
(used only for design reference — no code is copied; DuckStation is licensed under
CC BY-NC-ND 4.0, which is not a free/libre license). Hardware details come from the
public [psx-spx](https://problemkaputt.de/psx-spx.htm) documentation.

## Legal notice

The PS1 BIOS and game discs are copyrighted material. This emulator does **not**
include a BIOS or any games. You must provide your own BIOS dump (from your own
console) and game backups. Nothing is uploaded anywhere: files stay in your browser.

## License

MIT — see `LICENSE`.

## Running it in the browser

Requires Rust stable with the `wasm32-unknown-unknown` target and the `wasm-bindgen`
command-line tool at the same version as the `wasm-bindgen` crate in `Cargo.lock`:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <version from Cargo.lock>
sh web/build.sh            # or: powershell web/build.ps1
python -m http.server -d web 8080
```

Open <http://localhost:8080> and give it your BIOS (512 KB, kept in IndexedDB) and a
disc: a `.cue` with its `.bin` files, a single `.bin`, an `.iso`, or a PS-EXE program.

| | |
|---|---|
| Disc | read from disk on demand (a 650 MB disc costs no memory); "Cambiar disco" swaps it while the game runs |
| Controls | keyboard (rebindable) and USB or Bluetooth gamepads (per-pad port, button map, dead zone and rumble test in Settings → Mandos); by default the second gamepad drives port 2 |
| Analog | F9 or the quick menu toggles the DualShock analog mode; vibration uses the Gamepad API |
| Interface | menu bar, toolbar and a settings dialog; a see-through quick menu (Esc, F1 or the gamepad Home button; Alt+Enter for full screen) with quick save/load, volume, aspect, scaling, PGXP, overlay, speed and analog mode, navigable by keyboard, gamepad or mouse; keyboard shortcuts that work in full screen (F2 overlay, F3 aspect, F4 scaling, F5/F7 quick save/load, F6 slot, F8 mute, F10 pause, PgUp/PgDn volume); on-screen messages |
| Picture | aspect ratio 4:3, 16:9, zoom-to-cover without stretching, whole-number scale or fill; border crop; nearest or smooth scaling |
| Rendering | internal resolution 1x (default), 2x, 3x; PGXP (sub-pixel geometry and perspective-correct textures); bilinear texture filtering with binary alpha; true colour (24-bit); a widescreen option for 3D scenes (see below); adaptive quality, which steps the options above down by itself while the game cannot keep up and back up when there is room |
| Overlay | optional performance panel: FPS, speed, frame time, CPU load, resolution and renderer, audio buffer, memory |
| Audio | steady audio reserve with dynamic rate control and fades instead of cuts; automatic or fixed latency; smooth volume |
| Saves | memory cards 1 and 2 persist in IndexedDB (export, import of raw dumps, DexDrive `.gme` and PSP/PS3 `.vmp`, format); three save-state slots |
| Speed | ×1, ×2, ×4 or maximum; volume, mute and speed are remembered |
| Discs | `.cue`/`.bin` (with `PREGAP`, `POSTGAP` and `INDEX 00`), `.iso`, CloneCD `.ccd`/`.img`, optional `.sbi` |
| Timing | multiply/divide and GTE latencies, RAM read latency, GPU busy time, DMA durations and chopping, CD seek time by distance, timers 0/1 synchronised with the blanking |
| Guide | [docs/guia-de-usuario.md](docs/guia-de-usuario.md) (Spanish) |

The emulator runs in a Web Worker (`web/worker.js`): it draws into a transferred
`OffscreenCanvas` straight from the module's memory and sends audio to an
`AudioWorklet` through a message channel, so the page's main thread only handles the UI.

### How the rendering options work

The console's 1024x512 VRAM stays the source of truth for everything a game can see, and its
rasterizer is untouched (with every option off the picture is the console's, pixel for pixel).
When an option needs it, every drawing command is drawn a second time into a *scaled VRAM*
(`ps1-core/src/gpu/hires.rs`): `scale` times larger on both axes and 8 bits per channel, used only
for the picture on screen.

- **Internal resolution.** The scaled rasterizer uses the same edge rules and the same exact
  interpolation as the console's (tests draw random primitives and require identical pixels at 1x,
  and identical pixels on the console's sample points at 2x and 3x). Fills, uploads and copies are
  mirrored, so what was drawn at high resolution moves with a VRAM copy.
- **PGXP** (`ps1-core/src/pgxp.rs`, the technique of Beetle PSX and DuckStation's "memory" mode). `RTPS`/`RTPT`
  keep the unrounded screen position and depth next to each SXY FIFO entry; the tag follows the value
  through `swc2`, `mfc2`, `lw`, `sw`, `lwc2` and `mtc2` and RAM, checked against the integer value it was
  made for; when a polygon reaches the GPU by DMA each vertex word is looked up by the address it came
  from. A polygon whose vertices all have a tag that agrees with the integer position (the GTE rounds
  down and clamps to 11 bits) is drawn at the precise positions, so big polygons that cross the edge of
  the screen keep their shape; one with any untagged vertex (computed by the CPU) is drawn with integers,
  so neighbouring polygons never open seams.
  With a depth on every vertex the textures are perspective-correct (`u/w`, `v/w`, `1/w` interpolated).
- **Widescreen.** With PGXP the horizontal position of fully tracked 3D polygons is scaled by 3/4 around
  the centre of the drawing area; the page shows the picture at 16:9. 2D elements are stretched, and games
  that cull polygons outside the 4:3 view leave gaps at the sides (as DuckStation's widescreen hack does).
- **Texture filtering.** Bilinear sampling of polygons (sprites stay sharp), with texels clamped to the
  primitive's texture area and transparent texels left out of the average (binary alpha).

The cost is the number of pixels drawn: 2x is 4 times and 3x is 9 times the rasterizing work.
`cargo run --release -p ps1-core --example raster_bench` measures it per kind of triangle.

## Testing

`.cargo/config.toml` builds for `wasm32-unknown-unknown` by default, so run the
host tests with an explicit target (use your own host triple):

```bash
cargo test -p ps1-core --target x86_64-pc-windows-msvc
```

CI (`.github/workflows/ci.yml`) runs the same tests on Linux and builds the wasm module. The page's
pure code (aspect-ratio layout, settings, and the audio player's reserve, rate control and fades,
simulated with a jittery producer and clock drift) has tests that run with Node:

```bash
node --test web/layout.test.mjs web/audio-worklet.test.mjs web/quality.test.mjs
```

### Test ROMs and BIOS

Game discs and BIOS dumps are copyrighted and are **never committed**: the
`rom_test/` folder, disc images (`*.bin`, `*.cue`, `*.iso`, ...), BIOS dumps
and memory cards are listed in `.gitignore`. To test against a real disc, put
it under `rom_test/`; `tests/disc_image.rs` uses it when present and skips
itself otherwise.

The boot harness inspects a disc and runs a BIOS you supply (screenshots, VRAM dumps,
audio recording, disassembly):

```bash
cargo run --release -p ps1-core --example boot --target x86_64-pc-windows-msvc -- \
    --disc "rom_test/<game>/<game>.cue" --bios path/to/bios.bin --steps 20000000
```

To get a game to a given screen it can also press buttons at chosen CPU cycles
(`--press 0x4000@1900000000-1970000000`), plug memory cards (`--card 1=card.mcr`), run a
PS-X EXE (`--exe`), and log the kernel calls (`--bios-call B0:4F`), the arguments at an
address (`--log-pc`) or the interrupt status (`--irq-log`); the header of
`crates/ps1-core/examples/boot.rs` lists them all.

Three tests need the BIOS (and, for two of them, a disc) under `rom_test/` and are ignored by
default because they boot it: the games reach their title or memory card screens, and the BIOS
memory card manager copies a save between cards. Run them in release mode:

```bash
cargo test --release -p ps1-core --target x86_64-pc-windows-msvc \
    --test boot_games --test memory_card_bios -- --ignored
```

### Benchmarks and equivalence checks

`examples/bench.rs` measures speed and prints a checksum of the whole machine state, so a
change that should only make things faster can be checked to behave identically:

```bash
cargo run --release -p ps1-core --example bench --target x86_64-pc-windows-msvc -- \
    --bios path/to/bios.bin [--disc "game.cue"] --steps 300000000
```

| Option | What it does |
|---|---|
| `--synthetic alu\|mem\|branch` | times endless loops: the interpreter on its own |
| `--press MASK@FIRST-LAST` | holds buttons between two instructions (to get into a game) |
| `--frames N` | runs whole video frames, like the web frontend |
| `--roundtrip FIRST,SECOND` | saves a state after FIRST instructions and checks that a fresh machine restored from it ends identical to the original after SECOND more |
| `--features profile` (cargo) | prints the share of time per area (CPU, GPU, SPU, MDEC...) |
| `--scale N`, `--pgxp`, `--pgxp-textures`, `--bilinear`, `--true-color`, `--widescreen` | the rendering options of the web page |
| `--shot FILE.png` | writes the final picture as those options render it, to look at a run |

`examples/raster_bench.rs` times the console's rasterizer and the scaled one per pixel for flat,
Gouraud, textured and semi-transparent triangles, bilinear and perspective-correct included.

Speed numbers vary ±10 % with the machine's temperature: compare builds by running them
alternately, not one after the other.

## Roadmap

See [ROADMAP.md](ROADMAP.md): current state, what is pending, the measurements behind each
optimization and the decisions that were rejected with a technical reason.

## Project structure

```
crates/
├── ps1-core/      # Pure Rust emulation core (testable on host)
│   ├── cpu/       #   interpreter (exact) and its fast path
│   ├── gpu/       #   GP0/GP1, the software rasterizer and the scaled one (hires.rs)
│   ├── pgxp.rs    #   PGXP: precise vertices kept next to the GTE results
│   ├── spu/       #   voices, ADSR, reverb, Gaussian interpolation
│   └── ...        #   bus, dma, timers, cdrom, mdec, gte, controllers, save_state, xa, exe
└── ps1-wasm/      # wasm-bindgen glue for the browser
web/               # The page and the worker (plain JavaScript modules)
```
