//! Friend codes: short, human-relayable identifiers the server assigns
//! to each connection.

/// The alphabet omits characters that are easy to confuse when spoken
/// or handwritten: 0/O, 1/I/L.
pub const ALPHABET: &[u8; 31] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

pub const CODE_LEN: usize = 6;

/// An ephemeral friend code. Always stored in canonical form (uppercase,
/// alphabet characters only).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FriendCode([u8; CODE_LEN]);

impl FriendCode {
    /// Builds a code from arbitrary bytes, mapping each onto the
    /// alphabet. Useful for turning random bytes into a code.
    pub fn from_entropy(entropy: [u8; CODE_LEN]) -> Self {
        let mut code = [0u8; CODE_LEN];
        for (out, byte) in code.iter_mut().zip(entropy) {
            *out = ALPHABET[byte as usize % ALPHABET.len()];
        }
        Self(code)
    }

    /// Parses user input: trims surrounding whitespace and uppercases.
    /// Characters outside the alphabet (including the excluded 0/O and
    /// 1/I/L) are rejected rather than guessed at.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let mut code = [0u8; CODE_LEN];
        let mut len = 0;
        for c in text.chars() {
            let upper = c.to_ascii_uppercase() as u8;
            if !ALPHABET.contains(&upper) {
                return None;
            }
            if len == CODE_LEN {
                return None;
            }
            code[len] = upper;
            len += 1;
        }
        if len != CODE_LEN {
            return None;
        }
        Some(Self(code))
    }

    /// The canonical wire form.
    pub fn as_bytes(&self) -> &[u8; CODE_LEN] {
        &self.0
    }

    pub fn from_wire(bytes: [u8; CODE_LEN]) -> Option<Self> {
        if bytes.iter().all(|b| ALPHABET.contains(b)) {
            Some(Self(bytes))
        } else {
            None
        }
    }
}

impl std::fmt::Display for FriendCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(std::str::from_utf8(&self.0).expect("alphabet is ASCII"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_codes_are_canonical() {
        let code = FriendCode::from_entropy([0, 30, 31, 100, 200, 255]);
        assert_eq!(FriendCode::parse(&code.to_string()), Some(code));
    }

    #[test]
    fn parse_normalizes_case_and_whitespace() {
        let code = FriendCode::parse("  abcdef ").unwrap();
        assert_eq!(code.to_string(), "ABCDEF");
    }

    #[test]
    fn ambiguous_and_invalid_input_is_rejected() {
        assert_eq!(FriendCode::parse("ABC0EF"), None); // 0 not in alphabet
        assert_eq!(FriendCode::parse("ABC1EF"), None);
        assert_eq!(FriendCode::parse("ABCDE"), None); // too short
        assert_eq!(FriendCode::parse("ABCDEFG"), None); // too long
        assert_eq!(FriendCode::parse(""), None);
    }

    #[test]
    fn wire_roundtrip() {
        let code = FriendCode::parse("QWERTY").unwrap();
        assert_eq!(FriendCode::from_wire(*code.as_bytes()), Some(code));
        assert_eq!(FriendCode::from_wire(*b"qwerty"), None); // not canonical
    }
}
