//! Any byte stream, split anywhere across reads, must decode to the same events without panicking.
#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud_core::input::{Decoder, Event};

fuzz_target!(|data: &[u8]| {
    let Some((&split_seed, bytes)) = data.split_first() else { return };
    let whole = decode(&[bytes]);
    let chunks = chunked(bytes, split_seed);
    assert_eq!(whole, decode(&chunks));
});

fn decode(chunks: &[&[u8]]) -> Vec<Event> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for c in chunks {
        d.feed(c, &mut out);
    }
    d.flush(&mut out);
    out
}

fn chunked(bytes: &[u8], seed: u8) -> Vec<&[u8]> {
    let step = usize::from(seed % 7) + 1;
    bytes.chunks(step).collect()
}
