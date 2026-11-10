//! Config is hand-edited and reloaded mid-edit, so any text must give settings or problems, never a panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud_core::config::{Lock, parse, parse_layout};
use shroud_core::layout::Tree;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    if let Ok((_, problems)) = parse(text) {
        for p in problems {
            let _ = p.to_string();
        }
    }
    if let Ok(lock) = Lock::parse(text) {
        assert_eq!(Lock::parse(&lock.write()), Ok(lock));
    }
    if let Ok(tabs) = parse_layout(text) {
        for shape in tabs {
            let ids = shape.map(&mut |_| 1);
            assert!(Tree::from_shape(&ids).is_some());
        }
    }
});
