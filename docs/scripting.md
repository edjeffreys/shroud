# Scripting

Scripts and agents drive shroud through `shroud pane` and `shroud events`. They target the pane given with `-p`, else the pane the script is running in (`$SHROUD_PANE`), else the focused pane.

```bash
shroud pane run --wait 'cargo test'      # opens a pane, runs it, prints its output, exits with its code
shroud pane split --down --cwd ~/src     # prints the new pane's id; focus stays where it was
shroud pane split --float                # a floating pane over the tab
shroud pane run --float --close lazygit  # a popup: the pane closes when lazygit exits
shroud pane run --keep 'cargo test'      # a command pane: when it ends, Enter reruns and q closes
shroud pane send -p 7 'make' -k enter    # text and keys, in order
shroud pane output -p 7                  # the last command's output
shroud pane capture -p 7 --history       # the pane's text: the screen, or with --history everything kept
shroud command zoom                      # any command a key can be bound to, as if it were pressed
shroud pane close -p 7
shroud pane ls                           # sessions, tabs and panes as JSON
shroud events                            # JSON lines: command_finished, pane_opened, pane_closed
```

New panes don't take focus unless you pass `--focus`, so a script never moves your cursor. `--wait` and `command_finished` need [shell integration](usage.md#shell-integration) in the pane. Without it, `--wait` fails after 10 seconds rather than hang.

Underneath is newline-delimited JSON on `control.sock` in shroud's runtime directory (`$XDG_RUNTIME_DIR/shroud` where your system sets it, as most Linux desktops do; otherwise `$TMPDIR/shroud-$UID`), so any language can talk to it directly:

```
→ {"id":1,"cmd":"split","dir":"down","command":"cargo test","wait":true}
← {"id":1,"ok":true,"pane":8,"exit":0,"output":"…"}
→ {"cmd":"send","pane":8,"input":[{"text":"q"},{"key":"ctrl+c"}]}
← {"ok":true}
→ {"cmd":"subscribe"}
← {"ok":true}
← {"event":"command_finished","pane":8,"exit":1}
```

The commands are:
- **panes:** `list` (each pane's `process` is its foreground program, when that isn't the shell, and each tab says whether it `broadcast`s), `split` (`pane`, `dir`, `cwd`, `command`, `focus`, `wait`, `close_on_exit` or `keep`, `float` with `x`, `y`, `w`, `h` in percent), `send`, `output`, `capture` (`history` for everything kept), `close`, `focus`;
- **commands:** `run` (`command` such as `"zoom"`, `"goto-tab 3"`, `"sessions.save"` or a shortcut table like `{"exec": ["git", "pull"]}`, as if `client` pressed its key; without `client`, a client on `session`), so anything a key can do, a script or plugin can too;
- **layouts and sessions:** `layout` (a tab in the layout-file model, with sizes in percent), `apply_layout` (a new tab from one), `set_layout` (rearrange an existing tab: a pane node with `pane` moves that pane, one without starts a new one, and every tile must be placed), `pane_title` (names a pane on its border when `pane_frames` is on, in place of its program's title; no `title` goes back to that), `tag` (a small string a plugin keeps on a pane, like tmux's `@user` options: shown in `list` and `layout`, taken by `apply_layout` and `set_layout` for new panes, and saved by `sessions`; no `value` removes it), `session_new`, `session_rename`, `session_kill`, `tab_new`;
- **for plugins:** `message` (text in the status bar for a few seconds), `status_take` / `status_set` / `status_release` (draw the status bar yourself: you get a `status` event with each client's state whenever it changes, and answer with spans for the left and right), `overlay_open` (optionally for one `client`), `overlay_update` and `overlay_close` (a box of text whose spans take a theme token or colour for `style` and `bg`, plus `bold` and `italic`; a modal one sends its keys to you, and Escape always closes it), `register_command` (a `plugin.command` name with a suggested key, used only if it's free), `bind` (a key to any command or shortcut table, over config, until the connection closes; no `command` unbinds) and `bindings`;
- `subscribe`, for events: `command_finished`, `pane_opened`, `pane_closed`, `focus_changed`, `layout_changed`, `tab_changed`, `session_opened`, `session_closed`, `session_renamed`, `client_attached`, `client_detached`, `key_pending` (the leader and keys so far, after a 300 ms pause), `key_done`, `mode_changed` (`normal`, `leader`, `jump` or `visual`), `bell`, `title_changed`, `cwd_changed`, `tag_changed` and `output_started` (a pane printing after 2 s without output, not counting a key's echo: activity in a background pane). `command`, `overlay_key` and `overlay_closed` go only to the connection they concern.

Overlays and commands belong to the connection that made them and go away with it. A mistake gets `{"ok":false,"error":"…"}` and the connection stays open.
