# Builds the emulator for the browser into web/pkg.
# Needs the wasm-bindgen CLI at the same version as the wasm-bindgen crate in
# Cargo.lock:  cargo install wasm-bindgen-cli --version <that version>
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --release -p ps1-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg target/wasm32-unknown-unknown/release/ps1_wasm.wasm
Write-Host 'Built web/pkg. Serve the web/ folder, e.g.:  python -m http.server -d web 8080'
