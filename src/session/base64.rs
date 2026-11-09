//! Base64 (RFC 4648, standard alphabet, padded), for putting a picture in a JSON
//! reply and an MCP image block. Small enough to write than to depend on.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `bytes`.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn the_rfc_4648_test_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), encoded, "{plain:?}");
        }
    }

    #[test]
    fn every_byte_value_uses_only_the_alphabet() {
        let all: Vec<u8> = (0..=255).collect();
        let encoded = encode(&all);
        assert!(encoded.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)));
        assert_eq!(encoded.len(), 256usize.div_ceil(3) * 4);
    }

    #[test]
    fn a_png_header_encodes_to_the_well_known_prefix() {
        assert!(encode(b"\x89PNG\r\n\x1a\n").starts_with("iVBORw0KGgo"));
    }
}
