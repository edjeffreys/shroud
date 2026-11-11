<div align="center">

# shroud

**A terminal multiplexer for Ghostty, fast enough to forget it's there.**

[![CI](https://github.com/edjeffreys/shroud/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/edjeffreys/shroud/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/edjeffreys/shroud?sort=semver)](https://github.com/edjeffreys/shroud/releases/latest)
[![Homebrew](https://img.shields.io/badge/brew-edjeffreys%2Ftap%2Fshroud-orange?logo=homebrew)](#install)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)
[![Licence: MIT](https://img.shields.io/badge/licence-MIT-blue)](LICENSE)

</div>

Sessions, tabs, splits and floating panes that outlive your terminal window, built on [libghostty-vt](https://github.com/ghostty-org/ghostty), the engine inside [Ghostty](https://ghostty.org). shroud is made for one modern terminal on your own machine, not every terminal on every server, so it can lean on everything that terminal does well.

## Install

```bash
brew trust --formula edjeffreys/tap/shroud
brew install edjeffreys/tap/shroud
```

Prebuilt binaries for macOS and Linux, rolling `dev` builds of `main` and building from source: **[docs/install.md](docs/install.md)**.

## 30 seconds in

```text
shroud                  attach to your last session, or start one
Ctrl+Space  v / s       split right / below
Ctrl+Space  h j k l     move between panes
Ctrl+Space  f           pop up a floating pane
Ctrl+Space  ;           fuzzy-switch sessions
Ctrl+Space  y           copy the last command's output
Ctrl+Space  d           detach; your shells keep running
```

After the leader, keys are nvim's `CTRL-W` window commands. Forget one? Pause after `Ctrl+Space` and a hint box shows what comes next.

## Why shroud

- **Fast.** A 200 MB `cat` takes 1.87 s inside shroud and 1.57 s without it. Typing stays under a millisecond even while another pane floods. Damage-tracked frame diffing, synchronized output, backpressure instead of queued frames. [The numbers →](docs/performance.md)
- **Shell-aware.** Jump between prompts, copy a command's output with one key, open splits where you already are, and see a ✗ the moment a command fails in another pane.
- **Matches Ghostty.** Panes are parsed by Ghostty's own library, so graphemes, the kitty keyboard protocol, links and images behave the same inside shroud as outside.
- **Follows your theme.** By default shroud draws only with your terminal's palette, light or dark. Every colour is a semantic token you can override.
- **Scriptable to the core.** A JSON control socket, an event stream, and Lua plugins pinned in a lockfile. `shroud pane run --wait 'cargo test'` opens a pane, runs your tests and exits with their code: made for scripts and agents.
- **Hard to break.** A crashed server leaves your terminal intact, a config typo costs only that one setting, and every decoder of untrusted input is fuzzed.

tmux and Zellij shaped shroud: tmux's scripting model and copy mode, Zellij's floating and command panes. shroud makes different trade-offs, not better ones; [DESIGN.md](DESIGN.md) explains them.

## Docs

| | |
|---|---|
| [Using shroud](docs/usage.md) | sessions, every key, visual mode, shell integration, mouse |
| [Config](docs/config.md) | keys, theme, shortcuts, layout files; reloads as you save |
| [Scripting](docs/scripting.md) | `shroud pane`, `shroud events` and the JSON socket |
| [Plugins](docs/plugins.md) | Lua plugins, the lockfile, writing your own |
| [Performance](docs/performance.md) | benchmarks, `shroud bench`, comparisons with tmux and Zellij |
| [Design](DESIGN.md) · [Architecture](docs/ARCHITECTURE.md) | why it's built this way, and where everything lives |

## Status

shroud is young (0.1). It's developed on macOS in Ghostty, and builds and passes its tests on Linux. Other terminals that speak the kitty keyboard protocol should work, but some features rely on Ghostty. Expect the config, scripting and plugin APIs to change before 1.0.

Want to contribute? Start with [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

[MIT](LICENSE). libghostty-vt and LuaJIT, which shroud links statically, are MIT-licensed too.
