//! The grammar (leader, count, key sequence) is fixed here; which keys mean what is `Bindings`.

use super::bindings::{Bindings, Lookup};
use super::chord::Chord;
use super::commands::Command;
use super::event::{KeyAction, KeyEvent, key};
use super::shortcut::Shortcut;
use crate::layout::{Axis, Direction};

/// What visual mode starts with: nothing selected, or a selection begun as nvim's `v`, `V`
/// or `CTRL-V` would.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selecting {
    Nothing,
    Chars,
    Lines,
    Block,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Split(Axis),
    Focus(Direction),
    FocusNext,
    FocusPrev,
    /// 1-based, in layout order like nvim's window numbers.
    FocusNth(u16),
    FocusPrevious,
    FocusFirst,
    FocusLastPane,
    Close,
    Only,
    Grow(Axis, i32),
    SetSize(Axis, u16),
    Equalize,
    Maximize(Axis),
    MoveToEdge(Direction),
    Exchange,
    Rotate,
    /// Negative is towards older prompts.
    JumpPrompt(i32),
    LeaveJump,
    /// From jump mode: `y` copies the selected command's output, `Y` the command line with it.
    CopyJumped {
        command: bool,
    },
    VisualJumped,
    CopyLastOutput,
    CopyLastCommand,
    NewTab,
    MoveToNewTab,
    NextTab,
    PrevTab,
    /// 1-based, like nvim's `3gt`.
    GotoTab(u16),
    OpenSwitcher,
    NewSession,
    RenameSession,
    NextSession,
    PrevSession,
    Detach,
    SendLeader,
    ToggleFrameTime,
    ToggleFloats,
    NewFloat,
    LastTab,
    LastSession,
    ToggleZoom,
    /// Typed keys and pastes go to every pane drawn in the tab, not just the focused one.
    ToggleBroadcast,
    FloatOrTile,
    RenameTab,
    Visual(Selecting),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Passthrough,
    Pending,
    Action(Action),
    Plugin { name: String, count: Option<u16> },
    Shortcut(Shortcut),
    Consumed,
}

#[derive(Default, Debug)]
pub struct Keybinds {
    pending: bool,
    count: Option<u16>,
    keys: Vec<Chord>,
    jumping: bool,
    // A pane must never see the release of a key it never saw pressed.
    swallowed: Vec<u32>,
}

impl Keybinds {
    pub fn new() -> Keybinds {
        Keybinds::default()
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// The keys typed after the leader so far, while a sequence is waiting.
    pub fn pending_keys(&self) -> Option<&[Chord]> {
        self.pending.then_some(&self.keys[..])
    }

    pub fn is_jumping(&self) -> bool {
        self.jumping
    }

    pub fn stop_jumping(&mut self) {
        self.jumping = false;
    }

    pub fn handle(&mut self, ev: &KeyEvent, bindings: &Bindings) -> Outcome {
        let code = ev.base.unwrap_or(ev.code);
        if ev.action == KeyAction::Release {
            return match self.swallowed.iter().position(|&c| c == code) {
                Some(i) => {
                    self.swallowed.swap_remove(i);
                    Outcome::Consumed
                }
                None => Outcome::Passthrough,
            };
        }
        if is_modifier(code) {
            return if self.pending { Outcome::Consumed } else { Outcome::Passthrough };
        }
        let chord = Chord::from_event(ev);
        let is_leader = chord == Some(bindings.leader);
        if self.jumping
            && !self.pending
            && !is_leader
            && let Some(out) = self.jump_key(ev, chord, code)
        {
            return out;
        }
        if !self.pending {
            if is_leader {
                self.pending = true;
                self.count = None;
                self.keys.clear();
                self.swallow(code);
                return Outcome::Pending;
            }
            return match chord.and_then(|c| bindings.direct(c)) {
                Some(cmd) => {
                    self.swallow(code);
                    self.run(cmd.clone(), None)
                }
                None => Outcome::Passthrough,
            };
        }
        self.swallow(code);
        if is_leader {
            self.reset();
            self.jumping = false;
            return Outcome::Action(Action::SendLeader);
        }
        let Some(chord) = chord else {
            self.reset();
            return Outcome::Consumed;
        };
        // A bound digit (1-9 go to tabs) is a key; 0, or an unbound digit, starts a count.
        if self.keys.is_empty()
            && let Some(d) = chord.digit()
            && (self.count.is_some() || d == 0 || matches!(bindings.after_leader(&[chord]), Lookup::Unbound))
        {
            self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d as u16).min(9999));
            return Outcome::Pending;
        }
        self.keys.push(chord);
        let mut found = bindings.after_leader(&self.keys);
        if matches!(found, Lookup::Unbound)
            && let Some(alias) = chord.nvim_alias()
        {
            *self.keys.last_mut().expect("just pushed") = alias;
            found = bindings.after_leader(&self.keys);
        }
        match found {
            Lookup::Prefix => Outcome::Pending,
            Lookup::Found(cmd) => {
                let count = self.count.filter(|&n| n > 0);
                self.reset();
                self.run(cmd, count)
            }
            Lookup::Unbound => {
                self.reset();
                Outcome::Consumed
            }
        }
    }

    // `[` and `]` repeat without the leader, `y`, `Y` and `v` act on the selected command, and
    // Escape leaves. Any other key leaves and is handled as usual, so typing just resumes.
    fn jump_key(&mut self, ev: &KeyEvent, chord: Option<Chord>, code: u32) -> Option<Outcome> {
        let action = if chord == Some(Chord::plain('[')) {
            Action::JumpPrompt(-1)
        } else if chord == Some(Chord::plain(']')) {
            Action::JumpPrompt(1)
        } else if ev.code == key::ESCAPE {
            self.jumping = false;
            Action::LeaveJump
        } else if let Some(action) = [
            ('y', Action::CopyJumped { command: false }),
            ('Y', Action::CopyJumped { command: true }),
            ('v', Action::VisualJumped),
        ]
        .into_iter()
        .find(|(c, _)| chord == Some(Chord::plain(*c)))
        .map(|(_, a)| a)
        {
            self.jumping = false;
            action
        } else {
            self.jumping = false;
            return None;
        };
        self.swallow(code);
        Some(Outcome::Action(action))
    }

    fn run(&mut self, cmd: Command, count: Option<u16>) -> Outcome {
        match cmd {
            Command::Builtin { build, count: fixed, .. } => self.fire(build(fixed.or(count))),
            Command::Plugin(name) => {
                self.jumping = false;
                Outcome::Plugin { name, count }
            }
            Command::Shortcut(s) => {
                self.jumping = false;
                Outcome::Shortcut(s)
            }
        }
    }

    fn fire(&mut self, action: Action) -> Outcome {
        self.jumping = matches!(action, Action::JumpPrompt(_));
        Outcome::Action(action)
    }

    fn reset(&mut self) {
        self.pending = false;
        self.count = None;
        self.keys.clear();
    }

    fn swallow(&mut self, code: u32) {
        if !self.swallowed.contains(&code) {
            self.swallowed.push(code);
        }
    }
}

pub(super) fn is_modifier(code: u32) -> bool {
    (key::LEFT_SHIFT..=key::RIGHT_SUPER + 4).contains(&code) || code == key::CAPS_LOCK || code == key::NUM_LOCK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::bindings::parse_sequence;
    use crate::input::commands::command;
    use crate::input::event::Mods;

    fn press(c: char) -> KeyEvent {
        KeyEvent { text: c.to_string(), ..KeyEvent::press(c.to_ascii_lowercase() as u32, Mods::NONE) }
    }

    fn leader() -> KeyEvent {
        KeyEvent::press(key::SPACE, Mods::CTRL)
    }

    fn run(keys: &[KeyEvent]) -> Outcome {
        run_with(&Bindings::default(), keys)
    }

    fn run_with(b: &Bindings, keys: &[KeyEvent]) -> Outcome {
        let mut k = Keybinds::new();
        keys.iter().map(|e| k.handle(e, b)).last().unwrap()
    }

    #[test]
    fn nvim_window_commands() {
        assert_eq!(run(&[leader(), press('v')]), Outcome::Action(Action::Split(Axis::Horizontal)));
        assert_eq!(run(&[leader(), press('L')]), Outcome::Action(Action::MoveToEdge(Direction::Right)));
        assert_eq!(run(&[leader(), press('=')]), Outcome::Action(Action::Equalize));
        assert_eq!(run(&[leader(), press('|')]), Outcome::Action(Action::Maximize(Axis::Horizontal)));
    }

    #[test]
    fn counts_scale_resizes_and_select_panes() {
        assert_eq!(
            run(&[leader(), press('0'), press('1'), press('0'), press('+')]),
            Outcome::Action(Action::Grow(Axis::Vertical, 10))
        );
        assert_eq!(run(&[leader(), press('0'), press('3'), press('w')]), Outcome::Action(Action::FocusNth(3)));
        assert_eq!(run(&[leader(), press('0'), press('w')]), Outcome::Action(Action::FocusNext));
    }

    #[test]
    fn bare_digits_go_to_tabs_unless_unbound() {
        assert_eq!(run(&[leader(), press('3')]), Outcome::Action(Action::GotoTab(3)));
        assert_eq!(
            run(&[leader(), press('0'), press('3'), press('g'), press('t')]),
            Outcome::Action(Action::GotoTab(3))
        );
        let mut b = Bindings::default();
        b.bind_leader(parse_sequence("3").unwrap(), None);
        assert_eq!(run_with(&b, &[leader(), press('3'), press('w')]), Outcome::Action(Action::FocusNth(3)));
    }

    #[test]
    fn ctrl_letters_and_arrows_alias_like_nvim() {
        assert_eq!(
            run(&[leader(), KeyEvent::press('h' as u32, Mods::CTRL)]),
            Outcome::Action(Action::Focus(Direction::Left))
        );
        assert_eq!(
            run(&[leader(), KeyEvent::press(key::DOWN, Mods::NONE)]),
            Outcome::Action(Action::Focus(Direction::Down))
        );
    }

    #[test]
    fn shifted_symbols_without_text_resolve_from_kitty_fields() {
        let plus =
            KeyEvent { shifted: Some('+' as u32), base: Some('=' as u32), ..KeyEvent::press('=' as u32, Mods::SHIFT) };
        assert_eq!(run(&[leader(), plus]), Outcome::Action(Action::Grow(Axis::Vertical, 1)));
        let big_h = KeyEvent::press('h' as u32, Mods::SHIFT);
        assert_eq!(run(&[leader(), big_h]), Outcome::Action(Action::MoveToEdge(Direction::Left)));
    }

    #[test]
    fn leader_twice_sends_it_and_unbound_keys_are_swallowed() {
        assert_eq!(run(&[leader(), leader()]), Outcome::Action(Action::SendLeader));
        assert_eq!(run(&[leader(), press('Z')]), Outcome::Consumed);
        assert_eq!(run(&[press('v')]), Outcome::Passthrough);
    }

    #[test]
    fn releases_of_consumed_keys_are_swallowed_once() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        k.handle(&leader(), &d);
        let release = KeyEvent { action: KeyAction::Release, ..leader() };
        assert_eq!(k.handle(&release, &d), Outcome::Consumed);
        assert_eq!(k.handle(&release, &d), Outcome::Passthrough);
    }

    #[test]
    fn modifier_presses_while_pending_do_not_cancel() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        k.handle(&leader(), &d);
        assert_eq!(k.handle(&KeyEvent::press(key::LEFT_SHIFT, Mods::SHIFT), &d), Outcome::Consumed);
        assert!(k.is_pending());
    }

    #[test]
    fn prompt_jumps_repeat_without_the_leader_until_another_key() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        k.handle(&leader(), &d);
        assert_eq!(k.handle(&press('['), &d), Outcome::Action(Action::JumpPrompt(-1)));
        assert!(k.is_jumping());
        assert_eq!(k.handle(&press('['), &d), Outcome::Action(Action::JumpPrompt(-1)));
        assert_eq!(k.handle(&press(']'), &d), Outcome::Action(Action::JumpPrompt(1)));
        assert_eq!(k.handle(&press('l'), &d), Outcome::Passthrough);
        assert!(!k.is_jumping());
        assert_eq!(k.handle(&press('['), &d), Outcome::Passthrough);
    }

    #[test]
    fn y_and_v_act_on_the_selected_command_and_leave() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        for (key, action) in [
            ('y', Action::CopyJumped { command: false }),
            ('Y', Action::CopyJumped { command: true }),
            ('v', Action::VisualJumped),
        ] {
            k.handle(&leader(), &d);
            k.handle(&press('['), &d);
            assert_eq!(k.handle(&press(key), &d), Outcome::Action(action));
            assert!(!k.is_jumping());
        }
    }

    #[test]
    fn escape_leaves_jump_mode_without_reaching_the_pane() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        k.handle(&leader(), &d);
        k.handle(&press('0'), &d);
        k.handle(&press('3'), &d);
        assert_eq!(k.handle(&press('['), &d), Outcome::Action(Action::JumpPrompt(-3)));
        assert_eq!(k.handle(&KeyEvent::press(key::ESCAPE, Mods::NONE), &d), Outcome::Action(Action::LeaveJump));
        assert!(!k.is_jumping());
    }

    #[test]
    fn other_leader_commands_do_not_enter_jump_mode() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        k.handle(&leader(), &d);
        assert_eq!(k.handle(&press('y'), &d), Outcome::Action(Action::CopyLastOutput));
        assert!(!k.is_jumping());
    }

    #[test]
    fn tab_commands_follow_nvims_g_prefix() {
        assert_eq!(run(&[leader(), press('g'), press('t')]), Outcome::Action(Action::NextTab));
        assert_eq!(run(&[leader(), press('g'), press('T')]), Outcome::Action(Action::PrevTab));
        assert_eq!(run(&[leader(), press('g'), press('x')]), Outcome::Consumed);
        assert_eq!(
            run(&[leader(), press('g'), KeyEvent::press(key::TAB, Mods::NONE)]),
            Outcome::Action(Action::LastTab)
        );
        assert_eq!(run(&[leader(), press('T')]), Outcome::Action(Action::MoveToNewTab));
        assert_eq!(run(&[leader(), press(';')]), Outcome::Action(Action::OpenSwitcher));
    }

    #[test]
    fn session_keys() {
        assert_eq!(run(&[leader(), press('C')]), Outcome::Action(Action::NewSession));
        assert_eq!(run(&[leader(), press('$')]), Outcome::Action(Action::RenameSession));
        assert_eq!(run(&[leader(), press(')')]), Outcome::Action(Action::NextSession));
        assert_eq!(run(&[leader(), press('(')]), Outcome::Action(Action::PrevSession));
        assert_eq!(run(&[leader(), press('\'')]), Outcome::Action(Action::LastSession));
    }

    #[test]
    fn g_prefix_does_not_leak_into_the_next_leader() {
        let d = Bindings::default();
        let mut k = Keybinds::new();
        k.handle(&leader(), &d);
        k.handle(&press('g'), &d);
        k.handle(&press('t'), &d);
        k.handle(&leader(), &d);
        assert_eq!(k.handle(&press('t'), &d), Outcome::Action(Action::FocusFirst));
    }

    #[test]
    fn config_rebinds_sequences_and_direct_keys() {
        let mut b = Bindings::default();
        b.bind_leader(parse_sequence("ctrl+h").unwrap(), command("close"));
        b.bind_leader(parse_sequence("a b").unwrap(), command("new-tab"));
        b.bind_direct(Chord::parse("ctrl+l").unwrap(), command("focus-right"));
        let ctrl = |c: char| KeyEvent::press(c as u32, Mods::CTRL);
        assert_eq!(run_with(&b, &[leader(), ctrl('h')]), Outcome::Action(Action::Close));
        assert_eq!(run_with(&b, &[leader(), ctrl('j')]), Outcome::Action(Action::Focus(Direction::Down)));
        assert_eq!(run_with(&b, &[leader(), press('a')]), Outcome::Pending);
        assert_eq!(run_with(&b, &[leader(), press('a'), press('b')]), Outcome::Action(Action::NewTab));
        assert_eq!(run_with(&b, &[ctrl('l')]), Outcome::Action(Action::Focus(Direction::Right)));
        assert_eq!(run_with(&b, &[ctrl('k')]), Outcome::Passthrough);
    }

    #[test]
    fn a_new_leader_works_and_the_old_one_passes_through() {
        let mut b = Bindings::default();
        b.leader = Chord::parse("ctrl+a").unwrap();
        assert_eq!(
            run_with(&b, &[KeyEvent::press('a' as u32, Mods::CTRL), press('v')]),
            Outcome::Action(Action::Split(Axis::Horizontal))
        );
        assert_eq!(run_with(&b, &[leader()]), Outcome::Passthrough);
    }

    #[test]
    fn layout_independent_via_base_key() {
        let cyrillic_v =
            KeyEvent { base: Some('v' as u32), text: "м".into(), ..KeyEvent::press('м' as u32, Mods::NONE) };
        assert_eq!(run(&[leader(), cyrillic_v]), Outcome::Action(Action::Split(Axis::Horizontal)));
    }
}
