use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};

const UTF16_CONTROL_SAMPLE_CHARS: usize = 4 * 1024;
const MAX_UTF16_CONTROL_CHARS: usize = 4;

pub(crate) struct DecodedText {
    pub(crate) text: String,
    pub(crate) encoding: &'static str,
}

pub(crate) fn decode_text(bytes: &[u8], truncated: bool) -> Result<DecodedText, String> {
    if let Some((encoding, bom_length)) = Encoding::for_bom(bytes) {
        let text = decode_prefix(encoding, &bytes[bom_length..], truncated);
        ensure_no_nulls(&text)?;
        if is_utf16(encoding) && has_excessive_control_chars(&text) {
            return Err("binary content is not text".into());
        }
        return Ok(DecodedText {
            text,
            encoding: if encoding.name() == "UTF-8" {
                "UTF-8 BOM"
            } else {
                encoding.name()
            },
        });
    }
    if let Some(encoding) = guess_utf16(bytes) {
        let text = decode_prefix(encoding, bytes, truncated);
        ensure_no_nulls(&text)?;
        if has_excessive_control_chars(&text) {
            return Err("binary content is not text".into());
        }
        return Ok(DecodedText {
            text,
            encoding: encoding.name(),
        });
    }
    if bytes.contains(&0) {
        return Err("binary content is not text".into());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            return Ok(DecodedText {
                text: text.to_owned(),
                encoding: "UTF-8",
            });
        }
        Err(error) if truncated && error.error_len().is_none() => {
            return Ok(DecodedText {
                text: String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned(),
                encoding: "UTF-8",
            });
        }
        Err(_) => {}
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, !truncated);
    let guessed = detector.guess(None, true);
    Ok(DecodedText {
        text: decode_prefix(guessed, bytes, truncated),
        encoding: guessed.name(),
    })
}

fn ensure_no_nulls(text: &str) -> Result<(), String> {
    if text.contains('\0') {
        Err("binary content is not text".into())
    } else {
        Ok(())
    }
}

fn is_utf16(encoding: &'static Encoding) -> bool {
    matches!(encoding.name(), "UTF-16LE" | "UTF-16BE")
}

fn has_excessive_control_chars(text: &str) -> bool {
    text.chars()
        .take(UTF16_CONTROL_SAMPLE_CHARS)
        .filter(|character| {
            character.is_control() && !matches!(character, '\n' | '\r' | '\t' | '\u{1b}' | '\u{c}')
        })
        .take(MAX_UTF16_CONTROL_CHARS + 1)
        .count()
        > MAX_UTF16_CONTROL_CHARS
}

// A non-final decoder buffers a split UTF-8/UTF-16/legacy character instead of
// displaying a replacement character at a truncated-preview boundary.
fn decode_prefix(encoding: &'static Encoding, bytes: &[u8], truncated: bool) -> String {
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let capacity = decoder
        .max_utf8_buffer_length(bytes.len())
        .expect("bounded text file size");
    let mut text = String::with_capacity(capacity);
    let (_, read, _) = decoder.decode_to_string(bytes, &mut text, !truncated);
    debug_assert_eq!(read, bytes.len());
    text
}

fn guess_utf16(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.len() < 8 {
        return None;
    }
    let pairs = bytes.len() / 2;
    let even_zeroes = bytes.iter().step_by(2).filter(|byte| **byte == 0).count();
    let odd_zeroes = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    if odd_zeroes * 10 > pairs * 4 && even_zeroes * 50 < pairs {
        Some(UTF_16LE)
    } else if even_zeroes * 10 > pairs * 4 && odd_zeroes * 50 < pairs {
        Some(UTF_16BE)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_character_at_eof_is_not_discarded_as_split_utf8() {
        assert_eq!(decode_text(b"caf\xe9", false).unwrap().text, "café");
    }

    #[test]
    fn buffers_split_characters_only_for_truncated_input() {
        assert_eq!(
            decode_text(b"sample \xe2\x82", true).unwrap().text,
            "sample "
        );
        let bytes = [0xff, 0xfe, b'a', 0, 0x3d, 0xd8];
        assert_eq!(decode_text(&bytes, true).unwrap().text, "a");
        assert_eq!(decode_text(&bytes, false).unwrap().text, "a\u{fffd}");
    }

    #[test]
    fn bom_and_bomless_utf16_are_decoded() {
        assert_eq!(
            decode_text(b"\xef\xbb\xbfhello", false).unwrap().text,
            "hello"
        );
        assert_eq!(
            decode_text(b"h\0e\0l\0l\0o\0", false).unwrap().text,
            "hello"
        );
    }

    #[test]
    fn utf16_log_content_is_not_rejected_as_binary() {
        let decoded = decode_text(b"\xff\xfeL\0o\0g\0 \0l\0i\0n\0e\0\r\0\n\0", false).unwrap();
        assert_eq!(decoded.text, "Log line\r\n");
        assert_eq!(decoded.encoding, "UTF-16LE");
    }

    #[test]
    fn bom_utf8_with_embedded_nulls_is_rejected_as_binary() {
        assert!(decode_text(b"\xef\xbb\xbftext\0binary", false).is_err());
    }

    #[test]
    fn utf16_control_data_is_rejected_as_binary() {
        let controls: Vec<u8> = (1u16..=20).flat_map(u16::to_le_bytes).collect();
        assert!(decode_text(&controls, false).is_err());

        let mut bom_controls = vec![0xff, 0xfe];
        bom_controls.extend(controls);
        assert!(decode_text(&bom_controls, false).is_err());
    }

    #[test]
    fn truncated_legacy_multibyte_character_is_buffered() {
        assert_eq!(
            decode_prefix(encoding_rs::SHIFT_JIS, b"sample \x82", true),
            "sample "
        );
        assert_eq!(decode_prefix(encoding_rs::UTF_8, b"\xf0\x9f", true), "");
    }
}
