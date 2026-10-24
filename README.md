# RS1Emu — PlayStation 1 Emulator in Rust + WebAssembly

An open-source Sony PlayStation (PS1) emulator written in Rust, compiled to WebAssembly
so it runs directly in a web browser.

**Architecture reference:** [DuckStation](https://github.com/stenzek/duckstation)
(used only for design reference — no code is copied; DuckStation is licensed under
CC BY-NC-ND 4.0, which is not a free/libre license).

## Legal notice

The PS1 BIOS and game discs are copyrighted material. This emulator does **not**
include a BIOS or any games. You must provide your own BIOS dump (from your own
console) and game backups.

## License

MIT — see `LICENSE`.

## Building

Requires Rust stable with the `wasm32-unknown-unknown` target:

```bash
rustup target add wasm32-unknown-unknown
cargo build --release
```

For the web build (using [Trunk](https://trunkrs.dev/)):

```bash
cargo install trunk
trunk build --release --dir dist
```

Then serve `dist/` with any static file server.

## Testing

`.cargo/config.toml` builds for `wasm32-unknown-unknown` by default, so run the
host tests with an explicit target (use your own host triple):

```bash
cargo test -p ps1-core --target x86_64-pc-windows-msvc
```

### Test ROMs and BIOS

Game discs and BIOS dumps are copyrighted and are **never committed**: the
`rom_test/` folder, disc images (`*.bin`, `*.cue`, `*.iso`, ...), BIOS dumps
and memory cards are listed in `.gitignore`. To test against a real disc, put
it under `rom_test/`; `tests/disc_image.rs` uses it when present and skips
itself otherwise.

The boot harness inspects a disc and runs a BIOS you supply:

```bash
cargo run -p ps1-core --example boot --target x86_64-pc-windows-msvc -- \
    --disc "rom_test/<game>/<game>.cue" --bios path/to/bios.bin --steps 20000000
```

## Roadmap

See [ROADMAP.md](ROADMAP.md) for the pending work, ordered by priority.

## Project structure

```
crates/
├── ps1-core/      # Pure Rust emulation core (testable on host)
├── ps1-wasm/      # wasm-bindgen glue for the browser
└── ps1-web/       # Frontend (HTML + Rust/WASM)
```
