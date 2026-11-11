# Performance

`time cat` of a 200 MB file mixing text, colour and wide characters, on an M-series Mac with Ghostty 1.3.1, 2026-11-07:

| | Time |
|---|---|
| Ghostty, no multiplexer | 1.57 s |
| Ghostty, inside shroud (release build) | 1.87 s |

The cost is shroud's own parse of the stream, about 107 MB/s. Ghostty only ever sees a few dozen diffed frames, not the 200 MB. To reproduce, run `scripts/bench.sh` once in a plain Ghostty window and once inside shroud.

## Compared with tmux and Zellij

These numbers come from one synthetic workload on one machine, to keep shroud honest about its own performance rather than to rank the tools. tmux and Zellij do more than shroud does (remote use, many terminals, their own plugin systems) and make different trade-offs, and they may well do better on workloads not measured here.

`cargo run --release -p shroud-smoke --bin shroud-compare -- target/release/shroud` (add `--mb N` or `--only shroud,tmux`) drives each multiplexer headless through a PTY at 200×50: the same bash, an isolated socket and config, and the outer PTY drained as fast as it can be read, so it times the multiplexer rather than a terminal's renderer. M-series Mac, shroud release build, tmux 3.8, zellij 0.45.1, 2026-11-08:

| | `cat` 50 MB | CPU for it | key echo p50 / p99 | while another pane floods | idle CPU | memory after the `cat` | to first prompt |
|---|---|---|---|---|---|---|---|
| no multiplexer | 0.36 s | — | 0.32 / 0.55 ms | — | — | — | 0.01 s |
| shroud | 0.46 s | 0.38 s | 0.59 / 1.01 ms | 0.19 / 0.24 ms | 0% | 17 MB | 0.03 s |
| tmux | 2.61 s | 2.54 s | 0.43 / 0.79 ms | 200.6 / 200.7 ms | 0% | 8 MB | 0.03 s |
| zellij | 0.84 s | 2.16 s | 12.5 / 13.3 ms | 4.7 / 7.1 ms | 0% | 71 MB | 0.10 s |

Key echo is a bash loop that writes each key back as eight copies of itself, typed 15 ms apart; "while another pane floods" is the same with `yes` running in a second pane. At that relaxed pace tmux's echo is 0.1–0.2 ms quicker from run to run; typed back to back, shroud echoes in 0.05 ms. With a pane flooding, the other pane's echo took about 200 ms under tmux in this test. shroud's memory includes 10 MB of history (`scrollback`), where tmux keeps 2,000 lines by default.

Memory is resident size of every process the multiplexer started, shells excluded; shroud's includes the plugin host.

`shroud bench` times the server's hot path in-process, with no PTY or terminal involved, so its numbers repeat. On the same Mac, release build:

```
shroud bench (release, 64 MB, 200×50)
  parse    485.6 MB/s
  frame  p50 0.184 ms  p99 0.255 ms  max 0.268 ms  (4.7 KB written per frame)
  echo   p50 0.013 ms  p99 0.018 ms  max 0.188 ms  (53 B written per frame)
```

`frame` redraws a full screen of new output; `echo` adds one character to a full screen, as a keystroke does. `shroud bench --check` fails if parsing drops below 40 MB/s, a frame's p99 goes above 4 ms or an echo frame's above 1 ms; CI runs it. The smoke test also holds an idle server to under 1% CPU.
