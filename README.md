# zx-wasm

A ZX Spectrum 48K emulator written in Rust, compiled to WebAssembly and run in the browser.

## Features

- Z80 CPU running at 3.5 MHz with accurate T-state timing
- 320×240 display (rendered at 2× for a 640×480 canvas) with border colour support
- Beeper audio output at 44100 Hz via the Web Audio API
- Load the 48K ROM image (`spec48.rom`) or `.SNA` snapshot files
- Keyboard mapped to the ZX Spectrum layout, including cursor keys via CAPS SHIFT

## Build

Requires [Rust](https://rustup.rs/) and [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/).

```sh
wasm-pack build --target web --out-dir www/pkg
```

## Run locally

Requires [uv](https://docs.astral.sh/uv/).

```sh
cd www
uv run python -m http.server
```

Then open `http://localhost:8000` in your browser, load `spec48.rom` when prompted, and optionally load a `.SNA` snapshot to jump straight into a game.
