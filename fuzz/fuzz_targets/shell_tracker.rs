//! OSC 133 and OSC 7 come from whatever runs in a pane, so any sequence of marks and paths must be safe.
#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use shroud_core::shell::{Mark, Tracker};

fuzz_target!(|data: &[u8]| {
    let _ = run(&mut Unstructured::new(data));
});

fn run(u: &mut Unstructured) -> arbitrary::Result<()> {
    let mut t = Tracker::new();
    let mut finished = 0;
    while !u.is_empty() {
        if bool::arbitrary(u)? {
            t.set_cwd(u.arbitrary()?);
            if let Some(cwd) = &t.cwd {
                assert!(cwd.starts_with('/'));
            }
            continue;
        }
        let mark = match u.int_in_range(0..=3)? {
            0 => Mark::PromptStart,
            1 => Mark::InputStart,
            2 => Mark::OutputStart,
            _ => Mark::CommandEnd { exit: u.arbitrary()? },
        };
        if t.mark(mark).is_some() {
            finished += 1;
        }
    }
    assert_eq!(t.commands_finished, finished);
    Ok(())
}
