//! A peer can send anything; decode must reject or consume, never panic, and encode must round-trip.
#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use shroud_core::protocol::{Attach, ClientMsg, Control, ExitReason, ServerMsg, Size};

fuzz_target!(|data: &[u8]| {
    stream::<ClientMsg>(data, ClientMsg::decode);
    stream::<ServerMsg>(data, ServerMsg::decode);
    let _ = round_trip(&mut Unstructured::new(data));
});

fn stream<'a, M>(mut buf: &'a [u8], decode: fn(&'a [u8]) -> shroud_core::protocol::message::Decoded<'a, M>) {
    while let Ok(Some((_, n))) = decode(buf) {
        assert!(n > 0 && n <= buf.len());
        buf = &buf[n..];
    }
}

fn round_trip(u: &mut Unstructured) -> arbitrary::Result<()> {
    let bytes: &[u8] = u.arbitrary()?;
    let size = Size { cols: u.arbitrary()?, rows: u.arbitrary()?, px_w: u.arbitrary()?, px_h: u.arbitrary()? };
    let name: &str = u.arbitrary()?;
    let attach = match u.int_in_range(0..=2)? {
        0 => Attach::Recent,
        1 => Attach::Named(name),
        _ => Attach::New { name, cwd: u.arbitrary()? },
    };
    let control = match u.int_in_range(0..=2)? {
        0 => Control::List,
        1 => Control::Kill(name),
        _ => Control::Rename { from: name, to: u.arbitrary()? },
    };
    let client = match u.int_in_range(0..=4)? {
        0 => ClientMsg::Hello { version: u.arbitrary()?, size, attach },
        1 => ClientMsg::Input(bytes),
        2 => ClientMsg::Resize(size),
        3 => ClientMsg::Control { version: u.arbitrary()?, control },
        _ => ClientMsg::Detach,
    };
    let mut out = Vec::new();
    client.encode(&mut out);
    assert_eq!(ClientMsg::decode(&out), Ok(Some((client, out.len()))));

    let reason = match u.int_in_range(0..=4)? {
        0 => ExitReason::Detached,
        1 => ExitReason::ServerShutdown,
        2 => ExitReason::LastPaneClosed,
        3 => ExitReason::VersionMismatch { server: u.arbitrary()? },
        _ => ExitReason::Refused,
    };
    let server = match u.int_in_range(0..=3)? {
        0 => ServerMsg::Welcome { version: u.arbitrary()? },
        1 => ServerMsg::Output(bytes),
        2 => ServerMsg::Reply { ok: u.arbitrary()?, text: name },
        _ => ServerMsg::Exit(reason),
    };
    out.clear();
    server.encode(&mut out);
    assert_eq!(ServerMsg::decode(&out), Ok(Some((server, out.len()))));
    Ok(())
}
