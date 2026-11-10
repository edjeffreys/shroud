//! Program text is sanitised: a stray ESC or BEL would end the sequence early.

pub fn bell(out: &mut Vec<u8>) {
    out.push(0x07);
}

pub fn clipboard(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(b"\x1b]52;c;");
    base64(out, data);
    out.extend_from_slice(b"\x1b\\");
}

pub fn notification(out: &mut Vec<u8>, title: &str, body: &str) {
    out.extend_from_slice(b"\x1b]777;notify;");
    sanitised(out, title, true);
    out.push(b';');
    sanitised(out, body, false);
    out.extend_from_slice(b"\x1b\\");
}

fn sanitised(out: &mut Vec<u8>, s: &str, field: bool) {
    out.extend(s.bytes().filter(|&b| b >= 0x20 && b != 0x7f && !(field && b == b';')));
}

fn base64(out: &mut Vec<u8>, data: &[u8]) {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize]);
            } else {
                out.push(b'=');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_base64() {
        let mut out = Vec::new();
        clipboard(&mut out, b"hi!?");
        assert_eq!(out, b"\x1b]52;c;aGkhPw==\x1b\\");
    }

    #[test]
    fn notification_strips_control_bytes() {
        let mut out = Vec::new();
        notification(&mut out, "a;b\x1b", "x\x07y");
        assert_eq!(out, b"\x1b]777;notify;ab;xy\x1b\\");
    }
}
