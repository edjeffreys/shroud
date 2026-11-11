# Installing and building

With [Homebrew](https://brew.sh), on macOS or Linux. Homebrew asks you to trust a formula from a third-party tap before it loads it:

```bash
brew trust --formula edjeffreys/tap/shroud
brew install edjeffreys/tap/shroud
```

For the newest `main` instead, install `edjeffreys/tap/shroud-dev` (prebuilt whenever CI passes on `main`; trust it the same way), or `brew install --HEAD edjeffreys/tap/shroud` to build it from source. `shroud` and `shroud-dev` both install `shroud`, so have one at a time. In a Brewfile:

```ruby
tap "edjeffreys/tap"
brew "edjeffreys/tap/shroud", trusted: true   # or "edjeffreys/tap/shroud-dev"
```

Without Homebrew, each [release](https://github.com/edjeffreys/shroud/releases) has a prebuilt binary for macOS (Apple silicon and Intel) and Linux (x86_64 and arm64, glibc 2.35 or newer), with a SHA-256 checksum for each:

```bash
target=aarch64-apple-darwin   # or x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
version=0.1.0
curl -LO "https://github.com/edjeffreys/shroud/releases/download/v${version}/shroud-${version}-${target}.tar.gz"
tar -xzf "shroud-${version}-${target}.tar.gz"
install "shroud-${version}-${target}/shroud" ~/.local/bin/   # or anywhere on your PATH
```

To try the newest `main`, the [`dev` prerelease](https://github.com/edjeffreys/shroud/releases/tag/dev) is rebuilt whenever CI passes on it, at fixed links:

```bash
curl -LO "https://github.com/edjeffreys/shroud/releases/download/dev/shroud-dev-${target}.tar.gz"
```

`shroud --version` says which commit a dev build is, such as `0.2.0-dev.12+g3f9c2ab`. Or build it yourself.

## Building

You need Rust (stable) and Zig 0.17.x. Zig only builds libghostty-vt.

```bash
# macOS
brew install rustup zig && rustup default stable
# elsewhere: Rust from https://rustup.rs, Zig 0.17 from https://ziglang.org/download/
```

```bash
cargo build --release
```

The first build downloads the libghostty-vt 1.0.0 source tarball, checks its SHA-256, and builds it with Zig, which takes about 40 seconds on an M-series Mac. Later builds reuse it from `target/`.

To build offline or against a system package, set `SHROUD_LIBVT_DIR` to a prefix containing `lib/libghostty-vt.a`.

shroud is developed and tested in Ghostty. Other terminals that speak the kitty keyboard protocol should work, but some features, such as loading shell integration into panes, rely on Ghostty.
