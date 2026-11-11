# Contributing

```bash
cargo test
```

`cargo test` runs the unit tests, the golden-frame snapshots and the layer lint, with no PTY or socket involved.

```bash
cargo run -p shroud-smoke -- target/debug/shroud
```

The smoke test drives the real binary through PTYs, each section with its own isolated TMPDIR and server, all at once. It reads the frames back through libghostty-vt, so its checks look at the screen: it splits, types, reloads config, starts a layout, drives the scripting API and plugins, detaches, reattaches, restarts and exits. Name sections after the binary's path to run only those.

The decoders that read untrusted bytes have fuzz targets. Fuzzing needs nightly Rust and cargo-fuzz, but only for these targets; shroud itself builds on stable.

```bash
rustup toolchain install nightly && cargo install cargo-fuzz
```

```bash
cargo +nightly fuzz run -s none input_decode
```

The other targets are `protocol_message`, `shell_tracker`, `config_parse` and `api_request`.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first. Agents should start at [CLAUDE.md](CLAUDE.md).
