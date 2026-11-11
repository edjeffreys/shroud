# Using shroud

## Sessions

```bash
shroud    # or target/release/shroud if you built it
```

Running `shroud` attaches to the session you used most recently, starting the server and a session called `default` if needed. The server keeps every session's panes alive after you detach.

```bash
shroud session new [<name>]        # start a session in this directory and attach
shroud session attach [<name>]     # attach to a session
shroud session ls                  # list sessions
shroud session rename <old> <new>
shroud session kill <name>         # close a session and its shells
shroud server restart              # start a new server, e.g. after a rebuild
shroud server stop                 # stop the server and close every shell
```

The server outlives its windows on purpose, so quitting Ghostty leaves your shells running. `shroud server restart` stops it and every attached window starts the new binary and reattaches. Shells start afresh, since the old server owned them; with the `sessions` plugin's `auto_restore` on, the layouts come back as they were up to two seconds before.

Inside shroud, `Ctrl+Space C` creates a session (it asks for a name and starts in the current pane's directory), `Ctrl+Space $` renames the current one, and `Ctrl+Space (` / `)` move to the previous or next session.

`Ctrl+Space ;` opens the switcher: every session, most recently used first. Type to filter, and if the name you type doesn't exist, Enter creates it. Move with the arrows or `Ctrl+N` / `Ctrl+P`; Enter switches, `Ctrl+R` renames, `Ctrl+X` closes (after asking), Escape leaves.

When a session ends, its clients move to the most recently used session left, and exit if there isn't one.

## Keys

The leader is **Ctrl+Space**. After it, keys follow nvim's `Ctrl+W` window commands:

| Keys after the leader | Action |
|---|---|
| `v` / `s` (or `n`) | split: new pane right / below |
| `h` `j` `k` `l` (or arrows) | move focus |
| `w` / `W` | next / previous pane; with a count, e.g. `03w`, go to pane 3 |
| `p` | the previously focused pane |
| `t` / `b` | first / last pane |
| `c` / `q` | close pane |
| `o` | close every other pane |
| `+` / `-` | taller / shorter (counts work: `010+`) |
| `>` / `<` | wider / narrower |
| `=` | make all panes equal |
| `_` / `\|` | maximise height / width; with a count, set it (`020_`) |
| `H` `J` `K` `L` | move pane to the far left / bottom / top / right |
| `x` | swap with the next pane |
| `r` | rotate panes in the row or column |
| `[` / `]` | jump mode: select the last finished command, then keep pressing `[` `]` without the leader to move between commands; `y` copies the selected one's output, `Y` the command line with it, `v` selects its output in visual mode; any other key returns (shroud only) |
| `y` | copy the last command's output to the clipboard (shroud only) |
| `Y` | copy the last command line and its output, as `$ cmd` then the output; just the output if your shell doesn't mark where input starts (shroud only) |
| `d` | detach (shroud only) |
| `N` | new tab with a fresh shell (shroud only) |
| `T` | move the focused pane into its own tab (nvim's `CTRL-W T`) |
| `1`–`9` | go to that tab (shroud only) |
| `gt` / `gT` | next / previous tab |
| `g Tab` | the last tab you were on (nvim's `g<Tab>`) |
| `;` | open the session switcher (shroud only) |
| `C` | new session (shroud only) |
| `$` | rename this session |
| `(` / `)` | previous / next session |
| `'` | the last session you were in (shroud only) |
| `gp` | show or hide the frame-time overlay (shroud only) |
| `f` | show or hide this tab's floating panes; opens one if there are none (shroud only). From a float, `h`/`j`/`k`/`l` move to the next float that way, or past the last one back to the tiles, hiding the floats |
| `F` | open another floating pane (shroud only) |
| `e` | float the focused pane, or tile a floating one (shroud only) |
| `z` | zoom: the focused pane fills the tab until you press it again or move focus (shroud only) |
| `B` | broadcast: what you type and paste goes to every pane you can see in the tab, until you press it again (tmux's `synchronize-panes`). Floats count while shown; a zoomed tab types into the zoomed pane alone |
| `,` | name this tab; an empty name clears it (shroud only) |
| `V` | visual mode: scroll back, search and copy with nvim keys (shroud only) |
| `g v` / `g V` / `g Ctrl+v` | visual mode with a character, line or block selection already started (shroud only) |
| `Ctrl+Space` | send a literal Ctrl+Space |

As in nvim, `Ctrl+<letter>` after the leader works the same as the plain letter, so `Ctrl+Space Ctrl+h` moves focus left.

Counts start with `0`, because `1`–`9` go straight to tabs: `Ctrl+Space 03w` focuses pane 3. Unbind a digit in config and it starts a count again.

## Visual mode

Visual mode starts at the cursor and follows nvim:

- `h` `j` `k` `l`, `w` `b` `e` `ge`, `W` `B` `E` `gE`, `0` `^` `$`, `gg` `G`, `H` `M` `L`, `Ctrl+d` `Ctrl+u` `Ctrl+f` `Ctrl+b`, with counts;
- `f` `F` `t` `T` find a character on the line, and `;` / `,` repeat it;
- `[` / `]` jump between prompts;
- `/` and `?` search (smartcase), moving as you type, with matches on screen underlined; Escape goes back to where you started, and `n` / `N` repeat it;
- `v`, `V` and `Ctrl+v` select by character, line or block, and `o` jumps to the other end;
- with a selection, text objects replace it: `iw` `aw` `iW` `aW`, `i"` `a"` (and `'`, `` ` ``), `i(` `a(` (and `[`, `{`, `<`; `b` and `B` too);
- `y` copies the selection and leaves. Without one, `y` takes a motion or text object as nvim's does: `yw`, `ye`, `y$`, `yiw`, `yi"`, `yt,`, `yj`; `yy` and `Y` copy lines. Enter copies the selection, or the cursor's line;
- Escape cancels a pending `y`, then clears the selection, then leaves; `q` leaves at once. The leader also leaves, so window keys still work.

`visual` enters with nothing selected. `visual-char`, `visual-line` and `visual-block` enter with a selection already started, as `v`, `V` and `Ctrl+v` do in nvim. Their default keys are `g v`, `g V` and `g Ctrl+v`, since nvim's `CTRL-W v` is a split; with your splits bound elsewhere, `"v" = "visual-char"` and friends give you nvim's keys. The status bar shows `VISUAL`, `V-LINE` or `V-BLOCK`.

Prompt jumps follow nvim's `[[` and `]]` in terminal buffers.

## Status bar

The status bar along the bottom shows the leader or jump mode and the focused pane's directory and git branch on the left, and the time on the right. When a command fails or finishes in a pane you weren't looking at, that pane's number appears by the clock with ✗ or ✓ (`2✗`); `Ctrl+Space 02w` takes you there and clears it.

In jump mode the selected command's line is highlighted, and the bar says which it is, such as `3/12  cargo test  ✗1`: its place among the commands in the pane's history, the command, and how it exited when the shell said.

## Shell integration

Jumping, copying and badges need your shell to send shell-integration marks. In Ghostty, shroud loads Ghostty's own integration into each pane for zsh and fish, with no dotfile changes. For bash, source `$GHOSTTY_RESOURCES_DIR/shell-integration/bash/ghostty.bash` from your `.bashrc`.

Panes set `SHROUD=1`, so dotfiles can tell they're inside shroud, and `shroud` typed in a pane prints its usage rather than nesting. They also set `TERM_PROGRAM=ghostty`, because programs in a pane are talking to Ghostty's parser whatever terminal you run shroud in. A `.zshrc` that starts shroud in Ghostty windows should check `$SHROUD` first, or every pane will try to start it again:

```zsh
[[ -z $SHROUD && $TERM_PROGRAM == ghostty ]] && exec shroud
```

## Mouse

- click a pane to focus it;
- the wheel scrolls back through a pane's history unless the program in it uses the mouse itself;
- in a pane whose program doesn't use the mouse, drag to select and release to copy; double-click copies a word and triple-click a line.

A program that uses the mouse (nvim with `mouse=a`, htop) gets every click and drag itself. Ghostty selects natively if you hold Shift, but that selection spans pane borders.
