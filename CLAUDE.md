# shroud: guide for agents

The map is [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): layers, the keystroke and output traces, the glossary, and a "where does my change go?" table. The design and its decisions, with the reasoning, are in [DESIGN.md](DESIGN.md). This file only covers how to work here.

## Build and test

On macOS, Homebrew's rustup is keg-only, so put it on the PATH first:

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
```

- `cargo build` builds everything. The first run builds libghostty-vt with Zig 0.17.x.
- `cargo test` runs the unit tests, the golden snapshots (insta) and the layer lint. It must stay fast and must never spawn a PTY or open a socket.
- `cargo insta review` accepts golden-frame changes. Review the escape bytes; don't accept blindly.
- `cargo clippy --all-targets` must stay at zero warnings, and `cargo fmt --check` clean (`rustfmt.toml` sets the style; `fuzz/` is formatted too, and the generated `crates/vt/src/ffi.rs` is skipped).
- `cargo run -p shroud-smoke -- target/debug/shroud` is the end-to-end check (`crates/smoke`). Run it after touching `crates/shroud`; name sections after the path to run only those, e.g. `-- target/debug/shroud plugins`. Its checks read the screen through libghostty-vt and wait for a condition, never a fixed time.
- `cargo +nightly fuzz run -s none <target>` fuzzes a decoder; `cargo fuzz list` names the targets. Nightly is for `fuzz/` only and never touches the main workspace. Run the affected target after changing `input/decode.rs`, `protocol/message.rs`, `shell/tracker.rs`, `protocol/api.rs` or anything in `config/`, and turn any crash into a unit test.

## Layer rules (enforced)

- `shroud-core` (`crates/core`) is pure: no I/O, no `unsafe`, no C. The crate forbids `unsafe` outright.
- Only `shroud-vt` (`crates/vt`) calls libghostty-vt. Add new C API use there, behind a safe type.
- Only `shroud-plugins` (`crates/plugins`) links LuaJIT, and its Lua runs only in the `shroud __plugins` process. Never run Lua in the server: a JIT-compiled loop can't be interrupted.
- Within core, `crates/core/tests/layers.rs` lists which module may import which. Update its `ALLOWED` table deliberately, never to silence it.
- Colour values (`Color::Palette`, `Color::Rgb`, raw SGR colour codes) appear only in `theme.rs` and `render/sgr.rs`. New chrome gets a new theme token with an ANSI-only default.
- Pane content is never recoloured.

## Conventions

- One concept per file, named in the domain's words. Split any file past about 400 lines.
- Each file opens with a short `//!` saying what it must not do or why it exists. Comments are terse and explain why, not what.
- Public items come first. Tests go at the bottom of the file in `#[cfg(test)] mod tests`; the layer lint relies on that.
- When code changes, the docs it touches change in the same diff: ARCHITECTURE.md for structure and paths, DESIGN.md for decisions and the reasoning behind them (add a row to its decision record for a new one).
- Never put "ghostty" in shroud's own names.
- Don't commit unless asked: the maintainer reviews and commits changes.

## Updating libghostty-vt

1. Bump `VERSION` and `SHA256` in `crates/vt/build.rs`. Take `SHA256` from the release's signed `SHA256SUMS`.
2. Regenerate the bindings only if you need new API: `scripts/gen-bindings.sh <prefix>`, where `<prefix>` is `target/libvt-<version>/prefix`. A 1.x upgrade never requires it.
3. Zig's version is pinned by libghostty-vt's `build.zig.zon` (`minimum_zig_version`).

## Releasing

1. Bump `version` in the root `Cargo.toml` and run `cargo build` so `Cargo.lock` follows.
2. After that change is merged, tag `main` as `v<version>` and push the tag. `.github/workflows/release.yml` checks the tag matches, builds every target, and publishes the release.
3. Releases are immutable once published, so try a release first with the workflow's manual run (`gh workflow run release`), which builds the same artifacts without publishing.
