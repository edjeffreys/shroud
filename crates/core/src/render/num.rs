//! Decimal formatting without `format!`, which allocates on the hot path.

pub fn push_u16(out: &mut Vec<u8>, mut n: u16) {
    let mut buf = [0u8; 5];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats() {
        for n in [0u16, 7, 10, 65535] {
            let mut out = Vec::new();
            super::push_u16(&mut out, n);
            assert_eq!(out, n.to_string().as_bytes());
        }
    }
}
