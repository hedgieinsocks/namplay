# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
make build      # debug build
make run        # build + compile GSettings schema to target/schemas/ + launch
make release    # optimized build
make install    # install to ~/.local (default PREFIX)
make clean      # cargo clean + remove target/schemas/
```

`make run` is the only way to launch the app in dev — it sets `GSETTINGS_SCHEMA_DIR` to point at the locally compiled schema. Running the binary directly without that env var will crash on GSettings init.

`build.rs` runs `blueprint-compiler` at build time to compile `data/window.blp` into `window.ui` (written to `OUT_DIR`). `blueprint-compiler` must be installed; if missing, the build fails immediately.

## Architecture

Two files, two threads:

**`src/main.rs`** — GTK4/Libadwaita UI thread. Builds the window from the compiled Blueprint UI, binds all widgets to `gio::Settings` keys, and constructs `AudioEngine` with initial settings values. After construction, a single `settings.connect_changed` closure routes every settings-key change to the corresponding `AudioEngine` method. The engine is kept alive inside that `'static` closure.

**`src/audio.rs`** — JACK real-time audio thread. `AudioEngine` owns the JACK async client. It communicates with the RT `NamProcessor` via two mechanisms:
- **`mpsc::channel`** — for heavyweight model/IR swaps (`load_model`, `load_pedal_model`, `load_ir` each spawn a background thread to load the file, then send the result).
- **`Arc<Atomic*>`** — for all scalar parameters (gains, EQ, gate threshold). Float values are stored as `u32` bits (`f32::to_bits` / `f32::from_bits`) to use `AtomicU32`.

Signal chain order inside `NamProcessor::process` (per buffer):
1. Noise gate (optional)
2. EQ at pos 0 = pre-pedal (optional)
3. Pedal NAM model (optional) with pedal in/out gain
4. EQ at pos 1 = pre-amp (optional)
5. Amp NAM model with in gain
6. IR convolution via `FFTConvolver` — stereo if WAV has 2 channels
7. EQ at pos 2 = post-IR (optional)
8. Output gain; right channel copied from left if IR is mono

Output is silenced (zeroed) when both pedal and amp models are absent.

## GSettings schema

All persistent state lives in `data/io.github.hedgieinsocks.Namplay.gschema.xml`. Adding a new setting requires: (1) add the key to the schema XML, (2) bind or connect it in `main.rs`, (3) wire up the `AudioEngine` method in `audio.rs` if it affects audio.

## UI layout

`data/window.blp` is a Blueprint file (GTK4 UI DSL). Edit this for any layout changes — do not edit generated `.ui` XML. Each section is an `Adw.ExpanderRow`: noise gate, EQ, pedal profile, amp profile, impulse response.
