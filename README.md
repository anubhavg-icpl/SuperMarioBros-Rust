SuperMarioBros-Rust
===================

A playable reimplementation of the original NES *Super Mario Bros.* in **pure Rust**.

Rust port by **Anubhav Gain**. See [LICENSE](LICENSE) for copyright and acknowledgements.

The game logic is a statically-recompiled version of the original 6502 machine code:
the `docs/smbdis.asm` disassembly (by doppelganger, doppelheathen@gmail.com) was
translated into Rust. The only NES subsystems that remain emulated are the PPU
(video) and APU (audio); the CPU is now native Rust.

Originally this was a C++ project (translated via a C++/flex/bison codegen tool).
The entire codebase has since been converted to Rust and the C++ has been removed.

![Demo gif](https://github.com/MitchellSternke/SuperMarioBros-C/raw/master/demo.gif)

Building
--------

**Dependencies**
- Rust (stable, 2021 edition)
- SDL2

On macOS: `brew install sdl2`. On Debian/Ubuntu: `apt install libsdl2-dev`.

```
cargo build --release
```

This produces `target/release/smbc`.

Running
-------

Requires an *unmodified* copy of the `Super Mario Bros. (JU) (PRG0) [!].nes` ROM
(the CHR data in the ROM is used for rendering — without it there are no graphics).
By default the program looks for this file in the working directory; the path can
be overridden in `smbc.conf`.

```
cargo run --release
```

**Controls:** Arrow keys = D-pad, `X` = A, `Z` = B, `Enter` = Start,
`Backspace` = Select, `R` = reset, `F` = fullscreen, `Esc` = quit.

**Pause and save states:**

| Key | Action |
| --- | --- |
| `P` | Pause / resume (the game also pauses when its window loses focus) |
| `1`–`9` | Select save slot (default: slot 1) |
| `F5` | Save state to the selected slot |
| `F9` | Load state from the selected slot |

States are written next to the ROM as `smbc-slot1.state` … `smbc-slot9.state`,
so they survive restarting the program. The window title shows the selected
slot, whether the game is paused, and the result of the last save/load.

Configuration
-------------

Optionally create a `smbc.conf` (INI format) in the working directory:

```
[audio]
frequency = 48000

[game]
rom_file = Super Mario Bros. (JU) (PRG0) [!].nes
frame_rate = 60

[video]
scale = 3
scanlines = 1
vsync = 1
palette_file =
```

See the original option docs; all keys and defaults are preserved.

Architecture
------------

- `src/generated/` — Rust translated from the 6502 disassembly:
  - `smb_code.rs` — the game logic, as a `Label`-dispatched state machine
    (6502 `goto`/`JSR`/`RTS` control flow becomes a `loop { match label { … } }`).
  - `smb_data.rs` — the game's constant data.
  - `smb_constants.rs` — address/value constants.
- `src/engine.rs` — NES CPU emulation: registers, status flags, RAM, the
  call-return stack, and the ALU/memory operations the generated code calls.
- `src/ppu.rs` — Picture Processing Unit (rendering).
- `src/apu.rs` — Audio Processing Unit (sound/music).
- `src/controller.rs` — controller input.
- `src/configuration.rs`, `src/video.rs`, `src/constants.rs`, `src/main.rs` —
  config, palette loading, and the SDL2 window/audio/game loop.

License
-------

The Rust port code is MIT-licensed, Copyright (c) 2026 Anubhav Gain. See
[LICENSE](LICENSE).

*Super Mario Bros.* is a trademark and copyright of Nintendo. This repository
contains no ROM data — supply your own legally-obtained copy to play.

Acknowledgements
----------------

- Original 6502 disassembly (`docs/smbdis.asm`) by doppelganger (doppelheathen@gmail.com).
- Original C++ *SuperMarioBros-C* project by Mitchell Sternke, from which this Rust port was derived.
