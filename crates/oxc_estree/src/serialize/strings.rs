use oxc_data_structures::code_buffer::CodeBuffer;

use super::{ESTree, Serializer};

/// Convert `char` to UTF-8 bytes array.
const fn to_bytes<const N: usize>(ch: char) -> [u8; N] {
    let mut bytes = [0u8; N];
    ch.encode_utf8(&mut bytes);
    bytes
}

/// Lossy replacement character (U+FFFD) as UTF-8 bytes.
const LOSSY_REPLACEMENT_CHAR_BYTES: [u8; 3] = to_bytes('\u{FFFD}');
const LOSSY_REPLACEMENT_CHAR_FIRST_BYTE: u8 = LOSSY_REPLACEMENT_CHAR_BYTES[0]; // 0xEF

/// A string which does not need any escaping in JSON.
///
/// This provides better performance when you know that the string definitely contains no characters
/// that require escaping, as it avoids the cost of checking that.
///
/// If the string does in fact contain characters that did need escaping, it will result in invalid JSON.
pub struct JsonSafeString<'s>(pub &'s str);

impl ESTree for JsonSafeString<'_> {
    #[inline(always)]
    fn serialize<S: Serializer>(&self, mut serializer: S) {
        let buffer = serializer.buffer_mut();
        buffer.print_ascii_byte(b'"');
        buffer.print_str(self.0);
        buffer.print_ascii_byte(b'"');
    }
}

/// A string which contains lone surrogates escaped with lossy replacement character (U+FFFD).
///
/// Lone surrogates are encoded in the string as `\uFFFD1234` where `1234` is the code point in hex.
/// These are converted to `\u1234` in JSON.
/// An actual lossy replacement character is encoded in the string as `\uFFFDfffd`, and is converted
/// to the actual character.
pub struct LoneSurrogatesString<'s>(pub &'s str);

impl ESTree for LoneSurrogatesString<'_> {
    #[inline(always)]
    fn serialize<S: Serializer>(&self, mut serializer: S) {
        write_str(self.0, &ESCAPE_LONE_SURROGATES, serializer.buffer_mut());
    }
}

/// [`ESTree`] implementation for string slice.
impl ESTree for str {
    fn serialize<S: Serializer>(&self, mut serializer: S) {
        write_str(self, &ESCAPE, serializer.buffer_mut());
    }
}

/// [`ESTree`] implementation for `String`.
impl ESTree for String {
    fn serialize<S: Serializer>(&self, serializer: S) {
        self.as_str().serialize(serializer);
    }
}

/// Escapes
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Escape {
    __ = 0,
    BB = b'b',  // \x08
    TT = b't',  // \x09
    NN = b'n',  // \x0A
    FF = b'f',  // \x0C
    RR = b'r',  // \x0D
    QU = b'"',  // \x22
    BS = b'\\', // \x5C
    LO = LOSSY_REPLACEMENT_CHAR_FIRST_BYTE,
    UU = b'u', // \x00...\x1F except the ones above
}

/// Lookup table of escape sequences. A value of `b'x'` at index `i` means that byte `i`
/// is escaped as "\x" in JSON. A value of 0 means that byte `i` is not escaped.
///
/// A value of `UU` means that byte is escaped as `\u00xx`, where `xx` is the hex code of the byte.
/// e.g. `0x1F` is output as `\u001F`.
static ESCAPE: [Escape; 256] = create_table(Escape::__);

/// Same as `ESCAPE` but with `Escape::LO` for byte 0xEF.
static ESCAPE_LONE_SURROGATES: [Escape; 256] = create_table(Escape::LO);

const fn create_table(lo: Escape) -> [Escape; 256] {
    #[allow(clippy::enum_glob_use, clippy::allow_attributes)]
    use Escape::*;

    [
        //   1   2   3   4   5   6   7   8   9   A   B   C   D   E   F
        UU, UU, UU, UU, UU, UU, UU, UU, BB, TT, NN, UU, FF, RR, UU, UU, // 0
        UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, UU, // 1
        __, __, QU, __, __, __, __, __, __, __, __, __, __, __, __, __, // 2
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 3
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 4
        __, __, __, __, __, __, __, __, __, __, __, __, BS, __, __, __, // 5
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 6
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 7
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 8
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // 9
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // A
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // B
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // C
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // D
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, lo, // E
        __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, __, // F
    ]
}

/// Write string to buffer.
/// String is wrapped in `"`s, and with any characters which are not valid in JSON escaped.
//
// `#[inline(always)]` because this is a hot path, and to make compiler remove the code
// for handling lone surrogates when outputting a normal string (the common case).
#[inline(always)]
fn write_str(s: &str, table: &[Escape; 256], buffer: &mut CodeBuffer) {
    buffer.print_ascii_byte(b'"');

    let bytes = s.as_bytes();

    let mut start = 0;
    let mut iter = bytes.iter().enumerate();
    while let Some((index, &byte)) = iter.next() {
        let escape = table[byte as usize];
        if escape == Escape::__ {
            continue;
        }

        // Handle lone surrogates
        if table == &ESCAPE_LONE_SURROGATES && escape == Escape::LO {
            let (_, &next1) = iter.next().unwrap();
            let (_, &next2) = iter.next().unwrap();
            if [next1, next2] == [LOSSY_REPLACEMENT_CHAR_BYTES[1], LOSSY_REPLACEMENT_CHAR_BYTES[2]]
            {
                // Lossy replacement character (U+FFFD) is used as an escape before lone surrogates,
                // with the code point as 4 x hex characters after it.
                let (_, &hex1) = iter.next().unwrap();
                let (_, &hex2) = iter.next().unwrap();
                let (_, &hex3) = iter.next().unwrap();
                let (_, &hex4) = iter.next().unwrap();
                let hex = [hex1, hex2, hex3, hex4];

                // Print the chunk upto before the lossy replacement character.
                // SAFETY: 0xEF is always the start of a 3-byte unicode character.
                // Therefore `index` must be on a UTF-8 character boundary.
                unsafe { buffer.print_bytes_unchecked(&bytes[start..index]) };

                if hex == *b"fffd" {
                    // This is an actual lossy replacement character (not an escaped lone surrogate)
                    buffer.print_str("\u{FFFD}");
                } else {
                    // This is an escaped lone surrogate.
                    // Print `\uXXXX` where `XXXX` is hex characters. e.g. `\ud800`.
                    buffer.print_str("\\u");
                    // Check all 4 hex bytes are ASCII
                    assert_eq!(u32::from_ne_bytes(hex) & 0x8080_8080, 0);
                    // SAFETY: Just checked all 4 bytes are ASCII
                    unsafe { buffer.print_bytes_unchecked(&hex) };
                }

                // Skip the 3 bytes of the lossy replacement character + 4 hex bytes.
                // We checked that all 4 hex bytes are ASCII, so `start` is definitely left on
                // a UTF-8 character boundary.
                start = index + 7;
            } else {
                // Some other unicode character starting with 0xEF. Just continue the loop.
            }
            continue;
        }

        if start < index {
            // SAFETY: `bytes` is derived from a `&str`.
            // `escape` is only non-zero for ASCII bytes, except `Escape::LO` which is handled above.
            // Therefore current `index` must mark the end of a valid UTF8 character sequence.
            // `start` is either the start of string, or after an ASCII character,
            // therefore always the start of a valid UTF8 character sequence.
            unsafe { buffer.print_bytes_unchecked(&bytes[start..index]) };
        }

        write_char_escape(escape, byte, buffer);

        start = index + 1;
    }

    if start < bytes.len() {
        // SAFETY: `bytes` is derived from a `&str`.
        // `start` is either the start of string, or after an ASCII character,
        // therefore always the start of a valid UTF8 character sequence.
        unsafe { buffer.print_bytes_unchecked(&bytes[start..]) };
    }

    buffer.print_ascii_byte(b'"');
}

fn write_char_escape(escape: Escape, byte: u8, buffer: &mut CodeBuffer) {
    #[expect(clippy::if_not_else)]
    if escape != Escape::UU {
        buffer.print_ascii_byte(b'\\');
        // SAFETY: All values of `Escape` are ASCII
        unsafe { buffer.print_byte_unchecked(escape as u8) };
    } else {
        static HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";
        let bytes = [
            b'\\',
            b'u',
            b'0',
            b'0',
            HEX_DIGITS[(byte >> 4) as usize],
            HEX_DIGITS[(byte & 0xF) as usize],
        ];
        // SAFETY: `bytes` contains only ASCII bytes
        unsafe { buffer.print_bytes_unchecked(&bytes) }
    }
}

#[cfg(test)]
mod tests {
    use super::super::CompactTSSerializer;
    use super::*;

    #[test]
    fn serialize_string_slice() {
        let cases = [
            ("", r#""""#),
            ("foobar", r#""foobar""#),
            ("\n", r#""\n""#),
            ("\nfoobar", r#""\nfoobar""#),
            ("foo\nbar", r#""foo\nbar""#),
            ("foobar\n", r#""foobar\n""#),
            (
                "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x0F",
                r#""\u0000\u0001\u0002\u0003\u0004\u0005\u0006\u0007\b\t\n\u000b\f\r\u000e\u000f""#,
            ),
            (
                "\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1A\x1B\x1C\x1D\x1E\x1F",
                r#""\u0010\u0011\u0012\u0013\u0014\u0015\u0016\u0017\u0018\u0019\u001a\u001b\u001c\u001d\u001e\u001f""#,
            ),
            (
                r#"They call me "Bob" but I prefer "Dennis", innit?"#,
                r#""They call me \"Bob\" but I prefer \"Dennis\", innit?""#,
            ),
        ];

        for (input, output) in cases {
            let mut serializer = CompactTSSerializer::new();
            input.serialize(&mut serializer);
            let s = serializer.into_string();
            assert_eq!(&s, output);
        }
    }

    #[test]
    fn serialize_string() {
        let cases = [(String::new(), r#""""#), ("foobar".to_string(), r#""foobar""#)];

        for (input, output) in cases {
            let mut serializer = CompactTSSerializer::new();
            input.to_string().serialize(&mut serializer);
            let s = serializer.into_string();
            assert_eq!(&s, output);
        }
    }

    #[test]
    fn serialize_json_safe_string() {
        let cases = [("", r#""""#), ("a", r#""a""#), ("abc", r#""abc""#)];

        for (input, output) in cases {
            let mut serializer = CompactTSSerializer::new();
            JsonSafeString(input).serialize(&mut serializer);
            let s = serializer.into_string();
            assert_eq!(&s, output);
        }
    }

    #[test]
    fn serialize_lone_surrogates_string() {
        let cases = [
            ("\u{FFFD}fffd", "\"\u{FFFD}\""),
            ("_x_\u{FFFD}fffd_y_\u{FFFD}fffd_z_", "\"_x_\u{FFFD}_y_\u{FFFD}_z_\""),
            ("\u{FFFD}d834\u{FFFD}d835", r#""\ud834\ud835""#),
            ("_x_\u{FFFD}d834\u{FFFD}d835", r#""_x_\ud834\ud835""#),
            ("\u{FFFD}d834\u{FFFD}d835_y_", r#""\ud834\ud835_y_""#),
            ("_x_\u{FFFD}d834_y_\u{FFFD}d835_z_", r#""_x_\ud834_y_\ud835_z_""#),
        ];

        for (input, output) in cases {
            let mut serializer = CompactTSSerializer::new();
            LoneSurrogatesString(input).serialize(&mut serializer);
            let s = serializer.into_string();
            assert_eq!(&s, output);
        }
    }
}
