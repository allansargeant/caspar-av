//! A minimal OSC 1.0 decoder — enough for what CasparCG actually emits.
//!
//! The server sends one UDP packet per frame containing a `#bundle` of
//! messages, each addressed like `/channel/1/stage/layer/10/file/frame` with a
//! single argument. There is no need for pattern matching, timetag scheduling
//! or any of OSC's dispatch machinery here: we are a sink, not a target.

/// An OSC argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    Blob(Vec<u8>),
    Bool(bool),
    Null,
    Impulse,
}

/// One decoded message.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub address: String,
    pub args: Vec<Value>,
}

/// Why a packet could not be read.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("packet ended mid-value")]
    Truncated,
    #[error("string was not null-terminated")]
    Unterminated,
    #[error("unknown OSC type tag {0:?}")]
    UnknownTag(char),
    #[error("address did not start with '/'")]
    BadAddress,
}

/// Decode a UDP packet into its messages, flattening any nested bundles.
///
/// Timetags are ignored: CasparCG stamps bundles with the immediate tag, and a
/// telemetry mirror wants the newest value regardless.
pub fn decode_packet(bytes: &[u8]) -> Result<Vec<Message>, Error> {
    let mut out = Vec::new();
    decode_into(bytes, &mut out)?;
    Ok(out)
}

fn decode_into(bytes: &[u8], out: &mut Vec<Message>) -> Result<(), Error> {
    if bytes.starts_with(b"#bundle\0") {
        // 8 bytes of "#bundle\0", 8 of timetag, then length-prefixed elements.
        let mut r = Reader::new(&bytes[16..]);
        while !r.is_empty() {
            let len = r.i32()? as usize;
            let element = r.take(len)?;
            decode_into(element, out)?;
        }
        Ok(())
    } else {
        out.push(decode_message(bytes)?);
        Ok(())
    }
}

fn decode_message(bytes: &[u8]) -> Result<Message, Error> {
    let mut r = Reader::new(bytes);
    let address = r.string()?;
    if !address.starts_with('/') {
        return Err(Error::BadAddress);
    }

    // A message with no type-tag string is legal in OSC 1.0 and means no args.
    let tags = if r.is_empty() { String::new() } else { r.string()? };
    let tags = tags.strip_prefix(',').unwrap_or(&tags).to_string();

    let mut args = Vec::with_capacity(tags.len());
    for tag in tags.chars() {
        args.push(match tag {
            'i' => Value::Int(r.i32()?),
            'h' => Value::Long(r.i64()?),
            'f' => Value::Float(f32::from_bits(r.i32()? as u32)),
            'd' => Value::Double(f64::from_bits(r.i64()? as u64)),
            's' | 'S' => Value::String(r.string()?),
            'b' => {
                let len = r.i32()? as usize;
                let data = r.take(len)?.to_vec();
                r.pad_to_4(len)?;
                Value::Blob(data)
            }
            'T' => Value::Bool(true),
            'F' => Value::Bool(false),
            'N' => Value::Null,
            'I' => Value::Impulse,
            other => return Err(Error::UnknownTag(other)),
        });
    }

    Ok(Message { address, args })
}

/// A cursor over an OSC packet. Everything in OSC is 4-byte aligned.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let slice = self.buf.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn i32(&mut self) -> Result<i32, Error> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self) -> Result<i64, Error> {
        let b = self.take(8)?;
        Ok(i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// A null-terminated string padded to the next 4-byte boundary.
    fn string(&mut self) -> Result<String, Error> {
        let rest = self.buf.get(self.pos..).ok_or(Error::Truncated)?;
        let nul = rest.iter().position(|&b| b == 0).ok_or(Error::Unterminated)?;
        let s = String::from_utf8_lossy(&rest[..nul]).into_owned();
        // Advance past the string, its terminator, and the alignment padding.
        let consumed = (nul + 1).div_ceil(4) * 4;
        self.pos += consumed.min(rest.len());
        Ok(s)
    }

    /// Skip the padding that follows a blob of `len` bytes.
    fn pad_to_4(&mut self, len: usize) -> Result<(), Error> {
        let pad = (4 - (len % 4)) % 4;
        if pad > 0 {
            self.take(pad)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an OSC string: contents, a null, then padding to 4 bytes.
    fn ostr(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
        v
    }

    fn message(addr: &str, tags: &str, payload: &[u8]) -> Vec<u8> {
        let mut v = ostr(addr);
        v.extend(ostr(&format!(",{tags}")));
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn decodes_an_int_message() {
        let pkt = message("/channel/1/stage/layer/10/file/frame", "i", &1234i32.to_be_bytes());
        let msgs = decode_packet(&pkt).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].address, "/channel/1/stage/layer/10/file/frame");
        assert_eq!(msgs[0].args, vec![Value::Int(1234)]);
    }

    #[test]
    fn decodes_mixed_argument_types() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&7i32.to_be_bytes());
        payload.extend_from_slice(&0.5f32.to_bits().to_be_bytes());
        payload.extend(ostr("AMB"));
        let pkt = message("/channel/1/x", "ifsTF", &payload);
        let msgs = decode_packet(&pkt).unwrap();
        assert_eq!(
            msgs[0].args,
            vec![
                Value::Int(7),
                Value::Float(0.5),
                Value::String("AMB".into()),
                Value::Bool(true),
                Value::Bool(false),
            ]
        );
    }

    #[test]
    fn flattens_a_bundle() {
        let a = message("/a", "i", &1i32.to_be_bytes());
        let b = message("/b", "i", &2i32.to_be_bytes());
        let mut pkt = b"#bundle\0".to_vec();
        pkt.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]); // immediate timetag
        pkt.extend_from_slice(&(a.len() as i32).to_be_bytes());
        pkt.extend_from_slice(&a);
        pkt.extend_from_slice(&(b.len() as i32).to_be_bytes());
        pkt.extend_from_slice(&b);

        let msgs = decode_packet(&pkt).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].address, "/a");
        assert_eq!(msgs[1].address, "/b");
        assert_eq!(msgs[1].args, vec![Value::Int(2)]);
    }

    #[test]
    fn a_message_with_no_type_tags_has_no_args() {
        let pkt = ostr("/ping");
        let msgs = decode_packet(&pkt).unwrap();
        assert_eq!(msgs[0].address, "/ping");
        assert!(msgs[0].args.is_empty());
    }

    #[test]
    fn truncated_packets_error_rather_than_panic() {
        // An 'i' promised but only two bytes supplied.
        let pkt = message("/a", "i", &[0, 0]);
        assert_eq!(decode_packet(&pkt), Err(Error::Truncated));
    }

    #[test]
    fn an_unterminated_address_errors() {
        assert_eq!(decode_packet(b"/abc"), Err(Error::Unterminated));
    }

    #[test]
    fn blobs_carry_their_padding() {
        let mut payload = (3i32).to_be_bytes().to_vec();
        payload.extend_from_slice(&[1, 2, 3, 0]); // 3 bytes + 1 pad
        payload.extend_from_slice(&9i32.to_be_bytes());
        let pkt = message("/a", "bi", &payload);
        let msgs = decode_packet(&pkt).unwrap();
        assert_eq!(msgs[0].args, vec![Value::Blob(vec![1, 2, 3]), Value::Int(9)]);
    }
}
