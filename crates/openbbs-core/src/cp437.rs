//! CP437 (the original IBM PC character set) to Unicode conversion, so classic
//! ANSI art displays correctly in modern UTF-8 terminals.

/// Unicode for CP437 bytes 0x80..=0xFF.
const HIGH: [char; 128] = [
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E5}', '\u{00E7}',
    '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00EF}', '\u{00EE}', '\u{00EC}', '\u{00C4}', '\u{00C5}',
    '\u{00C9}', '\u{00E6}', '\u{00C6}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00FB}', '\u{00F9}',
    '\u{00FF}', '\u{00D6}', '\u{00DC}', '\u{00A2}', '\u{00A3}', '\u{00A5}', '\u{20A7}', '\u{0192}',
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{00F1}', '\u{00D1}', '\u{00AA}', '\u{00BA}',
    '\u{00BF}', '\u{2310}', '\u{00AC}', '\u{00BD}', '\u{00BC}', '\u{00A1}', '\u{00AB}', '\u{00BB}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{255C}', '\u{255B}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{255E}', '\u{255F}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256B}',
    '\u{256A}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{258C}', '\u{2590}', '\u{2580}',
    '\u{03B1}', '\u{00DF}', '\u{0393}', '\u{03C0}', '\u{03A3}', '\u{03C3}', '\u{00B5}', '\u{03C4}',
    '\u{03A6}', '\u{0398}', '\u{03A9}', '\u{03B4}', '\u{221E}', '\u{03C6}', '\u{03B5}', '\u{2229}',
    '\u{2261}', '\u{00B1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00F7}', '\u{2248}',
    '\u{00B0}', '\u{2219}', '\u{00B7}', '\u{221A}', '\u{207F}', '\u{00B2}', '\u{25A0}', '\u{00A0}',
];

/// Glyphs CP437 art uses for bytes 0x01..=0x1F. Control bytes that ANSI art relies
/// on (tab, newline, carriage return, escape) are passed through instead.
const LOW: [char; 32] = [
    '\u{0000}', '\u{263A}', '\u{263B}', '\u{2665}', '\u{2666}', '\u{2663}', '\u{2660}', '\u{2022}',
    '\u{25D8}', '\u{25CB}', '\u{25D9}', '\u{2642}', '\u{2640}', '\u{266A}', '\u{266B}', '\u{263C}',
    '\u{25BA}', '\u{25C4}', '\u{2195}', '\u{203C}', '\u{00B6}', '\u{00A7}', '\u{25AC}', '\u{21A8}',
    '\u{2191}', '\u{2193}', '\u{2192}', '\u{2190}', '\u{221F}', '\u{2194}', '\u{25B2}', '\u{25BC}',
];

/// Convert one CP437 byte to a Unicode character.
pub fn to_char(byte: u8) -> char {
    match byte {
        0x00 | b'\t' | b'\n' | b'\r' | 0x1B => byte as char,
        0x01..=0x1F => LOW[byte as usize],
        0x7F => '\u{2302}',
        0x80..=0xFF => HIGH[(byte - 0x80) as usize],
        _ => byte as char,
    }
}

/// Converts output from a running CP437 program (a door) for a modern
/// terminal. Unlike art files, programs use some control bytes as controls:
/// bell and backspace pass through, and form feed, the DOS way to clear the
/// screen, becomes an ANSI clear and home.
pub fn program_output(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x07 | 0x08 => out.push(b as char),
            0x0C => out.push_str("\x1b[2J\x1b[H"),
            _ => out.push(to_char(b)),
        }
    }
    out
}

/// Convert CP437 text (for example a `.ans` file) to a UTF-8 string. Stops at
/// the DOS end-of-file marker (0x1A), which also drops any SAUCE metadata record.
pub fn decode(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0x1A).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| to_char(b)).collect()
}

/// Decode a screen file: UTF-8 if it is valid UTF-8 and contains non-ASCII text
/// written that way, otherwise CP437.
pub fn decode_screen(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0x1A).unwrap_or(bytes.len());
    match std::str::from_utf8(&bytes[..end]) {
        Ok(text) if !text.is_ascii() => text.to_string(),
        _ => decode(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_drawing_and_blocks() {
        assert_eq!(decode(&[0xC9, 0xCD, 0xBB]), "╔═╗");
        assert_eq!(decode(&[0xDB, 0xB0, 0xB1, 0xB2]), "█░▒▓");
        assert_eq!(to_char(0x81), 'ü');
    }

    #[test]
    fn keeps_ansi_escapes_and_newlines() {
        assert_eq!(decode(b"\x1b[1;31mHi\r\n"), "\x1b[1;31mHi\r\n");
    }

    #[test]
    fn stops_at_sauce_marker() {
        assert_eq!(decode(b"art\x1aSAUCE00junk"), "art");
    }

    #[test]
    fn utf8_screens_pass_through() {
        assert_eq!(decode_screen("╔═╗".as_bytes()), "╔═╗");
        assert_eq!(decode_screen(&[0xC9, 0xCD, 0xBB]), "╔═╗");
    }

    #[test]
    fn program_output_handles_controls() {
        assert_eq!(program_output(b"\x0cHi\x07\x08"), "\x1b[2J\x1b[HHi\x07\x08");
        assert_eq!(program_output(&[0xDB, 0x01]), "\u{2588}\u{263A}");
    }
}
