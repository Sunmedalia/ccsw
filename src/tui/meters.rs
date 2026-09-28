use super::*;

// Independent display typography; no changes to the host's font or Nerd Font dependency.
pub(super) fn digits(text: &str) -> [String; 3] {
    let mut lines = [String::new(), String::new(), String::new()];
    for c in text.chars() {
        let glyph = match c {
            '0' => ["█▀█", "█ █", "█▄█"],
            '1' => [" ▄█", "  █", "  █"],
            '2' => ["▀▀█", "█▀▀", "█▄▄"],
            '3' => ["▀▀█", " ▀█", "▄▄█"],
            '4' => ["█ █", "▀▀█", "  █"],
            '5' => ["█▀▀", "▀▀█", "▄▄█"],
            '6' => ["█▀▀", "█▀█", "█▄█"],
            '7' => ["▀▀█", "  █", "  █"],
            '8' => ["█▀█", "█▀█", "█▄█"],
            '9' => ["█▀█", "▀▀█", "▄▄█"],
            '.' => ["   ", "   ", " ▪ "],
            'K' => ["█ █", "██ ", "█ █"],
            'M' => ["█▄█", "█▀█", "█ █"],
            'B' => ["█▀▄", "█▀▄", "█▄▀"],
            _ => ["   ", "━━━", "   "],
        };
        for i in 0..3 {
            lines[i].push_str(glyph[i]);
            lines[i].push(' ');
        }
    }
    lines
}

pub(super) fn progress_spans(
    fraction: Option<f64>,
    cells: usize,
    color: Color,
    track: Color,
) -> Vec<Span<'static>> {
    let fraction = fraction
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let units = (fraction * cells as f64 * 8.0).floor() as usize;
    let full = units / 8;
    let partial = units % 8;
    let mut fill = "█".repeat(full);
    if partial > 0 {
        fill.push([' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'][partial]);
    }
    vec![
        Span::styled(fill, Style::default().fg(color)),
        Span::styled(
            "░".repeat(cells.saturating_sub(full + usize::from(partial > 0))),
            Style::default().fg(track),
        ),
    ]
}
