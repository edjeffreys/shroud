//! Wire format: `u8 tag | u32 length (LE) | payload`. Decoding borrows from the receive
//! buffer and never allocates. Bump VERSION whenever an old peer would misread a message.

pub const VERSION: u16 = 3;
pub const MAX_PAYLOAD: usize = 16 << 20;
const HEADER: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub px_w: u16,
    pub px_h: u16,
}

/// Which session a new client wants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attach<'a> {
    /// The most recently used session, or a new `default` if there are none.
    Recent,
    Named(&'a str),
    /// An empty name lets the server pick one.
    New {
        name: &'a str,
        cwd: &'a str,
    },
}

/// One-shot requests from the CLI; the server answers with a `Reply` and closes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control<'a> {
    List,
    Kill(&'a str),
    Rename {
        from: &'a str,
        to: &'a str,
    },
    /// Stops the server, closing every shell. With `restart`, attached clients start a new one.
    StopServer {
        restart: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClientMsg<'a> {
    Hello { version: u16, size: Size, attach: Attach<'a> },
    Input(&'a [u8]),
    Resize(Size),
    Detach,
    Control { version: u16, control: Control<'a> },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitReason {
    Detached,
    ServerShutdown,
    LastPaneClosed,
    VersionMismatch {
        server: u16,
    },
    /// The preceding `Reply` says why.
    Refused,
    /// The server is stopping so a new one can take over; the client starts it and reattaches.
    Restart,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServerMsg<'a> {
    Welcome { version: u16 },
    Output(&'a [u8]),
    Exit(ExitReason),
    Reply { ok: bool, text: &'a str },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    UnknownTag(u8),
    TooLarge(usize),
    Malformed,
}

/// Ok(None) means more bytes are needed. Ok(Some((msg, consumed))) otherwise.
pub type Decoded<'a, M> = Result<Option<(M, usize)>, DecodeError>;

impl ClientMsg<'_> {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = begin(out);
        let tag = match self {
            ClientMsg::Hello { version, size, attach } => {
                out.extend_from_slice(&version.to_le_bytes());
                put_size(out, size);
                match attach {
                    Attach::Recent => out.push(0),
                    Attach::Named(name) => {
                        out.push(1);
                        put_str(out, name);
                    }
                    Attach::New { name, cwd } => {
                        out.push(2);
                        put_str(out, name);
                        put_str(out, cwd);
                    }
                }
                1
            }
            ClientMsg::Input(bytes) => {
                out.extend_from_slice(bytes);
                2
            }
            ClientMsg::Resize(size) => {
                put_size(out, size);
                3
            }
            ClientMsg::Detach => 4,
            ClientMsg::Control { version, control } => {
                out.extend_from_slice(&version.to_le_bytes());
                match control {
                    Control::List => out.push(0),
                    Control::Kill(name) => {
                        out.push(1);
                        put_str(out, name);
                    }
                    Control::Rename { from, to } => {
                        out.push(2);
                        put_str(out, from);
                        put_str(out, to);
                    }
                    Control::StopServer { restart } => {
                        out.push(3);
                        out.push(u8::from(*restart));
                    }
                }
                5
            }
        };
        finish(out, start, tag);
    }

    pub fn decode(buf: &[u8]) -> Decoded<'_, ClientMsg<'_>> {
        let Some(((tag, p), n)) = frame(buf)? else { return Ok(None) };
        let msg = match tag {
            1 => {
                let version = u16_at(p, 0)?;
                let size = size_at(p, 2)?;
                // Version comes first so an older client still gets a clean mismatch, not Malformed.
                let mut at = 10;
                let attach = match p.get(at) {
                    None => Attach::Recent,
                    Some(kind) => {
                        at += 1;
                        match kind {
                            0 => Attach::Recent,
                            1 => Attach::Named(str_at(p, &mut at)?),
                            2 => Attach::New { name: str_at(p, &mut at)?, cwd: str_at(p, &mut at)? },
                            _ => return Err(DecodeError::Malformed),
                        }
                    }
                };
                ClientMsg::Hello { version, size, attach }
            }
            2 => ClientMsg::Input(p),
            3 => ClientMsg::Resize(size_at(p, 0)?),
            4 => ClientMsg::Detach,
            5 => {
                let version = u16_at(p, 0)?;
                let mut at = 3;
                let control = match p.get(2) {
                    Some(0) => Control::List,
                    Some(1) => Control::Kill(str_at(p, &mut at)?),
                    Some(2) => Control::Rename { from: str_at(p, &mut at)?, to: str_at(p, &mut at)? },
                    Some(3) => match p.get(3) {
                        Some(0) => Control::StopServer { restart: false },
                        Some(1) => Control::StopServer { restart: true },
                        _ => return Err(DecodeError::Malformed),
                    },
                    _ => return Err(DecodeError::Malformed),
                };
                ClientMsg::Control { version, control }
            }
            t => return Err(DecodeError::UnknownTag(t)),
        };
        Ok(Some((msg, n)))
    }
}

impl ServerMsg<'_> {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = begin(out);
        let tag = match self {
            ServerMsg::Welcome { version } => {
                out.extend_from_slice(&version.to_le_bytes());
                1
            }
            ServerMsg::Output(bytes) => {
                out.extend_from_slice(bytes);
                2
            }
            ServerMsg::Exit(reason) => {
                let (code, extra) = match reason {
                    ExitReason::Detached => (0u8, 0u16),
                    ExitReason::ServerShutdown => (1, 0),
                    ExitReason::LastPaneClosed => (2, 0),
                    ExitReason::VersionMismatch { server } => (3, *server),
                    ExitReason::Refused => (4, 0),
                    ExitReason::Restart => (5, 0),
                };
                out.push(code);
                out.extend_from_slice(&extra.to_le_bytes());
                3
            }
            ServerMsg::Reply { ok, text } => {
                out.push(u8::from(*ok));
                out.extend_from_slice(text.as_bytes());
                4
            }
        };
        finish(out, start, tag);
    }

    pub fn decode(buf: &[u8]) -> Decoded<'_, ServerMsg<'_>> {
        let Some(((tag, p), n)) = frame(buf)? else { return Ok(None) };
        let msg = match tag {
            1 => ServerMsg::Welcome { version: u16_at(p, 0)? },
            2 => ServerMsg::Output(p),
            3 => {
                let extra = u16_at(p, 1)?;
                ServerMsg::Exit(match p.first() {
                    Some(0) => ExitReason::Detached,
                    Some(1) => ExitReason::ServerShutdown,
                    Some(2) => ExitReason::LastPaneClosed,
                    Some(3) => ExitReason::VersionMismatch { server: extra },
                    Some(4) => ExitReason::Refused,
                    Some(5) => ExitReason::Restart,
                    _ => return Err(DecodeError::Malformed),
                })
            }
            4 => {
                let ok = match p.first() {
                    Some(0) => false,
                    Some(1) => true,
                    _ => return Err(DecodeError::Malformed),
                };
                ServerMsg::Reply { ok, text: std::str::from_utf8(&p[1..]).map_err(|_| DecodeError::Malformed)? }
            }
            t => return Err(DecodeError::UnknownTag(t)),
        };
        Ok(Some((msg, n)))
    }
}

fn begin(out: &mut Vec<u8>) -> usize {
    let start = out.len();
    out.extend_from_slice(&[0; HEADER]);
    start
}

fn finish(out: &mut [u8], start: usize, tag: u8) {
    let len = (out.len() - start - HEADER) as u32;
    out[start] = tag;
    out[start + 1..start + HEADER].copy_from_slice(&len.to_le_bytes());
}

fn frame(buf: &[u8]) -> Decoded<'_, (u8, &[u8])> {
    if buf.len() < HEADER {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(DecodeError::TooLarge(len));
    }
    if buf.len() < HEADER + len {
        return Ok(None);
    }
    Ok(Some(((buf[0], &buf[HEADER..HEADER + len]), HEADER + len)))
}

fn put_size(out: &mut Vec<u8>, s: &Size) {
    for v in [s.cols, s.rows, s.px_w, s.px_h] {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let len = s.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&s.as_bytes()[..len]);
}

fn str_at<'a>(p: &'a [u8], at: &mut usize) -> Result<&'a str, DecodeError> {
    let len = u16_at(p, *at)? as usize;
    let bytes = p.get(*at + 2..*at + 2 + len).ok_or(DecodeError::Malformed)?;
    *at += 2 + len;
    std::str::from_utf8(bytes).map_err(|_| DecodeError::Malformed)
}

fn u16_at(p: &[u8], i: usize) -> Result<u16, DecodeError> {
    p.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(DecodeError::Malformed)
}

fn size_at(p: &[u8], i: usize) -> Result<Size, DecodeError> {
    Ok(Size { cols: u16_at(p, i)?, rows: u16_at(p, i + 2)?, px_w: u16_at(p, i + 4)?, px_h: u16_at(p, i + 6)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size { cols: 80, rows: 24, px_w: 800, px_h: 480 };

    #[test]
    fn client_messages_round_trip() {
        for msg in [
            ClientMsg::Hello { version: VERSION, size: SIZE, attach: Attach::Recent },
            ClientMsg::Hello { version: VERSION, size: SIZE, attach: Attach::Named("work") },
            ClientMsg::Hello { version: VERSION, size: SIZE, attach: Attach::New { name: "", cwd: "/Users/ed" } },
            ClientMsg::Control { version: VERSION, control: Control::List },
            ClientMsg::Control { version: VERSION, control: Control::Rename { from: "a", to: "b" } },
            ClientMsg::Control { version: VERSION, control: Control::StopServer { restart: true } },
            ClientMsg::Control { version: VERSION, control: Control::StopServer { restart: false } },
            ClientMsg::Input(b"\x1b[A"),
            ClientMsg::Resize(SIZE),
            ClientMsg::Detach,
        ] {
            let mut out = Vec::new();
            msg.encode(&mut out);
            assert_eq!(ClientMsg::decode(&out), Ok(Some((msg, out.len()))));
        }
    }

    #[test]
    fn server_messages_round_trip() {
        let exits =
            [ExitReason::Detached, ExitReason::VersionMismatch { server: 9 }, ExitReason::Refused, ExitReason::Restart]
                .map(ServerMsg::Exit);
        let others = [
            ServerMsg::Welcome { version: VERSION },
            ServerMsg::Output(b"frame"),
            ServerMsg::Reply { ok: false, text: "no session x" },
        ];
        for msg in others.into_iter().chain(exits) {
            let mut out = Vec::new();
            msg.encode(&mut out);
            assert_eq!(ServerMsg::decode(&out), Ok(Some((msg, out.len()))));
        }
    }

    #[test]
    fn a_version_1_hello_still_decodes_so_it_can_be_refused() {
        let mut out = vec![1, 10, 0, 0, 0, 1, 0];
        put_size(&mut out, &SIZE);
        assert_eq!(
            ClientMsg::decode(&out),
            Ok(Some((ClientMsg::Hello { version: 1, size: SIZE, attach: Attach::Recent }, out.len())))
        );
    }

    #[test]
    fn partial_frames_wait_and_bad_ones_error() {
        let mut out = Vec::new();
        ClientMsg::Input(b"abc").encode(&mut out);
        assert_eq!(ClientMsg::decode(&out[..out.len() - 1]), Ok(None));
        assert_eq!(ClientMsg::decode(&[9, 0, 0, 0, 0]), Err(DecodeError::UnknownTag(9)));
        assert_eq!(ClientMsg::decode(&[1, 1, 0, 0, 0, 0]), Err(DecodeError::Malformed));
        assert!(matches!(ClientMsg::decode(&[2, 0xff, 0xff, 0xff, 0xff]), Err(DecodeError::TooLarge(_))));
    }
}
