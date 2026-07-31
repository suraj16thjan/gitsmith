use ratatui::style::Color;

/// Parse text containing ANSI SGR escapes into styled rows (one `Vec` of
/// `(color, text)` spans per line). Recognizes reset (0) and basic/bright
/// foreground colors (30-37 / 90-97); other SGR params (bold, bg, underline)
/// are ignored, and non-SGR CSI sequences (cursor/erase) are stripped so they
/// don't render as garbage. ponytail: basic SGR only, no 256/truecolor.
pub fn parse(text: &str) -> Vec<Vec<(Color, String)>> {
    let bytes = text.as_bytes();
    let mut rows: Vec<Vec<(Color, String)>> = Vec::new();
    let mut row: Vec<(Color, String)> = Vec::new();
    let mut cur = Color::Reset;
    let mut buf: Vec<u8> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            flush(&mut buf, &mut row, cur);
            // Read the CSI param bytes up to the final byte (0x40..=0x7e).
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() {
                if bytes[j] == b'm' {
                    apply_sgr(&bytes[i + 2..j], &mut cur);
                }
                i = j + 1; // consume the whole sequence (SGR or stripped)
            } else {
                i = j; // unterminated escape: drop the remainder
            }
            continue;
        }
        if b == b'\n' {
            flush(&mut buf, &mut row, cur);
            if row.is_empty() {
                row.push((Color::Reset, String::new())); // keep blank lines
            }
            rows.push(std::mem::take(&mut row));
            i += 1;
            continue;
        }
        if b == b'\r' {
            i += 1; // drop bare CR
            continue;
        }
        buf.push(b);
        i += 1;
    }
    flush(&mut buf, &mut row, cur);
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn flush(buf: &mut Vec<u8>, row: &mut Vec<(Color, String)>, color: Color) {
    if !buf.is_empty() {
        row.push((color, String::from_utf8_lossy(buf).into_owned()));
        buf.clear();
    }
}

fn apply_sgr(params: &[u8], cur: &mut Color) {
    let s = std::str::from_utf8(params).unwrap_or("");
    if s.is_empty() {
        *cur = Color::Reset; // ESC[m == ESC[0m
        return;
    }
    for p in s.split(';') {
        match p.parse::<u8>() {
            Ok(0) => *cur = Color::Reset,
            Ok(code) if (30..=37).contains(&code) => *cur = basic(code - 30),
            Ok(code) if (90..=97).contains(&code) => *cur = bright(code - 90),
            _ => {} // bold/bg/underline/256/truecolor ignored (ponytail)
        }
    }
}

fn basic(n: u8) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

fn bright(n: u8) -> Color {
    match n {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_line_is_one_default_span() {
        let rows = parse("plain");
        assert_eq!(rows, vec![vec![(Color::Reset, "plain".to_string())]]);
    }

    #[test]
    fn colored_then_reset() {
        let rows = parse("\x1b[31mred\x1b[0m.");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], (Color::Red, "red".to_string()));
        assert_eq!(rows[0][1], (Color::Reset, ".".to_string()));
    }

    #[test]
    fn bright_and_bold_param() {
        // bold (1) ignored; bright green (92) applies
        let rows = parse("\x1b[1;92mok\x1b[0m");
        assert_eq!(rows[0][0], (Color::LightGreen, "ok".to_string()));
    }

    #[test]
    fn non_sgr_escape_is_stripped() {
        let rows = parse("\x1b[2Kfoo");
        assert_eq!(rows, vec![vec![(Color::Reset, "foo".to_string())]]);
    }

    #[test]
    fn multiple_lines() {
        let rows = parse("a\nb");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], (Color::Reset, "a".to_string()));
        assert_eq!(rows[1][0], (Color::Reset, "b".to_string()));
    }

    #[test]
    fn blank_line_preserved() {
        let rows = parse("a\n\nb");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec![(Color::Reset, String::new())]);
    }
}
