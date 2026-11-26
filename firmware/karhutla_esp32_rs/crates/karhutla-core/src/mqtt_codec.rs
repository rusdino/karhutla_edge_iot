//! Minimal MQTT 3.1.1 packet codec (QoS 0 client). No heap.

struct W<'a> {
    b: &'a mut [u8],
    n: usize,
}

impl<'a> W<'a> {
    fn put(&mut self, d: &[u8]) -> Result<(), ()> {
        let end = self.n.checked_add(d.len()).ok_or(())?;
        if end > self.b.len() {
            return Err(());
        }
        self.b[self.n..end].copy_from_slice(d);
        self.n = end;
        Ok(())
    }
    fn u16(&mut self, v: u16) -> Result<(), ()> {
        self.put(&v.to_be_bytes())
    }
    fn str(&mut self, s: &[u8]) -> Result<(), ()> {
        if s.len() > u16::MAX as usize {
            return Err(());
        }
        self.u16(s.len() as u16)?;
        self.put(s)
    }
}

fn write_header(w: &mut W, first: u8, rem: usize) -> Result<(), ()> {
    if rem > 268_435_455 {
        return Err(());
    }
    w.put(&[first])?;
    let mut x = rem;
    loop {
        let mut b = (x % 128) as u8;
        x /= 128;
        if x > 0 {
            b |= 0x80;
        }
        w.put(&[b])?;
        if x == 0 {
            return Ok(());
        }
    }
}

/// Returns (remaining_length, bytes_consumed). None if truncated/malformed.
pub fn decode_remaining_length(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut val = 0usize;
    let mut mul = 1usize;
    for i in 0..4 {
        let b = *bytes.get(i)?;
        val += (b & 0x7F) as usize * mul;
        if b & 0x80 == 0 {
            return Some((val, i + 1));
        }
        mul *= 128;
    }
    None
}

pub fn encode_connect_with_lwt(
    out: &mut [u8],
    client_id: &str,
    keepalive_s: u16,
    lwt_topic: &str,
    lwt_msg: &[u8],
) -> Result<usize, ()> {
    // proto name(6) + level(1) + flags(1) + keepalive(2) + payload
    let rem = 10
        + 2 + client_id.len()
        + 2 + lwt_topic.len()
        + 2 + lwt_msg.len();
    let mut w = W { b: out, n: 0 };
    write_header(&mut w, 0x10, rem)?;
    w.put(&[0, 4, b'M', b'Q', b'T', b'T', 4])?;
    // Will flag 0x04 | Will retain 0x20 | Clean session 0x02 (Will QoS 0)
    w.put(&[0x04 | 0x20 | 0x02])?;
    w.u16(keepalive_s)?;
    w.str(client_id.as_bytes())?;
    w.str(lwt_topic.as_bytes())?;
    w.str(lwt_msg)?;
    Ok(w.n)
}

pub fn encode_subscribe(out: &mut [u8], packet_id: u16, topic: &str) -> Result<usize, ()> {
    let rem = 2 + 2 + topic.len() + 1;
    let mut w = W { b: out, n: 0 };
    write_header(&mut w, 0x82, rem)?;
    w.u16(packet_id)?;
    w.str(topic.as_bytes())?;
    w.put(&[0])?; // requested QoS 0
    Ok(w.n)
}

pub fn encode_publish(
    out: &mut [u8],
    topic: &str,
    payload: &[u8],
    retain: bool,
) -> Result<usize, ()> {
    let rem = 2 + topic.len() + payload.len();
    let mut w = W { b: out, n: 0 };
    write_header(&mut w, 0x30 | retain as u8, rem)?;
    w.str(topic.as_bytes())?;
    w.put(payload)?;
    Ok(w.n)
}

/// `hdr` is the fixed-header first byte, `body` the bytes after remaining length.
pub fn parse_publish(hdr: u8, body: &[u8]) -> Option<(&str, &[u8])> {
    if hdr >> 4 != 3 {
        return None;
    }
    let qos = (hdr >> 1) & 3;
    if qos == 3 {
        return None;
    }
    if body.len() < 2 {
        return None;
    }
    let tl = u16::from_be_bytes([body[0], body[1]]) as usize;
    let mut p = 2usize.checked_add(tl)?;
    let topic = core::str::from_utf8(body.get(2..p)?).ok()?;
    if qos > 0 {
        p = p.checked_add(2)?;
    }
    Some((topic, body.get(p..)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mqtt_codec_lwt_and_multibyte_varint() {
        // 1420-byte PUBLISH payload => 2-byte varint
        let payload: [u8; 1420] = core::array::from_fn(|i| (i % 251) as u8);
        let mut buf = [0u8; 2048];
        let n = encode_publish(&mut buf, "karhutla/log/resp", &payload, true).unwrap();
        let rem = 2 + 17 + 1420;
        assert_eq!(n, 1 + 2 + rem);
        assert_eq!(buf[0], 0x31);
        assert_eq!(decode_remaining_length(&buf[1..n]), Some((rem, 2)));
        let (r, c) = decode_remaining_length(&buf[1..]).unwrap();
        let (t, p) = parse_publish(buf[0], &buf[1 + c..1 + c + r]).unwrap();
        assert_eq!(t, "karhutla/log/resp");
        assert_eq!(p, &payload[..]);
        assert_eq!(encode_publish(&mut buf[..100], "t", &payload, false), Err(()));
        // varint boundaries
        assert_eq!(decode_remaining_length(&[0x7F]), Some((127, 1)));
        assert_eq!(decode_remaining_length(&[0x80, 0x01]), Some((128, 2)));
        assert_eq!(decode_remaining_length(&[0xFF, 0x7F]), Some((16_383, 2)));
        assert_eq!(decode_remaining_length(&[0xFF, 0xFF, 0x7F]), Some((2_097_151, 3)));
        assert_eq!(decode_remaining_length(&[0xFF, 0xFF, 0xFF, 0x7F]), Some((268_435_455, 4)));
        assert_eq!(decode_remaining_length(&[0x80]), None);
        assert_eq!(decode_remaining_length(&[0x80, 0x80, 0x80, 0x80, 0x01]), None);
        assert_eq!(decode_remaining_length(&[]), None);

        // CONNECT with LWT
        let n = encode_connect_with_lwt(&mut buf, "kh", 30, "karhutla/status", b"offline").unwrap();
        assert_eq!(buf[0], 0x10);
        assert_eq!(buf[1] as usize, n - 2);
        assert_eq!(&buf[2..9], &[0, 4, b'M', b'Q', b'T', b'T', 4]);
        assert_eq!(buf[9], 0x26);
        assert_eq!(buf[9] & 0x04, 0x04);
        assert_eq!(buf[9] & 0x20, 0x20);
        assert_eq!(buf[9] & 0x02, 0x02);
        assert_eq!(&buf[10..12], &[0, 30]);
        assert_eq!(&buf[12..16], &[0, 2, b'k', b'h']);
        assert_eq!(&buf[n - 9..n], &[0, 7, b'o', b'f', b'f', b'l', b'i', b'n', b'e'][..9]);
        assert_eq!(encode_connect_with_lwt(&mut buf[..10], "kh", 30, "a", b"b"), Err(()));

        // SUBSCRIBE
        let n = encode_subscribe(&mut buf, 1, "karhutla/cmd").unwrap();
        assert_eq!(&buf[..n], &[&[0x82u8, 17, 0, 1, 0, 12][..], b"karhutla/cmd", &[0]].concat()[..]);

        // QoS 0 and QoS 1 parsing
        let q0 = [&[0u8, 1][..], b"a", b"xyz"].concat();
        assert_eq!(parse_publish(0x30, &q0), Some(("a", &b"xyz"[..])));
        let q1 = [&[0u8, 1][..], b"a", &[0x12, 0x34], b"xyz"].concat();
        assert_eq!(parse_publish(0x32, &q1), Some(("a", &b"xyz"[..])));
        assert_eq!(parse_publish(0x32, &[0, 1, b'a', 0x12]), None);
        assert_eq!(parse_publish(0x30, &[0, 9, b'a']), None);
        assert_eq!(parse_publish(0x20, &q0), None);
        assert_eq!(parse_publish(0x36, &q1), None);
    }
}
