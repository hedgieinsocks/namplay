# agents.md

Agent reference for Namplay. Read this before touching any code.

## Project at a Glance

Namplay is a GTK4/Libadwaita desktop app for running Neural Amp Modeler (NAM) `.nam` profiles via PipeWire (JACK). It exposes a noise gate, 3-position EQ, optional pedal profile, amp profile, and impulse response (IR) convolution.

Two source files. Two threads. No more.

---

## File Map

```
src/main.rs          GTK4/Libadwaita UI thread
src/audio.rs         JACK real-time audio thread
data/window.blp      Blueprint UI definition (edit this, not generated XML)
data/io.github.hedgieinsocks.Namplay.gschema.xml   GSettings schema
build.rs             Calls blueprint-compiler at build time
Cargo.toml           Dependencies
Makefile             Build/run/install targets
```

---

## Architecture

### Thread 1: UI (`src/main.rs`)

- Builds window from Blueprint-compiled XML (`include_str!` at compile time via `OUT_DIR`).
- Reads `gio::Settings` on startup to construct `InitialParams`, then calls `AudioEngine::new`.
- Single `settings.connect_changed` closure routes every settings-key change to the matching `AudioEngine` method.
- `AudioEngine` is moved into that `'static` closure — that's how it stays alive.
- File picker rows use `gtk4::FileDialog` (async, callback-based).
- Reset buttons call `settings.reset(key)`, which triggers `connect_changed` automatically.

### Thread 2: Audio (`src/audio.rs`)

`AudioEngine` owns a JACK async client. The RT callback is `NamProcessor::process`.

Two IPC mechanisms between `AudioEngine` and `NamProcessor`:

| Mechanism | Used for |
|-----------|----------|
| `mpsc::channel` | Heavyweight swaps: model load, pedal model load, IR load |
| `Arc<Atomic*>` | All scalar params: gains, EQ bands, gate threshold, enable flags |

Float atomics use `AtomicF32` (a newtype over `AtomicU32` storing `f32::to_bits`). Ordering is `Relaxed` throughout — no synchronization needed beyond eventual consistency.

File loads (`load_model`, `load_pedal_model`, `load_ir`) each spawn a background thread to avoid blocking the UI. The loaded result is sent over the channel; `NamProcessor::process` drains the channel at the top of every callback.

---

## Signal Chain

Order inside `NamProcessor::process` per buffer:

1. **Noise gate** (optional) — envelope follower, attack 1ms, release 100ms, hold 50ms, hysteresis -6dB
2. **EQ at pos 0** (`pre-pedal`) — optional
3. **Pedal NAM model** (optional) — with pedal in/out gain applied as linear scale
4. **EQ at pos 1** (`pre-amp`) — optional, default position
5. **Amp NAM model** — with amp in gain
6. **IR convolution** via `FFTConvolver` — stereo if WAV has 2 channels, otherwise mono
7. **EQ at pos 2** (`post-IR`) — optional; applied to both L and R when stereo IR
8. **Output gain** — applied to L always, R copied from L if IR is mono

Output is **zeroed** when both pedal model and amp model are absent (no passthrough).

---

## EQ Details

`Eq` struct chains 5 biquad filters in series: HP → low shelf → mid peaking → high shelf → LP.

Fixed center frequencies (not user-configurable):
- Low shelf: 150 Hz
- Mid peaking: 425 Hz, Q=1.5 (cut) / Q=0.7 (boost)
- High shelf: 1800 Hz

User-configurable:
- HP cutoff: 20–120 Hz (default 70 Hz)
- LP cutoff: 5000–16000 Hz (default 7000 Hz)
- Low/mid/high gain: ±20 dB (default 0)

`NamProcessor` detects param changes by comparing atomics against `last_*` fields each callback. Only recreates or updates `Eq` when something changed.

---

## GSettings Schema

All persistent state in `data/io.github.hedgieinsocks.Namplay.gschema.xml`.

Schema ID: `io.github.hedgieinsocks.Namplay`

**Adding a new setting — all 3 steps required:**

1. Add key to the schema XML with type, default, range, summary.
2. Bind or connect it in `main.rs`:
   - Numeric/bool scalar → `bind_adjustment` / `bind_toggle` + `setup_reset_button`
   - String enum → manual `connect_toggled` pattern (see `setup_eq_position`)
   - Add to `InitialParams` struct and pass initial value to `AudioEngine::new`
   - Add a `connect_changed` arm in the big match block
3. Add method to `AudioEngine` + wire up the corresponding `Arc<Atomic*>` or channel in `audio.rs`.

Forgetting any step causes silent bugs (UI changes don't reach audio, or state doesn't persist).

---

## UI Layout (`data/window.blp`)

Blueprint file — GTK4 UI DSL. **Never edit the generated `.ui` XML.**

Structure: single `Adw.PreferencesPage` containing one `Adw.PreferencesGroup` with five `Adw.ExpanderRow`s:
- `noise_gate_row` — noise gate, has enable switch
- `eq_row` — EQ, has enable switch + position toggle buttons
- `pedal_profile_row` — pedal profile file + gains
- `amp_profile_row` — amp profile file + gains
- `ir_row` — IR file + level

Each spinner widget has a matching `Gtk.Adjustment` declared at the top of the `.blp` file. Reset buttons carry the `edit-undo-symbolic` icon. Clear buttons carry `edit-clear-symbolic`.

The `collapse-on-launch` menu toggle collapses all five rows on startup.

---

## Build System

`build.rs` runs `blueprint-compiler compile data/window.blp --output $OUT_DIR/window.ui` at build time. If `blueprint-compiler` is missing, the build fails immediately with a panic.

```sh
make build      # cargo build (debug)
make run        # build + compile schema to target/schemas/ + launch with GSETTINGS_SCHEMA_DIR set
make release    # cargo build --release
make install    # install to ~/.local (PREFIX)
make clean      # cargo clean + rm target/schemas/
```

**`make run` is mandatory for dev.** Running the binary directly crashes on GSettings init because `GSETTINGS_SCHEMA_DIR` is not set.

---

## Dependencies

| Crate | Role |
|-------|------|
| `gtk4` (v4_10) | GTK4 widgets |
| `libadwaita` (v1_4) | Adwaita widgets (`ExpanderRow`, `SpinRow`, etc.) |
| `gio`, `glib` | GSettings, GLib integration |
| `nam-rs` 0.3 | NAM model loading and inference (`NamModel`, `Model`) |
| `jack` 0.13 | JACK audio client and RT callback |
| `fft-convolver` 0.4 | FFT-based IR convolution (`FFTConvolver<f32>`) |
| `hound` 3.5 | WAV file reading (int and float formats, mono and stereo) |
| `log` + `env_logger` | Logging; set `RUST_LOG=debug` for verbose output |

Release profile enables LTO.

---

## Key Invariants

- `NamProcessor` is `!Send` — it lives entirely on the JACK RT thread. Never access it from the UI thread.
- All cross-thread scalar state goes through `Arc<Atomic*>`. Never add a `Mutex` for scalars.
- Model/IR loads always happen on a spawned thread, result sent via `mpsc`. Never load files in `process()`.
- `db_to_gain(db)` = `10^(db/20)` — convert before storing in atomics (linear gain, not dB).
- IR sample rate mismatch vs JACK rate logs a warning but does not fail — pitch will differ.
- Stereo IR: left channel convolves `out_l`, right convolves the original `conv_buf` copy. Both get IR level and out gain.
- Mono IR: `out_r.copy_from_slice(out_l)` after post-IR EQ and out gain on left.

---

## Logging

Enable with `RUST_LOG=debug namplay` (or `RUST_LOG=namplay=debug`).

Log points:
- JACK connection: sample rate + block size
- Every parameter change: value in dB or Hz
- Model/IR load start, success, failure
- Engine construction: all initial values

Errors (model load failure, JACK init failure) use `log::error`. Audio unavailability is non-fatal — the UI still works, just no sound.

---

## Common Pitfalls

- **Schema not compiled** — run `make run` not the binary directly.
- **`blueprint-compiler` missing** — install it (`dnf install blueprint-compiler` / `apt install blueprint-compiler`).
- **Adding UI widget without schema key** — settings binding silently does nothing.
- **Adding schema key without `connect_changed` arm** — live changes don't reach audio.
- **Editing `window.ui` directly** — it gets overwritten on next build.
- **IR stereo right channel bug** — the right FFTConvolver processes `conv_buf` (pre-amp copy), not `out_l` (post-amp). That's intentional — both channels share the same pre-amp signal.
- **EQ position integers** — 0=pre-pedal, 1=pre-amp (default), 2=post-IR. The string "pre-amp" maps to 1 via the `_ =>` arm in both `main.rs` and `audio.rs`.
