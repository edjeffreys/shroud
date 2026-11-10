//! Anything may write to `control.sock`, so every line must parse or be refused, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud_core::protocol::api::{encode_request, error, ok, parse_request};

fuzz_target!(|data: &[u8]| {
    let line = String::from_utf8_lossy(data);
    let (id, request) = parse_request(&line);
    match request {
        Ok(r) => {
            let (_, again) = parse_request(encode_request(None, &r).trim_end());
            assert_eq!(again, Ok(r));
            assert!(ok(id.as_ref(), serde_json::json!({})).ends_with('\n'));
        }
        Err(e) => assert_eq!(error(id.as_ref(), &e).matches('\n').count(), 1),
    }
});
