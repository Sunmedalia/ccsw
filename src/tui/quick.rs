//! Read-only monitor, independent of the configuration editor and its sync loop.
use super::*;
use crate::usage::{Query, Reader, Snapshot, Totals};
use chrono::Timelike;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) const BG: Color = Color::Rgb(20, 30, 42);
pub(super) const INK: Color = Color::Rgb(223, 233, 240);
pub(super) const SOFT: Color = Color::Rgb(139, 161, 181);
pub(super) const BLUE: Color = Color::Rgb(123, 190, 218);
pub(super) const GOLD: Color = Color::Rgb(234, 193, 126);
pub(super) const RED: Color = Color::Rgb(236, 139, 131);
pub(super) const GREEN: Color = Color::Rgb(147, 204, 178);
pub(super) const RAIL: Color = Color::Rgb(48, 67, 84);
const LABEL: &str = "CCSW Pulse";

fn initial_client(agent: Option<&str>) -> usize {
    match agent.unwrap_or("").to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" | "claude code" => 0,
        "codex" => 1,
        _ => 2,
    }
}

fn rightmost_split_target(layout: &serde_json::Value) -> Option<(&str, u64)> {
    let layout = &layout["result"]["layout"];
    let edge = layout["area"]["x"].as_u64()? + layout["area"]["width"].as_u64()?;
    layout["panes"]
        .as_array()?
        .iter()
        .filter_map(|pane| {
            let rect = &pane["rect"];
            let x = rect["x"].as_u64()?;
            let width = rect["width"].as_u64()?;
            let height = rect["height"].as_u64()?;
            if x + width == edge && width >= 76 {
                Some((pane["pane_id"].as_str()?, width, height))
            } else {
                None
            }
        })
        .max_by_key(|(_, width, height)| (*height, *width))
        .map(|(id, width, _)| (id, width))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentSession {
    client: &'static str,
    id: String,
}

fn agent_session(pane: &serde_json::Value) -> Option<AgentSession> {
    let pane = pane.get("result").map_or(pane, |result| &result["pane"]);
    let session = &pane["agent_session"];
    let client = match (pane["agent"].as_str()?, session["agent"].as_str()?) {
        ("codex", "codex") => "Codex",
        ("claude", "claude") => "Claude",
        _ => return None,
    };
    let value = session["value"].as_str()?;
    let id = match session["kind"].as_str()? {
        "id" => value,
        "path" => std::path::Path::new(value).file_stem()?.to_str()?,
        _ => return None,
    };
    (!id.is_empty()).then(|| AgentSession {
        client,
        id: id.into(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FocusedAgent {
    pane_id: String,
    client: usize,
    session: Option<AgentSession>,
}

fn focused_agent(panes: &serde_json::Value, tab_id: &str) -> Option<FocusedAgent> {
    panes["result"]["panes"]
        .as_array()?
        .iter()
        .find_map(|pane| {
            if pane["focused"] != true || pane["tab_id"].as_str()? != tab_id {
                return None;
            }
            let pane_id = pane["pane_id"].as_str()?;
            let client = initial_client(pane["agent"].as_str());
            (client != 2).then(|| FocusedAgent {
                pane_id: pane_id.into(),
                client,
                session: agent_session(pane),
            })
        })
}

fn focused_pane(
    pane: &serde_json::Value,
    tab_id: Option<&str>,
    source: &str,
) -> Option<FocusedAgent> {
    if tab_id.is_some_and(|tab| pane["tab_id"].as_str() != Some(tab))
        || (tab_id.is_none() && pane["pane_id"].as_str() != Some(source))
        || (tab_id.is_some() && pane["focused"] != true)
    {
        return None;
    }
    let pane_id = pane["pane_id"].as_str()?;
    let client = initial_client(pane["agent"].as_str());
    (client != 2).then(|| FocusedAgent {
        pane_id: pane_id.into(),
        client,
        session: agent_session(pane),
    })
}

#[derive(Clone, Debug)]
struct FocusUpdate {
    pane_id: String,
    client: usize,
    session: Option<AgentSession>,
}

#[derive(Default)]
struct FocusTracker {
    previous: Option<AgentSession>,
    pane: Option<String>,
    client: Option<usize>,
    missing: u8,
}

impl FocusTracker {
    fn observe(
        &mut self,
        focused: Option<FocusedAgent>,
        send: &mpsc::SyncSender<FocusUpdate>,
        event_driven: bool,
    ) -> bool {
        let Some(focused) = focused else { return true };
        let changed_focus = self.pane.as_deref() != Some(focused.pane_id.as_str())
            || self.client != Some(focused.client);
        if changed_focus {
            self.pane = Some(focused.pane_id.clone());
            self.client = Some(focused.client);
            self.previous = focused.session.clone();
            self.missing = 0;
            return send
                .send(FocusUpdate {
                    pane_id: focused.pane_id,
                    client: focused.client,
                    session: focused.session,
                })
                .is_ok();
        }
        // A full pane event is authoritative. The polling fallback waits for
        // three missing observations because CLI snapshots can be transient.
        if event_driven && focused.session.is_none() && self.previous.is_some() {
            self.missing = 2;
        }
        if let Some(session) =
            stable_agent_session(&mut self.previous, &mut self.missing, focused.session)
        {
            return send
                .send(FocusUpdate {
                    pane_id: focused.pane_id,
                    client: focused.client,
                    session,
                })
                .is_ok();
        }
        true
    }
}
fn stable_agent_session(
    previous: &mut Option<AgentSession>,
    missing: &mut u8,
    observed: Option<AgentSession>,
) -> Option<Option<AgentSession>> {
    if let Some(current) = observed {
        *missing = 0;
        if previous.as_ref() != Some(&current) {
            *previous = Some(current.clone());
            return Some(Some(current));
        }
    } else {
        *missing = missing.saturating_add(1);
        if *missing >= 3 && previous.take().is_some() {
            return Some(None);
        }
    }
    None
}

#[derive(Default)]
struct Monitor {
    pulse_theme: theme::PulseTheme,
    snapshot: Snapshot,
    sessions: crate::sessions::Snapshot,
    sessions_refreshed: Option<Instant>,
    sessions_mode: bool,
    chart_mode: bool,
    visual_mode: bool,
    roomy_visual: bool,
    sessions_sort_tokens: bool,
    source_pane: Option<String>,
    focused_pane: Option<String>,
    focused_client: Option<usize>,
    active_session: Option<AgentSession>,
    client: usize,
    scroll: u16,
    limit: u16,
    models: bool,
    help: bool,
    refreshed: Option<Instant>,
    error: Option<String>,
    notice: Option<String>,
}
#[derive(Default)]
struct Metrics {
    total: Totals,
    compact: i64,
    hours: [i64; 24],
    providers: BTreeMap<String, (String, Totals)>,
    models: BTreeMap<String, Totals>,
}
fn metrics(snapshot: &Snapshot, client: Option<&str>) -> Metrics {
    let today = snapshot.today();
    let mut m = Metrics::default();
    for row in &snapshot.rows {
        if row.day != today || client.is_some_and(|c| row.client != c) {
            continue;
        }
        m.total.add(&row.totals);
        if row.kind == "compact" {
            m.compact += row.totals.calls;
        }
        if let Some(hour) = m.hours.get_mut(row.hour as usize) {
            *hour += row.totals.calls;
        }
        let entry = m
            .providers
            .entry(format!("{}:{}", row.client, row.provider))
            .or_insert_with(|| (row.name.clone(), Totals::default()));
        entry.1.add(&row.totals);
        m.models
            .entry(row.model.clone())
            .or_default()
            .add(&row.totals);
    }
    m
}
fn rate(t: &Totals) -> Option<f64> {
    let completed = t.success + t.failed + t.interrupted;
    (completed > 0).then(|| 100.0 * t.success as f64 / completed as f64)
}
fn short(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}
fn token_label(t: &Totals) -> String {
    if t.calls > 0 && t.unknown == t.calls {
        return "unknown".into();
    }
    format!(
        "{}{}",
        short(t.input + t.output),
        if t.unknown > 0 { "+?" } else { "" }
    )
}
// Independent display typography; no changes to the host's font or Nerd Font dependency.
fn digits(text: &str) -> [String; 3] {
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
fn token_digits(value: &str, color: Color) -> Vec<Line<'static>> {
    digits(value)
        .into_iter()
        .map(|row| line(row, color))
        .collect()
}
fn mini_token_total(value: &str, width: u16, color: Color) -> Vec<Line<'static>> {
    if width < 2 {
        return vec![mini_line(value, width, color)];
    }
    let chars: Vec<char> = value.chars().collect();
    chars
        .chunks((usize::from(width) + 1) / 3)
        .flat_map(|chunk| {
            let mut rows = [String::new(), String::new(), String::new()];
            for (index, ch) in chunk.iter().enumerate() {
                // Two-column glyphs retain a middle row so all ten digits are distinct.
                let glyph = match ch {
                    '0' => ["▛▜", "▌▐", "▙▟"],
                    '1' => ["▗▌", " ▌", " ▌"],
                    '2' => ["▀▜", "▛▀", "▙▄"],
                    '3' => ["▀▜", " ▜", "▄▟"],
                    '4' => ["▌▐", "▀▜", " ▐"],
                    '5' => ["▛▀", "▀▜", "▄▟"],
                    '6' => ["▛▀", "▛▜", "▙▟"],
                    '7' => ["▀▜", " ▐", " ▐"],
                    '8' => ["▛▜", "▛▜", "▙▟"],
                    '9' => ["▛▜", "▀▜", "▄▟"],
                    '.' => [" ", " ", "▪"],
                    'K' => ["▌▞", "▛▖", "▌▚"],
                    'M' => ["▙▟", "▌▐", "▌▐"],
                    'B' => ["▛▖", "▛▖", "▙▘"],
                    '?' => ["▀▜", " ▘", " ▖"],
                    _ => ["  ", "━━", "  "],
                };
                for (row, part) in rows.iter_mut().zip(glyph) {
                    if index > 0 {
                        row.push(' ');
                    }
                    row.push_str(part);
                }
            }
            rows.into_iter().map(|row| line(row, color))
        })
        .collect()
}

fn line(text: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(Span::styled(text.into(), Style::default().fg(color)))
}
fn clipped(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    let mut result = String::new();
    for ch in text.chars() {
        if result.width() + ch.width().unwrap_or(0) + 1 > width {
            break;
        }
        result.push(ch);
    }
    result.push('…');
    result
}
fn pair(label: &str, value: impl Into<String>, width: u16, color: Color) -> Line<'static> {
    let value = value.into();
    let available = usize::from(width).saturating_sub(value.width() + 1);
    let mut used = 0;
    let label: String = label
        .chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= available
        })
        .collect();
    let gap = usize::from(width).saturating_sub(label.width() + value.width());
    Line::from(vec![
        Span::styled(label, Style::default().fg(SOFT)),
        Span::raw(" ".repeat(gap)),
        Span::styled(
            value,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}
fn duo_line(
    width: u16,
    left: (&str, &str, &str, Color),
    right: (&str, &str, &str, Color),
) -> Line<'static> {
    let full = format!("{} {}  ·  {} {}", left.0, left.2, right.0, right.2);
    let (left_label, right_label, separator) = if full.width() <= usize::from(width) {
        (left.0, right.0, "  ·  ")
    } else {
        (left.1, right.1, " · ")
    };
    let compact = format!(
        "{left_label} {}{separator}{right_label} {}",
        left.2, right.2
    );
    if compact.width() > usize::from(width) {
        return line(clipped(&compact, width.into()), SOFT);
    }
    Line::from(vec![
        Span::styled(format!("{left_label} "), Style::default().fg(SOFT)),
        Span::styled(
            left.2.to_owned(),
            Style::default().fg(left.3).add_modifier(Modifier::BOLD),
        ),
        Span::styled(separator, Style::default().fg(SOFT)),
        Span::styled(format!("{right_label} "), Style::default().fg(SOFT)),
        Span::styled(
            right.2.to_owned(),
            Style::default().fg(right.3).add_modifier(Modifier::BOLD),
        ),
    ])
}
fn output_speed(t: &Totals) -> Option<f64> {
    (t.speed_ms > 0).then(|| t.speed_output as f64 * 1000.0 / t.speed_ms as f64)
}
fn speed_color(speed: f64) -> Color {
    if speed < 50.0 {
        Color::Rgb(236, 104, 113)
    } else if speed < 100.0 {
        Color::Rgb(180, 133, 222)
    } else if speed < 200.0 {
        Color::Rgb(112, 171, 235)
    } else {
        Color::Rgb(133, 212, 162)
    }
}
fn speed_spans(value: &str, speed: Option<f64>) -> Vec<Span<'static>> {
    let style = |color| Style::default().fg(color).add_modifier(Modifier::BOLD);
    match speed {
        None => vec![Span::styled(value.to_owned(), style(SOFT))],
        Some(speed) if speed >= 300.0 => {
            let rainbow = [
                (236, 104, 113),
                (244, 164, 101),
                (239, 214, 111),
                (133, 212, 162),
                (112, 201, 228),
                (112, 171, 235),
                (180, 133, 222),
            ];
            let chars: Vec<char> = value.chars().collect();
            let last = chars.len().saturating_sub(1).max(1);
            chars
                .into_iter()
                .enumerate()
                .map(|(index, ch)| {
                    let color = rainbow[index * (rainbow.len() - 1) / last];
                    Span::styled(ch.to_string(), style(Color::Rgb(color.0, color.1, color.2)))
                })
                .collect()
        }
        Some(speed) => vec![Span::styled(value.to_owned(), style(speed_color(speed)))],
    }
}
fn speed_pair(label: &str, totals: &Totals, width: u16) -> Line<'static> {
    let speed = output_speed(totals);
    let value = speed.map_or("—".into(), |n| format!("{n:.1} tok/s"));
    let available = usize::from(width).saturating_sub(value.width() + 1);
    let label = clipped(label, available);
    let gap = usize::from(width).saturating_sub(label.width() + value.width());
    let mut spans = vec![
        Span::styled(label, Style::default().fg(SOFT)),
        Span::raw(" ".repeat(gap)),
    ];
    spans.extend(speed_spans(&value, speed));
    Line::from(spans)
}
fn mini_speed_line(totals: &Totals, width: u16) -> Line<'static> {
    let speed = output_speed(totals);
    let value = speed.map_or("—".into(), |n| format!("{n:.1} tok/s"));
    let value = clipped(&value, usize::from(width.saturating_sub(2)));
    let mut spans = vec![Span::styled("↓ ", Style::default().fg(SOFT))];
    spans.extend(speed_spans(&value, speed));
    Line::from(spans)
}
fn section(title: &str, width: u16) -> Line<'static> {
    line(
        format!(
            "{title} {}",
            "─".repeat(usize::from(width).saturating_sub(title.width() + 1))
        ),
        SOFT,
    )
}
fn section_action(title: &str, action: &str, width: u16) -> Line<'static> {
    let available = usize::from(width).saturating_sub(action.width() + 1);
    let title = clipped(title, available);
    let gap = usize::from(width).saturating_sub(title.width() + action.width());
    let separator = if gap >= 2 {
        format!(" {} ", "─".repeat(gap - 2))
    } else {
        " ".into()
    };
    Line::from(vec![
        Span::styled(format!("{title}{separator}"), Style::default().fg(SOFT)),
        Span::styled(
            action.to_string(),
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ),
    ])
}
fn meter(label: &str, fraction: Option<f64>, width: u16, color: Color) -> Line<'static> {
    let prefix = format!("{label} ");
    let cells = usize::from(width).saturating_sub(prefix.width());
    let filled = fraction
        .map(|n| (n.clamp(0.0, 1.0) * cells as f64).round() as usize)
        .unwrap_or(0);
    Line::from(vec![
        Span::styled(prefix, Style::default().fg(SOFT)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled(
            "░".repeat(cells.saturating_sub(filled)),
            Style::default().fg(RAIL),
        ),
    ])
}
fn health_meter(t: &Totals, width: u16) -> Line<'static> {
    let cells = usize::from(width.saturating_sub(4));
    let counts = [
        t.success.max(0) as u64,
        t.failed.max(0) as u64,
        t.interrupted.max(0) as u64,
    ];
    let total: u64 = counts.iter().sum();
    if total == 0 || cells == 0 {
        return meter("OK ", None, width, GREEN);
    }
    let active = counts.iter().filter(|&&count| count > 0).count();
    let mut lengths = [0usize; 3];
    if cells >= active {
        for (length, count) in lengths.iter_mut().zip(counts) {
            *length = usize::from(count > 0);
        }
    }
    let remaining = cells.saturating_sub(lengths.iter().sum::<usize>());
    let mut remainders = [0u64; 3];
    for i in 0..3 {
        let weighted = (remaining as u128) * u128::from(counts[i]);
        lengths[i] += (weighted / u128::from(total)) as usize;
        remainders[i] = (weighted % u128::from(total)) as u64;
    }
    let mut leftover = cells.saturating_sub(lengths.iter().sum::<usize>());
    while leftover > 0 {
        let i = (0..3).max_by_key(|&i| remainders[i]).unwrap();
        lengths[i] += 1;
        remainders[i] = 0;
        leftover -= 1;
    }
    Line::from(vec![
        Span::styled("OK  ", Style::default().fg(SOFT)),
        Span::styled("█".repeat(lengths[0]), Style::default().fg(GREEN)),
        Span::styled("█".repeat(lengths[1]), Style::default().fg(RED)),
        Span::styled("█".repeat(lengths[2]), Style::default().fg(GOLD)),
    ])
}
fn compact_token_meter(input: i64, output: i64, width: u16) -> Line<'static> {
    let info = format!("I {}  O {}", short(input), short(output));
    let cells = usize::from(width).saturating_sub(info.width() + 5).min(10);
    let total = input.saturating_add(output);
    let mut input_cells = if total > 0 {
        ((input as f64 / total as f64) * cells as f64).round() as usize
    } else {
        0
    };
    if input > 0 && output > 0 && cells >= 2 {
        input_cells = input_cells.clamp(1, cells - 1);
    }
    let output_cells = if total > 0 {
        cells.saturating_sub(input_cells)
    } else {
        0
    };
    Line::from(vec![
        Span::styled("I/O ", Style::default().fg(SOFT)),
        Span::styled("█".repeat(input_cells), Style::default().fg(BLUE)),
        Span::styled("█".repeat(output_cells), Style::default().fg(GOLD)),
        Span::styled(
            "░".repeat(cells.saturating_sub(input_cells + output_cells)),
            Style::default().fg(RAIL),
        ),
        Span::raw(" "),
        Span::styled(info, Style::default().fg(INK)),
    ])
}
fn compact_cache_meter(
    read: i64,
    write: i64,
    reuse: Option<f64>,
    known: bool,
    width: u16,
) -> Line<'static> {
    let info = format!(
        "{} R {} W {}",
        reuse.map_or("—".into(), |n| format!("{n:.1}%")),
        if known { short(read) } else { "—".into() },
        if known { short(write) } else { "—".into() }
    );
    let cells = usize::from(width).saturating_sub(info.width() + 3).min(10);
    let filled = reuse
        .map(|n| (n.clamp(0.0, 100.0) / 100.0 * cells as f64).round() as usize)
        .unwrap_or(0);
    Line::from(vec![
        Span::styled("↺ ", Style::default().fg(SOFT)),
        Span::styled("█".repeat(filled), Style::default().fg(GREEN)),
        Span::styled(
            "░".repeat(cells.saturating_sub(filled)),
            Style::default().fg(RAIL),
        ),
        Span::raw(" "),
        Span::styled(info, Style::default().fg(INK)),
    ])
}
fn gateway_cache_meter(t: &Totals, unknown: bool, width: u16) -> Vec<Line<'static>> {
    let hit = (t.cache_input > 0).then(|| 100.0 * t.cache_hits as f64 / t.cache_input as f64);
    let percent = hit.map_or("—".into(), |n| format!("{n:.1}%"));
    let read = if unknown {
        "—".into()
    } else {
        short(t.cache_read)
    };
    let write = if unknown {
        "—".into()
    } else {
        short(t.cache_write)
    };
    let info = format!("{percent} R {read} W {write}");
    let available = usize::from(width).saturating_sub("HIT  ".width() + info.width());
    if available < 4 {
        return vec![
            meter("HIT", hit.map(|n| n / 100.0), width, GREEN),
            pair(
                "R / W",
                format!("{read} / {write} · {percent}"),
                width,
                GREEN,
            ),
        ];
    }
    let cells = available.min(10);
    let filled = hit
        .map(|n| (n.clamp(0.0, 100.0) / 100.0 * cells as f64).round() as usize)
        .unwrap_or(0);
    vec![Line::from(vec![
        Span::styled("HIT ", Style::default().fg(SOFT)),
        Span::styled("█".repeat(filled), Style::default().fg(GREEN)),
        Span::styled(
            "░".repeat(cells.saturating_sub(filled)),
            Style::default().fg(RAIL),
        ),
        Span::raw(" "),
        Span::styled(info, Style::default().fg(INK)),
    ])]
}
fn buttons(area: Rect) -> Vec<Rect> {
    let constraints = if area.width >= 44 {
        vec![
            Constraint::Length(10),
            Constraint::Min(6),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(6),
        ]
    } else {
        vec![Constraint::Ratio(1, 5); 5]
    };
    Layout::horizontal(constraints)
        .split(Rect::new(
            area.x,
            area.bottom().saturating_sub(1),
            area.width,
            1,
        ))
        .to_vec()
}
fn content_body(inner: Rect) -> Rect {
    Rect::new(
        inner.x,
        inner.y + 4,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(7),
    )
}
fn mini_body(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(2),
        area.width,
        area.height.saturating_sub(4),
    )
}
fn mini_buttons(area: Rect) -> Vec<Rect> {
    Layout::horizontal([Constraint::Ratio(1, 5); 5])
        .split(Rect::new(
            area.x,
            area.bottom().saturating_sub(1),
            area.width,
            1,
        ))
        .to_vec()
}
fn mini_line(text: impl AsRef<str>, width: u16, color: Color) -> Line<'static> {
    line(clipped(text.as_ref(), usize::from(width)), color)
}
fn mini_sparkline(hours: &[i64; 24]) -> String {
    let buckets: Vec<i64> = hours.chunks(3).map(|hours| hours.iter().sum()).collect();
    let peak = buckets.iter().copied().max().unwrap_or(0);
    let bars = ['·', '▁', '▂', '▃', '▄', '▅', '▆', '█'];
    buckets
        .into_iter()
        .map(|count| {
            if peak == 0 || count == 0 {
                bars[0]
            } else {
                bars[((count as f64 / peak as f64 * 7.0).round() as usize).clamp(1, 7)]
            }
        })
        .collect()
}
fn mini_hourly_rows(hours: &[i64; 24], width: u16, color: Color) -> Vec<Line<'static>> {
    let peak = hours.iter().copied().max().unwrap_or(0);
    let bar_width = usize::from(width.saturating_sub(12)).max(1);
    hours
        .iter()
        .enumerate()
        .map(|(hour, count)| {
            let filled = if peak > 0 && *count > 0 {
                ((*count as f64 / peak as f64 * bar_width as f64).round() as usize)
                    .clamp(1, bar_width)
            } else {
                0
            };
            mini_line(
                format!(
                    "{hour:02} {:<bar_width$} {}",
                    if filled == 0 {
                        "·".into()
                    } else {
                        "█".repeat(filled)
                    },
                    short(*count),
                ),
                width,
                color,
            )
        })
        .collect()
}
fn scrollbar_target(body: Rect, rail_x: u16, column: u16, row: u16, limit: u16) -> Option<u16> {
    if limit == 0 || body.height < 2 || column != rail_x || row < body.y || row >= body.bottom() {
        return None;
    }
    let relative = u32::from(row - body.y);
    let maximum = u32::from(body.height - 1);
    Some(((relative * u32::from(limit) + maximum / 2) / maximum) as u16)
}

fn hourly_chart(hours: &[i64; 24], width: u16, color: Color) -> Vec<Line<'static>> {
    let plot = usize::from(width.saturating_sub(1));
    if plot < 24 {
        return vec![];
    }
    let max = hours.iter().copied().max().unwrap_or(0);
    let mut lines = vec![];
    let blocks = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    for row in (0..6).rev() {
        let mut spans = vec![Span::styled("│", Style::default().fg(RAIL))];
        for (hour, count) in hours.iter().enumerate() {
            let cells = (hour + 1) * plot / 24 - hour * plot / 24;
            let level = if max == 0 || *count == 0 {
                0
            } else {
                ((*count as f64 / max as f64 * 48.0).round() as usize).max(1)
            };
            let fill = level.saturating_sub(row * 8).min(8);
            spans.push(Span::styled(
                blocks[fill].to_string().repeat(cells),
                Style::default().fg(color),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(line(format!("└{}", "─".repeat(plot)), RAIL));
    let mut labels = vec![' '; plot];
    for hour in [0, 6, 12, 18, 23] {
        let x = (hour * plot / 24).min(plot.saturating_sub(2));
        for (i, c) in format!("{hour:02}").chars().enumerate() {
            labels[x + i] = c;
        }
    }
    lines.push(line(
        format!(" {}", labels.into_iter().collect::<String>()),
        SOFT,
    ));
    lines
}
impl Monitor {
    fn apply_focus(&mut self, update: FocusUpdate) {
        let focus_changed = self.focused_pane.as_deref() != Some(update.pane_id.as_str());
        let agent_changed = self.focused_client != Some(update.client);
        let session_changed = self.active_session != update.session;
        self.focused_pane = Some(update.pane_id);
        self.focused_client = Some(update.client);
        if focus_changed || agent_changed {
            self.client = update.client;
        }
        if focus_changed || agent_changed || session_changed {
            self.active_session = update.session;
            if self.sessions_mode {
                self.scroll = 0;
            }
        }
    }
    fn mini_content(&self, width: u16) -> Vec<Line<'static>> {
        if self.help {
            let mut out = vec![
                mini_line("↑ input  ↓ output", width, BLUE),
                mini_line("↺ cache · R read · W write", width, SOFT),
            ];
            out.extend(
                self.content(width)
                    .into_iter()
                    .map(|l| mini_line(l.to_string(), width, SOFT)),
            );
            return out;
        }
        if self.sessions_mode {
            let mut out = vec![mini_line("SESSIONS / ALL TIME", width, BLUE)];
            match (self.active_session.as_ref(), self.active_row()) {
                (None, _) => out.push(mini_line("◌ Waiting for agent pane", width, SOFT)),
                (Some(active), None) => out.push(mini_line(
                    format!("● {} · loading log", active.client),
                    width,
                    GREEN,
                )),
                (_, Some(session)) => {
                    let project = std::path::Path::new(&session.project)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    out.push(mini_line(
                        format!("● {} / {}", session.client, project),
                        width,
                        GREEN,
                    ));
                    out.push(mini_line(
                        format!(
                            "{} tok{}",
                            if session.tokens.known {
                                short(session.tokens.total())
                            } else {
                                "?".into()
                            },
                            if session.incomplete { "+?" } else { "" }
                        ),
                        width,
                        INK,
                    ));
                    if session.tokens.known {
                        out.push(mini_line(
                            format!(
                                "↑ {}  ↓ {}",
                                short(session.tokens.input),
                                short(session.tokens.output)
                            ),
                            width,
                            SOFT,
                        ));
                        out.push(mini_line(
                            format!(
                                "↺ R {}  W {}",
                                short(session.tokens.read),
                                short(session.tokens.write)
                            ),
                            width,
                            SOFT,
                        ));
                    }
                }
            }
            out.push(mini_line("HISTORY · t sort", width, BLUE));
            let rows = self.session_rows();
            if rows.is_empty() {
                out.push(mini_line("No local sessions yet", width, SOFT));
            }
            for session in rows.into_iter().filter(|s| {
                !self
                    .active_session
                    .as_ref()
                    .is_some_and(|a| a.client == s.client && a.id == s.id)
            }) {
                let project = std::path::Path::new(&session.project)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                out.push(mini_line(
                    format!(
                        "{} {} · {} tok",
                        session.client,
                        project,
                        if session.tokens.known {
                            short(session.tokens.total())
                        } else {
                            "?".into()
                        }
                    ),
                    width,
                    INK,
                ));
                if session.tokens.known {
                    out.push(mini_line(
                        format!(
                            "  ↑ {} · ↓ {}",
                            short(session.tokens.input),
                            short(session.tokens.output)
                        ),
                        width,
                        SOFT,
                    ));
                }
            }
            return out;
        }
        if self.chart_mode {
            let gateway = metrics(&self.snapshot, self.client());
            let session = self.session_hours_today();
            let mut out = vec![
                mini_line("CHARTS / TODAY", width, BLUE),
                mini_line(format!("Calls  {}", gateway.total.calls), width, INK),
                mini_line(mini_sparkline(&gateway.hours), width, BLUE),
                mini_line("00   06   12   18   24", width, SOFT),
                mini_line(
                    format!("Session  {} tok", short(session.iter().sum())),
                    width,
                    GREEN,
                ),
                mini_line(mini_sparkline(&session), width, GREEN),
            ];
            out.push(mini_line("GATEWAY / HOURLY CALLS", width, BLUE));
            out.extend(mini_hourly_rows(&gateway.hours, width, BLUE));
            out.push(mini_line("SESSION / HOURLY TOKENS", width, GREEN));
            out.extend(mini_hourly_rows(&session, width, GREEN));
            return out;
        }
        let m = metrics(&self.snapshot, self.client());
        let t = &m.total;
        let unknown = t.calls > 0 && t.unknown == t.calls;
        let total = if self.refreshed.is_none() || unknown {
            "—".into()
        } else {
            short(t.input + t.output)
        };
        let mut out = vec![mini_line(
            format!("TODAY  {}", self.snapshot.today()),
            width,
            BLUE,
        )];
        out.extend(mini_token_total(&total, width, BLUE));
        out.extend([
            mini_line(format!("{} calls", t.calls), width, SOFT),
            mini_line(
                format!(
                    "↑ {}  ↓ {}",
                    if unknown { "?".into() } else { short(t.input) },
                    if unknown { "?".into() } else { short(t.output) }
                ),
                width,
                SOFT,
            ),
            mini_line(
                format!(
                    "OK {}  ×{}  !{}",
                    rate(t).map_or("—".into(), |r| format!("{r:.0}%")),
                    t.failed,
                    t.interrupted
                ),
                width,
                GREEN,
            ),
        ]);
        if let Some(active) = self.active_row() {
            out.push(mini_line("CURRENT SESSION", width, GREEN));
            out.push(mini_line(
                format!("● {} session", active.client),
                width,
                GREEN,
            ));
            let total = if active.tokens.known {
                short(active.tokens.total())
            } else {
                "?".into()
            };
            out.extend(mini_token_total(&total, width, GREEN));
            if active.tokens.known {
                out.push(mini_line(
                    format!(
                        "↑ {}  ↓ {}",
                        short(active.tokens.input),
                        short(active.tokens.output)
                    ),
                    width,
                    SOFT,
                ));
                out.push(mini_line(
                    format!(
                        "↺ R {}  W {}",
                        short(active.tokens.read),
                        short(active.tokens.write)
                    ),
                    width,
                    SOFT,
                ));
            }
        }
        let entries: Vec<(String, &Totals)> = if self.models {
            m.models.iter().map(|(name, t)| (name.clone(), t)).collect()
        } else {
            m.providers
                .values()
                .map(|(name, t)| (name.clone(), t))
                .collect()
        };
        out.push(mini_line(
            if self.models {
                "MODELS · m switch"
            } else {
                "PROVIDERS · m switch"
            },
            width,
            BLUE,
        ));
        if entries.is_empty() {
            out.push(mini_line("No tracked requests", width, SOFT));
        }
        for (name, totals) in entries {
            out.push(mini_line(
                format!("{}  {} calls", name, totals.calls),
                width,
                INK,
            ));
            out.push(mini_line(
                format!(
                    "  {} tok  ×{}",
                    token_label(totals),
                    totals.failed + totals.interrupted
                ),
                width,
                SOFT,
            ));
        }
        out.push(mini_line("GATEWAY / DETAIL", width, BLUE));
        out.push(mini_line(
            format!(
                "↺ R {}  W {}",
                if unknown {
                    "?".into()
                } else {
                    short(t.cache_read)
                },
                if unknown {
                    "?".into()
                } else {
                    short(t.cache_write)
                }
            ),
            width,
            SOFT,
        ));
        out.push(mini_line(
            format!(
                "↺ hit {}",
                if t.cache_input > 0 {
                    format!("{:.1}%", 100.0 * t.cache_hits as f64 / t.cache_input as f64)
                } else {
                    "—".into()
                }
            ),
            width,
            SOFT,
        ));
        out.push(mini_speed_line(t, width));
        out.push(mini_line(
            format!(
                "✓{}  ×{}  !{}  ◌{}",
                t.success, t.failed, t.interrupted, t.pending
            ),
            width,
            GREEN,
        ));
        out.push(mini_line(format!("Compactions {}", m.compact), width, SOFT));
        if t.unknown > 0 {
            out.push(mini_line(
                format!("Unknown tokens: {} calls", t.unknown),
                width,
                GOLD,
            ));
        }
        out.push(mini_line("CALLS / EACH HOUR", width, BLUE));
        out.extend(mini_hourly_rows(&m.hours, width, BLUE));
        out
    }
    fn draw_mini(&mut self, f: &mut ratatui::Frame, area: Rect) {
        if area.height < 4 {
            f.render_widget(
                Paragraph::new(clipped("◈ Pulse · q close", area.width.into()))
                    .style(Style::default().fg(BLUE)),
                area,
            );
            self.limit = 0;
            self.scroll = 0;
            return;
        }
        let title = if self.help {
            "HELP"
        } else if self.sessions_mode {
            "SESSIONS"
        } else if self.chart_mode {
            "CHARTS"
        } else {
            "PULSE"
        };
        f.render_widget(
            Paragraph::new(format!("◈ {title}"))
                .style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Rect::new(area.x, area.y, area.width.saturating_sub(5), 1),
        );
        if area.width >= 10 {
            f.render_widget(
                Paragraph::new("v ?").style(Style::default().fg(SOFT)),
                Rect::new(area.right().saturating_sub(4), area.y, 4, 1),
            );
        }
        let tabs = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            1,
        ));
        for (i, name) in ["Claude", "Codex", "All"].into_iter().enumerate() {
            let label = if area.width < 21 {
                ["Cl", "Cx", "All"][i]
            } else {
                name
            };
            f.render_widget(
                Paragraph::new(label).alignment(Alignment::Center).style(
                    Style::default()
                        .fg(if self.client == i { BG } else { SOFT })
                        .bg(if self.client == i { BLUE } else { RAIL }),
                ),
                tabs[i],
            );
        }
        let body = mini_body(area);
        let mut content = self.mini_content(body.width);
        if self.visual_mode && !self.help && !self.sessions_mode && !self.chart_mode {
            let totals = metrics(&self.snapshot, self.client()).total;
            content.insert(4, health_meter(&totals, body.width));
        }
        self.limit = (content.len() as u16).saturating_sub(body.height);
        self.scroll = self.scroll.min(self.limit);
        f.render_widget(Paragraph::new(content).scroll((self.scroll, 0)), body);
        if area.height >= 4 {
            let status = if let Some(error) = &self.error {
                format!("! {error}")
            } else if let Some(notice) = &self.notice {
                notice.clone()
            } else if self.sessions_mode && self.sessions_refreshed.is_none() {
                "◌ sessions".into()
            } else if !self.sessions_mode && self.refreshed.is_none() {
                "◌ loading".into()
            } else if self.limit > 0 {
                "↑↓ scroll".into()
            } else {
                "● live".into()
            };
            f.render_widget(
                Paragraph::new(clipped(&status, area.width.into()))
                    .style(Style::default().fg(if self.error.is_some() { RED } else { SOFT })),
                Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1),
            );
        }
        for (label, rect) in [
            "e",
            if self.sessions_mode { "t" } else { "c" },
            "s",
            "r",
            "q",
        ]
        .into_iter()
        .zip(mini_buttons(area))
        {
            f.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(BLUE).bg(RAIL)),
                rect,
            );
        }
    }
    fn client(&self) -> Option<&'static str> {
        [Some("Claude"), Some("Codex"), None][self.client]
    }
    fn provider_header_hit(&self, body: Rect, column: u16, row: u16) -> bool {
        if self.sessions_mode || self.chart_mode || !contains(body, column, row) {
            return false;
        }
        let Some(index) = self.content(body.width).iter().position(|line| {
            let text = line.to_string();
            text.starts_with("PROVIDERS / TODAY") || text.starts_with("MODELS / TODAY")
        }) else {
            return false;
        };
        let index = index as u16;
        index >= self.scroll && row == body.y + index - self.scroll
    }
    fn active_row(&self) -> Option<&crate::sessions::Session> {
        let current = self.active_session.as_ref()?;
        self.sessions
            .rows
            .iter()
            .find(|s| s.client == current.client && s.id == current.id)
    }
    fn session_hours_today(&self) -> [i64; 24] {
        let mut hours = [0; 24];
        let Some(session) = self.active_row() else {
            return hours;
        };
        let zone = chrono::FixedOffset::east_opt(self.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        let today = self.snapshot.today();
        for (&timestamp, &tokens) in &session.activity {
            if let Some(time) = chrono::DateTime::from_timestamp(timestamp, 0) {
                let local = time.with_timezone(&zone);
                if local.format("%Y-%m-%d").to_string() == today {
                    hours[local.hour() as usize] += tokens;
                }
            }
        }
        hours
    }
    fn chart_content(&self, width: u16) -> Vec<Line<'static>> {
        let gateway = metrics(&self.snapshot, self.client());
        let session = self.session_hours_today();
        let mut out = vec![
            section("CHARTS / TODAY", width),
            line("Gateway calls + active session tokens", SOFT),
            Line::default(),
            section("GATEWAY / REQUESTS BY HOUR", width),
            pair("Calls today", gateway.total.calls.to_string(), width, BLUE),
        ];
        out.extend(hourly_chart(&gateway.hours, width, BLUE));
        out.extend([Line::default(), section("SESSION / TOKENS BY HOUR", width)]);
        if self.active_session.is_none() {
            out.push(line("Waiting for agent session ID…", SOFT));
        } else if self.active_row().is_none() {
            out.push(line("Waiting for local session log…", SOFT));
        } else {
            out.push(pair(
                "Tokens today",
                short(session.iter().sum()),
                width,
                GREEN,
            ));
            out.extend(hourly_chart(&session, width, GREEN));
            out.push(line("Input + output from local log", SOFT));
        }
        out
    }
    fn visual_active_content(&self, width: u16) -> Vec<Line<'static>> {
        if self.source_pane.is_none() {
            return vec![];
        }
        let mut out = vec![section("SESSION / ALL TIME", width)];
        if self.roomy_visual {
            out.push(Line::default());
        }
        let Some(current) = &self.active_session else {
            out.push(line("◌ Waiting for agent session ID", SOFT));
            out.push(line("Open or resume a session in the focused pane", SOFT));
            return out;
        };
        let Some(s) = self.active_row() else {
            out.push(pair(
                &format!(
                    "● {} · {}",
                    current.client,
                    current.id.chars().take(12).collect::<String>()
                ),
                "— tok",
                width,
                GREEN,
            ));
            out.push(line("Waiting for local session log…", SOFT));
            return out;
        };
        let project = std::path::Path::new(&s.project)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        out.push(line(
            clipped(
                &format!(
                    "● {} / {}",
                    s.client,
                    if project.is_empty() {
                        "Unknown project"
                    } else {
                        &project
                    }
                ),
                width.into(),
            ),
            GREEN,
        ));
        out.push(pair(
            "Session",
            s.id.chars().take(12).collect::<String>(),
            width,
            SOFT,
        ));
        if self.roomy_visual {
            out.push(Line::default());
        }
        if !s.tokens.known {
            out.push(line("Token usage unavailable in local log", SOFT));
            return out;
        }
        out.extend(token_digits(&short(s.tokens.total()), GREEN));
        if self.roomy_visual {
            out.push(Line::default());
        }
        if s.incomplete {
            out.push(line("+? partial token log", GOLD));
        }
        out.push(compact_token_meter(s.tokens.input, s.tokens.output, width));
        out.push(compact_cache_meter(
            s.tokens.read,
            s.tokens.write,
            s.tokens.cache_reuse_percent(),
            s.tokens.cache_known,
            width,
        ));
        out
    }
    fn visual_home_content(&self, width: u16) -> Vec<Line<'static>> {
        let m = metrics(&self.snapshot, self.client());
        let t = &m.total;
        let mut out = vec![pair("TOKENS", self.snapshot.today(), width, SOFT)];
        if self.roomy_visual {
            out.push(Line::default());
        }
        if self.refreshed.is_none() {
            out.push(line("◌ Reading gateway usage…", SOFT));
            let active = self.visual_active_content(width);
            if !active.is_empty() {
                out.push(Line::default());
                out.extend(active);
            }
            return out;
        }
        let unknown = t.calls > 0 && t.unknown == t.calls;
        let total = if unknown {
            "—".into()
        } else {
            short(t.input + t.output)
        };
        out.extend(token_digits(&total, BLUE));
        if self.roomy_visual {
            out.push(Line::default());
        }
        out.push(if unknown {
            line("I/O ░░░░░░░░░░ I ? O ?", SOFT)
        } else {
            compact_token_meter(t.input, t.output, width)
        });
        if self.roomy_visual {
            out.push(Line::default());
        }
        out.push(pair(
            "● Calls / unknown",
            format!("{} / {}", t.calls, t.unknown),
            width,
            INK,
        ));
        out.extend(gateway_cache_meter(t, unknown, width));
        out.push(speed_pair("↗ Rate", t, width));
        out.push(line(format!("{} measured streams", t.speed_samples), SOFT));
        out.push(Line::default());
        out.push(section("CALL HEALTH", width));
        if self.roomy_visual {
            out.push(Line::default());
        }
        let health = rate(t);
        out.push(health_meter(t, width));
        out.push(pair(
            "Success rate",
            health.map_or("— no samples".into(), |n| format!("{n:.1}%")),
            width,
            GREEN,
        ));
        out.push(line(
            format!(
                "✓ {}   × {}   ! {}   ◌ {}",
                t.success, t.failed, t.interrupted, t.pending
            ),
            INK,
        ));
        out.push(pair("Compaction", m.compact.to_string(), width, SOFT));
        let active = self.visual_active_content(width);
        if !active.is_empty() {
            out.push(Line::default());
            out.extend(active);
        }
        out.push(Line::default());
        out.push(if self.models {
            section_action("MODELS / TODAY", "[Providers m]", width)
        } else {
            section_action("PROVIDERS / TODAY", "[Models m]", width)
        });
        if self.roomy_visual {
            out.push(Line::default());
        }
        let mut entries: Vec<(String, &Totals)> = if self.models {
            m.models
                .iter()
                .map(|(name, totals)| {
                    (
                        if name.is_empty() {
                            "Unknown model".into()
                        } else {
                            name.clone()
                        },
                        totals,
                    )
                })
                .collect()
        } else {
            m.providers
                .values()
                .map(|(name, totals)| (name.clone(), totals))
                .collect()
        };
        entries.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then_with(|| a.0.cmp(&b.0)));
        if entries.is_empty() {
            out.push(line("No tracked requests today", SOFT));
            out.push(line("Waiting for gateway traffic…", SOFT));
        }
        for (index, (name, totals)) in entries.into_iter().enumerate() {
            if self.roomy_visual && index > 0 {
                out.push(Line::default());
            }
            out.push(pair(&name, format!("{} calls", totals.calls), width, INK));
            out.push(pair(
                &format!("{} tok", token_label(totals)),
                format!("× {}", totals.failed + totals.interrupted),
                width,
                if totals.failed + totals.interrupted > 0 {
                    RED
                } else {
                    SOFT
                },
            ));
        }
        out
    }
    fn active_content(&self, width: u16) -> Vec<Line<'static>> {
        if self.source_pane.is_none() {
            return vec![];
        }
        let mut out = vec![section("SESSION / ALL TIME", width)];
        let Some(current) = &self.active_session else {
            out.push(line("◌ Waiting for agent session ID", SOFT));
            out.push(line(
                clipped("Open or resume a session in the focused pane", width.into()),
                SOFT,
            ));
            return out;
        };
        let Some(s) = self.active_row() else {
            out.push(pair(
                &format!(
                    "● {} · {}",
                    current.client,
                    current.id.chars().take(12).collect::<String>()
                ),
                "— tok",
                width,
                GREEN,
            ));
            out.push(line(
                clipped("Waiting for local session log…", width.into()),
                SOFT,
            ));
            return out;
        };
        let project = std::path::Path::new(&s.project)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        out.push(line(
            clipped(
                &format!(
                    "● {} / {}",
                    s.client,
                    if project.is_empty() {
                        "Unknown project"
                    } else {
                        &project
                    }
                ),
                width.into(),
            ),
            GREEN,
        ));
        out.push(pair(
            "Session",
            s.id.chars().take(12).collect::<String>(),
            width,
            SOFT,
        ));
        if !s.tokens.known {
            out.push(line(
                clipped("Token usage unavailable in local log", width.into()),
                SOFT,
            ));
            return out;
        }
        let suffix = if s.incomplete { "+?" } else { "" };
        out.push(Line::default());
        out.extend(token_digits(&short(s.tokens.total()), GREEN));
        if !suffix.is_empty() {
            out.push(line("+? partial token log", GOLD));
        }
        let input = short(s.tokens.input);
        let output = short(s.tokens.output);
        out.push(duo_line(
            width,
            ("↑ Input", "↑", &input, BLUE),
            ("↓ Output", "↓", &output, GOLD),
        ));
        let read = if s.tokens.cache_known {
            short(s.tokens.read)
        } else {
            "—".into()
        };
        let write = if s.tokens.cache_known {
            short(s.tokens.write)
        } else {
            "—".into()
        };
        out.push(duo_line(
            width,
            ("↺ Read", "R", &read, GREEN),
            ("Write", "W", &write, SOFT),
        ));
        out.push(pair(
            "Cache reuse",
            s.tokens
                .cache_reuse_percent()
                .map_or("—".into(), |rate| format!("{rate:.1}%")),
            width,
            GREEN,
        ));
        out
    }
    fn content(&self, width: u16) -> Vec<Line<'static>> {
        if self.sessions_mode {
            return self.session_content(width);
        }
        if self.chart_mode && !self.help {
            return self.chart_content(width);
        }
        if self.help {
            return vec![
                section("ABOUT THIS DATA", width),
                Line::default(),
                line("Current session: local log", INK),
                line("for the focused agent pane.", INK),
                Line::default(),
                line("Today's gateway totals: only", SOFT),
                line("this CCSW config's traffic.", SOFT),
                line("Direct API and subscription", SOFT),
                line("traffic aren't in that total.", SOFT),
                Line::default(),
                line("Success rate excludes pending", SOFT),
                line("requests; failures and", SOFT),
                line("interruptions count against it.", SOFT),
                Line::default(),
                line("Charts: hourly today; each", SOFT),
                line("series scales to its peak.", SOFT),
                line("Visual I/O: blue input,", SOFT),
                line("gold output; bars show share.", SOFT),
                line("HIT: cache hit share;", SOFT),
                line("R/W: cache read/write.", SOFT),
                line("Calls / unknown: requests", SOFT),
                line("with missing token counts.", SOFT),
                Line::default(),
                line("Cache hit = cached reads /", SOFT),
                line("all input in measured", SOFT),
                line("successful generation calls.", SOFT),
                line("Cache writes are not hits.", SOFT),
                line("Write uses upstream-reported", SOFT),
                line("cache creation tokens;", SOFT),
                line("cache misses aren't writes.", SOFT),
                Line::default(),
                line("Output rate: successful", SOFT),
                line("streams, all output tokens /", SOFT),
                line("request start to completion.", SOFT),
                line("Includes first-token wait,", SOFT),
                line("reasoning and network time.", SOFT),
                line("Weighted average today;", SOFT),
                line("not pure model decode speed.", SOFT),
                line("Old/unmeasured calls excluded.", SOFT),
                line("Speed colors (tok/s):", SOFT),
                line("<50 red · 50–99 purple", SOFT),
                line("100–199 blue · 200–299 green", SOFT),
                line("300+ rainbow", SOFT),
                Line::default(),
                line("? / Esc to return", BLUE),
                line("v: text / visual view", BLUE),
            ];
        }
        if self.visual_mode {
            return self.visual_home_content(width);
        }
        let mut out = vec![];
        let m = metrics(&self.snapshot, self.client());
        let t = &m.total;
        let ready = self.refreshed.is_some();
        if !ready {
            out.push(pair("TOKENS", self.snapshot.today(), width, SOFT));
            out.push(line("◌ Reading gateway usage…", SOFT));
            let active = self.active_content(width);
            if !active.is_empty() {
                out.push(Line::default());
                out.extend(active);
            }
            return out;
        }
        let unknown = t.calls > 0 && t.unknown == t.calls;
        let value = if !ready || (t.calls > 0 && t.unknown == t.calls) {
            "—".into()
        } else {
            short(t.input + t.output)
        };
        out.extend([
            pair("TOKENS", self.snapshot.today(), width, SOFT),
            Line::default(),
        ]);
        out.extend(token_digits(&value, BLUE));
        let input = if unknown {
            "—".into()
        } else {
            short(t.input)
        };
        let output = if unknown {
            "—".into()
        } else {
            short(t.output)
        };
        out.push(duo_line(
            width,
            ("↑ Input", "↑", &input, BLUE),
            ("↓ Output", "↓", &output, GOLD),
        ));
        if t.unknown > 0 {
            out.push(line(
                format!(
                    "! {} {} lack token data",
                    t.unknown,
                    if t.unknown == 1 { "call" } else { "calls" }
                ),
                GOLD,
            ));
        }
        out.push(Line::default());
        out.push(pair(
            "REQUESTS",
            if ready {
                t.calls.to_string()
            } else {
                "—".into()
            },
            width,
            INK,
        ));
        let read = if unknown {
            "—".into()
        } else {
            short(t.cache_read)
        };
        let write = if unknown {
            "—".into()
        } else {
            short(t.cache_write)
        };
        out.push(duo_line(
            width,
            ("↺ Read", "R", &read, GREEN),
            ("Write", "W", &write, SOFT),
        ));
        out.push(pair(
            "Cache hit",
            if t.cache_input > 0 {
                format!("{:.1}%", 100.0 * t.cache_hits as f64 / t.cache_input as f64)
            } else {
                "—".into()
            },
            width,
            BLUE,
        ));
        out.push(speed_pair("Output rate (E2E)", t, width));
        out.push(line(
            format!("  ↳ {} measured streams", t.speed_samples),
            SOFT,
        ));
        out.push(Line::default());
        out.push(section("CALL HEALTH", width));
        let health = rate(t);
        let color = if t.failed + t.interrupted > 0 {
            GOLD
        } else {
            GREEN
        };
        out.push(pair(
            "Success rate",
            health.map_or("— no samples".into(), |r| format!("{r:.1}%")),
            width,
            if health.is_some() { color } else { SOFT },
        ));
        let filled = health.map_or(0, |r| (r / 100.0 * f64::from(width)).round() as usize);
        out.push(Line::from(vec![
            Span::styled("━".repeat(filled), Style::default().fg(GREEN)),
            Span::styled(
                "━".repeat(usize::from(width).saturating_sub(filled)),
                Style::default().fg(if t.failed > 0 {
                    RED
                } else if t.interrupted > 0 {
                    GOLD
                } else {
                    RAIL
                }),
            ),
        ]));
        out.push(pair("✓ Success", t.success.to_string(), width, GREEN));
        out.push(pair(
            "× Failed",
            t.failed.to_string(),
            width,
            if t.failed + t.interrupted > 0 {
                RED
            } else {
                SOFT
            },
        ));
        out.push(pair(
            "! Interrupted",
            t.interrupted.to_string(),
            width,
            if t.interrupted > 0 { GOLD } else { SOFT },
        ));
        out.push(pair("◌ Pending", t.pending.to_string(), width, GOLD));
        out.push(pair("↘ Compaction", m.compact.to_string(), width, SOFT));
        let active = self.active_content(width);
        if !active.is_empty() {
            out.push(Line::default());
            out.extend(active);
        }
        out.push(Line::default());
        out.push(if self.models {
            section_action("MODELS / TODAY", "[Providers m]", width)
        } else {
            section_action("PROVIDERS / TODAY", "[Models m]", width)
        });
        let mut entries: Vec<(String, &Totals)> = if self.models {
            m.models
                .iter()
                .map(|(name, t)| {
                    (
                        if name.is_empty() {
                            "Unknown model".into()
                        } else {
                            name.clone()
                        },
                        t,
                    )
                })
                .collect()
        } else {
            m.providers
                .values()
                .map(|(name, t)| (name.clone(), t))
                .collect()
        };
        entries.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then_with(|| a.0.cmp(&b.0)));
        if entries.is_empty() {
            out.push(line("No tracked requests today", SOFT));
            out.push(line("Waiting for gateway traffic…", SOFT));
        }
        for (index, (name, t)) in entries.into_iter().enumerate() {
            if index > 0 {
                out.push(Line::default());
            }
            out.push(line(name, INK));
            out.push(pair(
                &format!("{} calls · {} tok", t.calls, token_label(t)),
                format!("× {}", t.failed + t.interrupted),
                width,
                if t.failed + t.interrupted > 0 {
                    RED
                } else {
                    SOFT
                },
            ));
        }
        out
    }
    fn session_rows(&self) -> Vec<&crate::sessions::Session> {
        let mut rows: Vec<_> = self
            .sessions
            .rows
            .iter()
            .filter(|s| self.client().is_none_or(|c| s.client == c))
            .collect();
        rows.sort_by(|a, b| {
            let active = |s: &crate::sessions::Session| {
                self.active_session
                    .as_ref()
                    .is_some_and(|current| s.client == current.client && s.id == current.id)
            };
            active(b).cmp(&active(a)).then_with(|| {
                if self.sessions_sort_tokens {
                    b.tokens
                        .known
                        .cmp(&a.tokens.known)
                        .then_with(|| b.tokens.total().cmp(&a.tokens.total()))
                        .then_with(|| b.updated.cmp(&a.updated))
                } else {
                    b.updated.cmp(&a.updated)
                }
                .then_with(|| a.id.cmp(&b.id))
            })
        });
        rows
    }
    fn visual_session_content(&self, width: u16) -> Vec<Line<'static>> {
        let rows = self.session_rows();
        let history_count = rows
            .iter()
            .filter(|s| {
                !self
                    .active_session
                    .as_ref()
                    .is_some_and(|current| s.client == current.client && s.id == current.id)
            })
            .count();
        let mut out = self.visual_active_content(width);
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.push(section("SESSION HISTORY", width));
        if self.roomy_visual {
            out.push(Line::default());
        }
        out.push(pair(
            "Earlier sessions",
            history_count.to_string(),
            width,
            BLUE,
        ));
        out.push(line(
            if self.sessions_sort_tokens {
                "Sorted by tokens · t: recent"
            } else {
                "Sorted by recent · t: tokens"
            },
            SOFT,
        ));
        if self.sessions_refreshed.is_none() {
            out.push(line("Reading local session logs…", SOFT));
        } else if rows.is_empty() {
            out.push(line("No local sessions for this client", SOFT));
        } else if history_count == 0 {
            out.push(line("No other sessions for this client", SOFT));
        }
        let zone = chrono::FixedOffset::east_opt(self.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        for s in rows {
            if self
                .active_session
                .as_ref()
                .is_some_and(|current| s.client == current.client && s.id == current.id)
            {
                continue;
            }
            let project = std::path::Path::new(&s.project)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let id: String = s.id.chars().take(8).collect();
            let detail = format!(
                "{project} · {id}{}{}",
                if s.child { " [child]" } else { "" },
                if s.fork { " *" } else { "" }
            );
            out.push(Line::default());
            out.push(pair(
                &detail,
                format!(
                    "{} tok",
                    if s.tokens.known {
                        format!(
                            "{}{}",
                            short(s.tokens.total()),
                            if s.incomplete { "+?" } else { "" }
                        )
                    } else {
                        "?".into()
                    }
                ),
                width,
                BLUE,
            ));
            let time = chrono::DateTime::from_timestamp(s.updated, 0)
                .map(|t| t.with_timezone(&zone).format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "time ?".into());
            out.push(pair(s.client, time, width, SOFT));
            if s.tokens.known {
                out.push(compact_token_meter(s.tokens.input, s.tokens.output, width));
                out.push(compact_cache_meter(
                    s.tokens.read,
                    s.tokens.write,
                    s.tokens.cache_reuse_percent(),
                    s.tokens.cache_known,
                    width,
                ));
            } else {
                out.push(line("In ? · Out ? · Cache — · R ? / W ?", SOFT));
            }
        }
        if self.sessions.warnings > 0 {
            if history_count > 0 {
                out.push(Line::default());
            }
            out.push(line(
                format!("! {} logs unavailable / partial", self.sessions.warnings),
                GOLD,
            ));
        }
        out
    }
    fn session_content(&self, width: u16) -> Vec<Line<'static>> {
        if self.help {
            return vec![
                section("ABOUT SESSIONS", width),
                line("Current: focused agent pane.", INK),
                line("Local Claude / Codex logs.", INK),
                line("Each row: whole session", SOFT),
                line("tokens, not today's usage.", SOFT),
                line("Input includes cached tokens.", SOFT),
                line("Cache reuse = read / input.", SOFT),
                line("Writes are not cache hits.", SOFT),
                line("Forks may include inherited", SOFT),
                line("tokens; children are separate.", SOFT),
                line("Current follows pane focus.", SOFT),
                line("Visual I/O: blue input,", SOFT),
                line("gold output; bars show share.", SOFT),
                line("s: Usage / Sessions", BLUE),
                line("t: recent / tokens sort", BLUE),
                line("? / Esc to return", BLUE),
                line("v: text / visual view", BLUE),
            ];
        }
        if self.visual_mode {
            return self.visual_session_content(width);
        }
        let rows = self.session_rows();
        let history_count = rows
            .iter()
            .filter(|s| {
                !self
                    .active_session
                    .as_ref()
                    .is_some_and(|current| s.client == current.client && s.id == current.id)
            })
            .count();
        let mut out = self.active_content(width);
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.extend([
            section("SESSION HISTORY", width),
            pair("Earlier sessions", history_count.to_string(), width, BLUE),
            line(
                if self.sessions_sort_tokens {
                    "Sorted by tokens · t: recent"
                } else {
                    "Sorted by recent · t: tokens"
                },
                SOFT,
            ),
        ]);
        if self.sessions_refreshed.is_none() {
            out.push(line("Reading local session logs…", SOFT));
        } else if rows.is_empty() {
            out.push(line("No local sessions for this client", SOFT));
        } else if history_count == 0 {
            out.push(line("No other sessions for this client", SOFT));
        }
        let format_tokens = |s: &crate::sessions::Session, n| {
            if !s.tokens.known {
                "?".into()
            } else {
                format!("{}{}", short(n), if s.incomplete { "+?" } else { "" })
            }
        };
        let zone = chrono::FixedOffset::east_opt(self.snapshot.offset)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        for s in rows {
            if self
                .active_session
                .as_ref()
                .is_some_and(|current| s.client == current.client && s.id == current.id)
            {
                continue;
            }
            out.push(Line::default());
            let project = std::path::Path::new(&s.project)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let id: String = s.id.chars().take(8).collect();
            let detail = format!(
                "{project} · {id}{}{}",
                if s.child { " [child]" } else { "" },
                if s.fork { " *" } else { "" }
            );
            out.push(line(clipped(&detail, width.into()), INK));
            out.push(pair(
                s.client,
                format!("{} tok", format_tokens(s, s.tokens.total())),
                width,
                BLUE,
            ));
            out.push(line(
                format!(
                    "In {} · Out {}",
                    format_tokens(s, s.tokens.input),
                    format_tokens(s, s.tokens.output)
                ),
                SOFT,
            ));
            let time = chrono::DateTime::from_timestamp(s.updated, 0)
                .map(|t| t.with_timezone(&zone).format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "time ?".into());
            let cache = format!(
                "↺ {} · {} read",
                s.tokens
                    .cache_reuse_percent()
                    .map_or("—".into(), |rate| format!("{rate:.1}%")),
                if s.tokens.cache_known {
                    format_tokens(s, s.tokens.read)
                } else {
                    "—".into()
                }
            );
            out.push(pair(&cache, time, width, SOFT));
        }
        if self.sessions.warnings > 0 {
            if history_count > 0 {
                out.push(Line::default());
            }
            out.push(line(
                format!("! {} logs unavailable / partial", self.sessions.warnings),
                GOLD,
            ));
        }
        out
    }
    fn draw(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        f.render_widget(
            Block::default().style(Style::default().bg(BG).fg(INK)),
            area,
        );
        if area.width < 32 || area.height < 12 {
            self.draw_mini(f, area);
            return;
        }
        self.roomy_visual = area.height >= 32;
        let inner = area.inner(Margin::new(2, 0));
        let title = if self.help {
            "◈ CCSW / HELP"
        } else if self.sessions_mode {
            "◈ SESSIONS / ALL TIME"
        } else if self.chart_mode {
            "◈ CHARTS / TODAY"
        } else {
            "◈ GATEWAY / TODAY"
        };
        f.render_widget(
            Paragraph::new(clipped(title, usize::from(inner.width.saturating_sub(10))))
                .style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Rect::new(inner.x, inner.y, inner.width.saturating_sub(9), 1),
        );
        f.render_widget(
            Paragraph::new(if self.visual_mode { "T(v)" } else { "V(v)" })
                .style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Rect::new(inner.right() - 9, inner.y, 4, 1),
        );
        f.render_widget(
            Paragraph::new("?(?)").style(Style::default().fg(SOFT)),
            Rect::new(inner.right() - 4, inner.y, 4, 1),
        );
        let tabs = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            1,
        ));
        for (i, name) in ["Claude", "Codex", "All"].into_iter().enumerate() {
            f.render_widget(
                Paragraph::new(name).alignment(Alignment::Center).style(
                    Style::default()
                        .fg(if self.client == i { BG } else { SOFT })
                        .bg(if self.client == i { BLUE } else { BG }),
                ),
                tabs[i],
            );
        }
        let body = content_body(inner);
        let content = self.content(body.width);
        self.limit = (content.len() as u16).saturating_sub(body.height);
        self.scroll = self.scroll.min(self.limit);
        f.render_widget(Paragraph::new(content).scroll((self.scroll, 0)), body);
        if self.limit > 0 {
            let mut state = ScrollbarState::new(usize::from(self.limit + body.height))
                .position(usize::from(self.scroll))
                .viewport_content_length(usize::from(body.height));
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .thumb_style(Style::default().fg(BLUE))
                    .track_style(Style::default().fg(RAIL)),
                Rect::new(inner.right() - 1, body.y, 1, body.height),
                &mut state,
            );
        }
        let status = if self.sessions_mode {
            if self.sessions.warnings > 0 {
                format!("! {} logs unavailable · r retry", self.sessions.warnings)
            } else if let Some(time) = self.sessions_refreshed {
                format!(
                    "● Sessions updated {}s ago · on change",
                    time.elapsed().as_secs()
                )
            } else {
                "◌ Reading local sessions…".into()
            }
        } else if self.chart_mode {
            if self.sessions.warnings > 0 {
                format!(
                    "! {} session logs unavailable · r retry",
                    self.sessions.warnings
                )
            } else {
                "● Charts auto-update · c: Home".into()
            }
        } else if let Some(error) = &self.error {
            format!("! STALE · {error}")
        } else if let Some(note) = &self.notice {
            note.clone()
        } else if let Some(time) = self.refreshed {
            format!("● Checked {}s ago · every 2s", time.elapsed().as_secs())
        } else {
            "◌ Reading local usage…".into()
        };
        f.render_widget(
            Paragraph::new(status).style(Style::default().fg(
                if ((self.sessions_mode || self.chart_mode) && self.sessions.warnings > 0)
                    || (!self.sessions_mode && self.error.is_some())
                {
                    RED
                } else {
                    SOFT
                },
            )),
            Rect::new(inner.x, area.bottom() - 3, inner.width, 1),
        );
        for (label, rect) in [
            if inner.width >= 44 {
                "↗ Edit(e)"
            } else {
                "↗(e)"
            },
            if inner.width < 44 {
                if self.sessions_mode { "T(t)" } else { "C(c)" }
            } else if self.sessions_mode {
                if self.sessions_sort_tokens {
                    "Recent(t)"
                } else {
                    "Tokens(t)"
                }
            } else if self.chart_mode {
                "Home(c)"
            } else {
                "Chart(c)"
            },
            if inner.width < 36 {
                if self.sessions_mode { "H(s)" } else { "S(s)" }
            } else if inner.width < 44 {
                if self.sessions_mode {
                    "Home(s)"
                } else {
                    "Sess(s)"
                }
            } else if self.sessions_mode {
                "Home(s)"
            } else {
                "Sessions(s)"
            },
            "↻(r)",
            "×(q)",
        ]
        .into_iter()
        .zip(buttons(inner))
        {
            f.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(BLUE).bg(RAIL)),
                rect,
            );
        }
    }
}

fn spawn_session_reader(
    session_send: mpsc::SyncSender<crate::sessions::Snapshot>,
    session_requests: mpsc::Receiver<()>,
    session_refresh: mpsc::SyncSender<()>,
) {
    std::thread::spawn(move || {
        let mut reader = crate::sessions::Reader::default();
        let mut watcher = None;
        let mut watched_at = Instant::now();
        loop {
            let roots = crate::sessions::roots();
            if watcher.is_none() || watched_at.elapsed() >= Duration::from_secs(30) {
                watcher = roots.as_ref().ok().and_then(|roots| {
                    crate::sessions::watch_changes(roots, session_refresh.clone())
                });
                watched_at = Instant::now();
            }
            let snapshot = match roots {
                Ok(roots) => reader.read(&roots),
                Err(_) => crate::sessions::Snapshot {
                    rows: vec![],
                    warnings: 1,
                },
            };
            if session_send.send(snapshot).is_err() {
                break;
            }
            let wait = if watcher.is_some() {
                Duration::from_secs(30).saturating_sub(watched_at.elapsed())
            } else {
                Duration::from_secs(2)
            };
            match session_requests.recv_timeout(wait) {
                Ok(()) => {
                    std::thread::sleep(Duration::from_millis(150));
                    while session_requests.try_recv().is_ok() {}
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    });
}

fn current_focus(workspace: Option<&str>, tab: Option<&str>, source: &str) -> Option<FocusedAgent> {
    match (workspace, tab) {
        (Some(workspace), Some(tab)) => herdr(&["pane", "list", "--workspace", workspace])
            .ok()
            .and_then(|panes| focused_agent(&panes, tab)),
        _ => herdr(&["pane", "get", source])
            .ok()
            .and_then(|result| focused_pane(&result["result"]["pane"], None, source)),
    }
}

fn focus_from_event(
    event: &serde_json::Value,
    workspace: Option<&str>,
    tab: Option<&str>,
    source: &str,
) -> Option<FocusedAgent> {
    let data = &event["data"];
    match event["event"].as_str()? {
        "pane_updated" | "pane_created" | "pane_moved" => focused_pane(&data["pane"], tab, source),
        "pane_focused" | "pane_agent_detected" => {
            let id = data["pane_id"].as_str()?;
            herdr(&["pane", "get", id])
                .ok()
                .and_then(|result| focused_pane(&result["result"]["pane"], tab, source))
        }
        "pane_closed" => current_focus(workspace, tab, source),
        _ => None,
    }
}

#[cfg(unix)]
fn focus_event_socket(path: &std::path::Path) -> Result<Box<dyn ReadWrite>> {
    Ok(Box::new(std::os::unix::net::UnixStream::connect(path)?))
}

#[cfg(windows)]
fn focus_event_socket(path: &std::path::Path) -> Result<Box<dyn ReadWrite>> {
    Ok(Box::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?,
    ))
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn follow_focus_events(
    tracker: &mut FocusTracker,
    send: &mpsc::SyncSender<FocusUpdate>,
    workspace: Option<&str>,
    tab: Option<&str>,
    source: &str,
) -> Result<bool> {
    let path = std::env::var_os("HERDR_SOCKET_PATH").context("Herdr socket unavailable")?;
    let mut socket = focus_event_socket(std::path::Path::new(&path))?;
    let request = serde_json::json!({
        "id": "ccsw_focus",
        "method": "events.subscribe",
        "params": {"subscriptions": [
            {"type": "pane.focused"},
            {"type": "pane.updated"},
            {"type": "pane.created"},
            {"type": "pane.moved"},
            {"type": "pane.closed"},
            {"type": "pane.agent_detected"}
        ]}
    });
    socket.write_all(request.to_string().as_bytes())?;
    socket.write_all(b"\n")?;
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    anyhow::ensure!(
        reader.read_line(&mut line)? > 0,
        "Herdr subscription closed"
    );
    let response: serde_json::Value = serde_json::from_str(&line)?;
    anyhow::ensure!(
        response["result"].is_object(),
        "Herdr subscription rejected"
    );
    if !tracker.observe(current_focus(workspace, tab, source), send, true) {
        return Ok(false);
    }
    loop {
        line.clear();
        anyhow::ensure!(
            reader.read_line(&mut line)? > 0,
            "Herdr subscription closed"
        );
        let event: serde_json::Value = serde_json::from_str(&line)?;
        if !tracker.observe(focus_from_event(&event, workspace, tab, source), send, true) {
            return Ok(false);
        }
    }
}

pub(super) fn run(paths: AppPaths) -> Result<()> {
    let (send, updates) = mpsc::sync_channel(1);
    let (refresh, requests) = mpsc::sync_channel(1);
    let (session_send, session_updates) = mpsc::sync_channel(1);
    let (session_refresh, session_requests) = mpsc::sync_channel(1);
    let source_pane = std::env::var("CCSW_MONITOR_SOURCE_PANE")
        .ok()
        .filter(|id| !id.is_empty());
    let workspace = std::env::var("CCSW_MONITOR_WORKSPACE").ok();
    let tab = std::env::var("CCSW_MONITOR_TAB").ok();
    let theme_paths = paths.clone();
    let (active_send, active_updates) = mpsc::sync_channel(1);
    if let Some(source) = source_pane.clone() {
        std::thread::spawn(move || {
            let mut tracker = FocusTracker::default();
            loop {
                match follow_focus_events(
                    &mut tracker,
                    &active_send,
                    workspace.as_deref(),
                    tab.as_deref(),
                    &source,
                ) {
                    Ok(false) => break,
                    Ok(true) | Err(_) => {}
                }
                if !tracker.observe(
                    current_focus(workspace.as_deref(), tab.as_deref(), &source),
                    &active_send,
                    false,
                ) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
    let mut session_worker = Some((session_send, session_requests));
    let mut session_reader_started = false;
    std::thread::spawn(move || {
        let mut reader = Reader::new(paths.state_dir.join(crate::usage::FILE), paths.config);
        let mut snapshot = Snapshot::default();
        loop {
            let today = snapshot.today();
            let result = reader.read(&Query::Range {
                start: today.clone(),
                end: today,
            });
            if let Ok(Some(s)) = &result {
                snapshot = s.clone();
            }
            if send
                .send(result.map_err(|_| "Usage unavailable; check database access".to_string()))
                .is_err()
            {
                break;
            }
            if matches!(
                requests.recv_timeout(Duration::from_secs(2)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ) {
                break;
            }
        }
    });
    let (mut terminal, _guard) = setup_terminal()?;
    let mut monitor = Monitor {
        pulse_theme: theme::PulseTheme::load(&theme_paths),
        client: initial_client(std::env::var("CCSW_MONITOR_CLIENT").ok().as_deref()),
        source_pane,
        ..Default::default()
    };
    if monitor.source_pane.is_some() {
        session_reader_started = true;
        let (session_send, session_requests) = session_worker.take().unwrap();
        spawn_session_reader(session_send, session_requests, session_refresh.clone());
    }
    let mut redraw = true;
    let mut last_theme_check = Instant::now();
    let mut last_clock_redraw = Instant::now();
    loop {
        if last_theme_check.elapsed() >= Duration::from_secs(2) {
            let theme = theme::PulseTheme::load(&theme_paths);
            if monitor.pulse_theme != theme {
                monitor.pulse_theme = theme;
                redraw = true;
            }
            last_theme_check = Instant::now();
        }
        while let Ok(update) = active_updates.try_recv() {
            monitor.apply_focus(update);
            redraw = true;
        }
        while let Ok(snapshot) = session_updates.try_recv() {
            monitor.sessions = snapshot;
            monitor.sessions_refreshed = Some(Instant::now());
            redraw = true;
        }
        while let Ok(result) = updates.try_recv() {
            match result {
                Ok(snapshot) => {
                    if let Some(s) = snapshot {
                        monitor.snapshot = s;
                    }
                    monitor.refreshed = Some(Instant::now());
                    monitor.error = None;
                }
                Err(error) => monitor.error = Some(error),
            }
            redraw = true;
        }
        if last_clock_redraw.elapsed() >= Duration::from_secs(1) {
            redraw = true;
            last_clock_redraw = Instant::now();
        }
        if redraw {
            terminal.draw(|f| {
                monitor.draw(f);
                monitor.pulse_theme.apply(f.buffer_mut());
            })?;
            redraw = false;
        }
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        redraw = true;
        let key = match event::read()? {
            Event::Key(k) if k.kind == event::KeyEventKind::Press => Some(k),
            Event::Mouse(m) => {
                let size = terminal.size()?;
                let screen = Rect::new(0, 0, size.width, size.height);
                let mini = screen.width < 32 || screen.height < 12;
                let area = if mini {
                    screen
                } else {
                    screen.inner(Margin::new(2, 0))
                };
                let body = content_body(area);
                let dragging = matches!(
                    m.kind,
                    MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left)
                );
                let code = if dragging {
                    if !mini
                        && let Some(target) = scrollbar_target(
                            body,
                            area.right().saturating_sub(1),
                            m.column,
                            m.row,
                            monitor.limit,
                        )
                    {
                        monitor.scroll = target;
                        None
                    } else if !mini
                        && m.kind == MouseEventKind::Down(MouseButton::Left)
                        && monitor.provider_header_hit(body, m.column, m.row)
                    {
                        monitor.models = !monitor.models;
                        None
                    } else {
                        match m.kind {
                            MouseEventKind::Down(MouseButton::Left)
                                if m.row == 0
                                    && (!mini || area.width >= 10)
                                    && m.column
                                        >= area.right().saturating_sub(if mini {
                                            2
                                        } else {
                                            4
                                        }) =>
                            {
                                Some(KeyCode::Char('?'))
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if m.row == 0
                                    && (!mini || area.width >= 10)
                                    && m.column
                                        >= area.right().saturating_sub(if mini {
                                            4
                                        } else {
                                            9
                                        }) =>
                            {
                                Some(KeyCode::Char('v'))
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if m.row == if mini { 1 } else { 2 } =>
                            {
                                let tabs = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(
                                    Rect::new(area.x, if mini { 1 } else { 2 }, area.width, 1),
                                );
                                if let Some(i) =
                                    tabs.iter().position(|r| contains(*r, m.column, m.row))
                                {
                                    monitor.client = i;
                                    monitor.scroll = 0;
                                }
                                None
                            }
                            MouseEventKind::Down(MouseButton::Left) => (if mini {
                                mini_buttons(area)
                            } else {
                                buttons(area)
                            })
                            .iter()
                            .position(|r| contains(*r, m.column, m.row))
                            .map(|i| {
                                [
                                    KeyCode::Char('e'),
                                    KeyCode::Char(if monitor.sessions_mode { 't' } else { 'c' }),
                                    KeyCode::Char('s'),
                                    KeyCode::Char('r'),
                                    KeyCode::Char('q'),
                                ][i]
                            }),
                            _ => None,
                        }
                    }
                } else {
                    match m.kind {
                        MouseEventKind::ScrollDown => Some(KeyCode::Down),
                        MouseEventKind::ScrollUp => Some(KeyCode::Up),
                        _ => None,
                    }
                };
                code.map(|c| KeyEvent::new(c, KeyModifiers::NONE))
            }
            _ => None,
        };
        if let Some(key) = key {
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Char('e') => {
                    monitor.notice = Some(match open_editor() {
                        Ok(()) => "↗ Editor opened in a new tab".into(),
                        Err(e) => format!("! {e}"),
                    });
                }
                KeyCode::Char('m') if !monitor.sessions_mode && !monitor.chart_mode => {
                    monitor.help = false;
                    monitor.models = !monitor.models;
                    monitor.scroll = monitor
                        .content(40)
                        .iter()
                        .position(|line| {
                            let text = line.to_string();
                            text.starts_with("MODELS / TODAY")
                                || text.starts_with("PROVIDERS / TODAY")
                        })
                        .unwrap_or(0) as u16;
                }
                KeyCode::Char('t') if monitor.sessions_mode => {
                    monitor.help = false;
                    monitor.sessions_sort_tokens = !monitor.sessions_sort_tokens;
                    monitor.scroll = 0;
                }
                KeyCode::Char('v') => {
                    monitor.visual_mode = !monitor.visual_mode;
                    monitor.help = false;
                    monitor.scroll = 0;
                }
                KeyCode::Char('c') => {
                    monitor.chart_mode = !monitor.chart_mode;
                    monitor.sessions_mode = false;
                    monitor.help = false;
                    monitor.scroll = 0;
                }
                KeyCode::Char('s') => {
                    monitor.sessions_mode = !monitor.sessions_mode;
                    monitor.chart_mode = false;
                    monitor.help = false;
                    monitor.scroll = 0;
                    if monitor.sessions_mode
                        && let Some(client) = monitor.focused_client
                    {
                        monitor.client = client;
                    }
                    if monitor.sessions_mode && !session_reader_started {
                        session_reader_started = true;
                        let (session_send, session_requests) = session_worker.take().unwrap();
                        spawn_session_reader(
                            session_send,
                            session_requests,
                            session_refresh.clone(),
                        );
                    }
                }
                KeyCode::Char('r') => {
                    let _ = refresh.try_send(());
                    if session_reader_started {
                        let _ = session_refresh.try_send(());
                    }
                    monitor.notice = None;
                }
                KeyCode::Tab => {
                    monitor.client = (monitor.client + 1) % 3;
                    monitor.scroll = 0;
                }
                KeyCode::Char(c @ '1'..='3') => {
                    monitor.client = (c as u8 - b'1') as usize;
                    monitor.scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    monitor.scroll = monitor.scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    monitor.scroll = monitor.scroll.saturating_add(1).min(monitor.limit)
                }
                KeyCode::PageDown => {
                    monitor.scroll = monitor.scroll.saturating_add(10).min(monitor.limit)
                }
                KeyCode::PageUp => monitor.scroll = monitor.scroll.saturating_sub(10),
                KeyCode::Char('?') => {
                    monitor.help = !monitor.help;
                    monitor.scroll = 0;
                }
                KeyCode::Home | KeyCode::Esc => {
                    monitor.help = false;
                    monitor.scroll = 0;
                }
                KeyCode::End => monitor.scroll = monitor.limit,
                _ => {}
            }
        }
    }
    Ok(())
}
fn herdr(args: &[&str]) -> Result<serde_json::Value> {
    let output = Command::new(std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()))
        .args(args)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "Herdr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if output.stdout.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}
pub(super) fn open_pane() -> Result<()> {
    anyhow::ensure!(
        std::env::var("HERDR_ENV").as_deref() == Ok("1"),
        "Open the monitor from inside Herdr"
    );
    let current = herdr(&["pane", "current", "--current"])?;
    let pane = &current["result"]["pane"];
    // Capture the invoking agent before creating the new, agent-free monitor pane.
    let client = ["claude", "codex", "all"][initial_client(pane["agent"].as_str())];
    let client_env = format!("CCSW_MONITOR_CLIENT={client}");
    let workspace = pane["workspace_id"].as_str().context("Missing workspace")?;
    let tab = pane["tab_id"].as_str().context("Missing tab")?;
    let target = pane["pane_id"].as_str().context("Missing calling pane")?;
    let source_env = format!("CCSW_MONITOR_SOURCE_PANE={target}");
    let workspace_env = format!("CCSW_MONITOR_WORKSPACE={workspace}");
    let tab_env = format!("CCSW_MONITOR_TAB={tab}");
    let list = herdr(&["pane", "list", "--workspace", workspace])?;
    if let Some(existing) = list["result"]["panes"].as_array().and_then(|panes| {
        panes
            .iter()
            .find(|p| p["tab_id"] == tab && p["label"] == LABEL)
    }) {
        let id = existing["pane_id"]
            .as_str()
            .context("Missing monitor pane")?;
        let process = herdr(&["pane", "process-info", "--pane", id])?;
        let binary = std::env::current_exe()?;
        let plugin = herdr(&["plugin", "list", "--plugin", "ccsw", "--json"])?;
        let linked_binary = plugin["result"]["plugins"]
            .as_array()
            .and_then(|plugins| plugins.first())
            .and_then(|plugin| plugin["plugin_root"].as_str())
            .and_then(|root| {
                std::fs::canonicalize(std::path::Path::new(root).join("target/release/ccsw")).ok()
            });
        let is_monitor = process["result"]["process_info"]["foreground_processes"]
            .as_array()
            .is_some_and(|ps| {
                ps.iter().any(|p| {
                    p["argv"].as_array().is_some_and(|args| {
                        let executable = args
                            .first()
                            .and_then(|a| a.as_str())
                            .map(std::path::Path::new);
                        let executable = executable.map(|path| {
                            if path.is_absolute() {
                                path.to_path_buf()
                            } else {
                                std::path::Path::new(p["cwd"].as_str().unwrap_or("")).join(path)
                            }
                        });
                        args.len() == 2
                            && args[1] == "quick"
                            && executable
                                .and_then(|path| std::fs::canonicalize(path).ok())
                                .is_some_and(|path| {
                                    path == binary || linked_binary.as_ref() == Some(&path)
                                })
                    })
                })
            });
        anyhow::ensure!(
            is_monitor,
            "The pane named CCSW Pulse is no longer running the monitor; leave it open"
        );
        return herdr(&["pane", "close", id]).map(|_| ());
    }
    let cwd = std::env::current_dir()?;
    let layout = herdr(&["pane", "layout", "--pane", target])?;
    let (split_target, width) = rightmost_split_target(&layout).context(
        "The right edge is too narrow for Pulse; widen a pane at the right edge to 76 columns",
    )?;
    let monitor_width = 48u64.min(width / 2);
    let split = herdr(&[
        "plugin",
        "pane",
        "open",
        "--plugin",
        "ccsw",
        "--entrypoint",
        "quick",
        "--placement",
        "split",
        "--target-pane",
        split_target,
        "--direction",
        "right",
        "--cwd",
        &cwd.to_string_lossy(),
        "--env",
        &client_env,
        "--env",
        &source_env,
        "--env",
        &workspace_env,
        "--env",
        &tab_env,
        "--no-focus",
    ])?;
    let id = split["result"]["plugin_pane"]["pane"]["pane_id"]
        .as_str()
        .context("Missing new pane")?;
    herdr(&["pane", "rename", id, LABEL])?;
    // Native plugin panes start directly (no shell echo), with a half-width split.
    // Resize the right-edge target to retain the monitor's narrow footprint.
    let amount = 0.5 - monitor_width as f64 / width as f64;
    if amount > 0.001 {
        herdr(&[
            "pane",
            "resize",
            "--pane",
            split_target,
            "--direction",
            "right",
            "--amount",
            &format!("{amount:.4}"),
        ])?;
    }
    Ok(())
}
fn open_editor() -> Result<()> {
    anyhow::ensure!(
        std::env::var("HERDR_ENV").as_deref() == Ok("1"),
        "Edit requires Herdr; run ccsw in another terminal"
    );
    let workspace = std::env::var("HERDR_WORKSPACE_ID").context("Missing Herdr workspace")?;
    let opened = herdr(&[
        "plugin",
        "pane",
        "open",
        "--plugin",
        "ccsw",
        "--entrypoint",
        "editor",
        "--placement",
        "tab",
        "--workspace",
        &workspace,
        "--no-focus",
    ])?;
    let pane = &opened["result"]["plugin_pane"]["pane"];
    let tab_id = pane["tab_id"]
        .as_str()
        .context("Editor opened, but Herdr returned no tab ID")?;
    let pane_id = pane["pane_id"]
        .as_str()
        .context("Editor opened, but Herdr returned no pane ID")?;
    // Explicit navigation after creation also selects the new tab in the client.
    herdr(&["tab", "focus", tab_id])?;
    herdr(&["plugin", "pane", "focus", pane_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    #[test]
    fn reads_only_identified_agent_session() {
        let pane = json!({"result":{"pane":{"agent":"codex","agent_session":{
            "agent":"codex","kind":"id","value":"session-123"
        }}}});
        assert_eq!(agent_session(&pane).unwrap().id, "session-123");
        let mut wrong = pane.clone();
        wrong["result"]["pane"]["agent_session"]["agent"] = json!("claude");
        assert!(agent_session(&wrong).is_none());
        wrong["result"]["pane"]["agent_session"] = serde_json::Value::Null;
        assert!(agent_session(&wrong).is_none());
    }

    #[test]
    fn pulse_split_uses_the_outer_right_edge_not_the_invoking_pane() {
        let layout = json!({"result":{"layout":{
            "area":{"x":0,"width":300},
            "panes":[
                {"pane_id":"caller","rect":{"x":0,"width":140,"height":40}},
                {"pane_id":"middle","rect":{"x":140,"width":80,"height":40}},
                {"pane_id":"right","rect":{"x":220,"width":80,"height":40}}
            ]
        }}});
        assert_eq!(rightmost_split_target(&layout), Some(("right", 80)));
        let narrow = json!({"result":{"layout":{
            "area":{"x":0,"width":300},
            "panes":[
                {"pane_id":"caller","rect":{"x":0,"width":252,"height":40}},
                {"pane_id":"right","rect":{"x":252,"width":48,"height":40}}
            ]
        }}});
        assert_eq!(rightmost_split_target(&narrow), None);
    }

    #[test]
    fn focused_agent_tracks_the_active_pane_in_the_monitor_tab() {
        let mut panes = json!({"result":{"panes":[
            {"pane_id":"codex","tab_id":"tab-a","focused":true,"agent":"codex",
             "agent_session":{"agent":"codex","kind":"id","value":"codex-one"}},
            {"pane_id":"claude","tab_id":"tab-a","focused":false,"agent":"claude",
             "agent_session":{"agent":"claude","kind":"id","value":"claude-one"}},
            {"pane_id":"other","tab_id":"tab-b","focused":false,"agent":"claude"}
        ]}});
        let current = focused_agent(&panes, "tab-a").unwrap();
        assert_eq!(current.client, 1);
        assert_eq!(current.session.unwrap().id, "codex-one");

        panes["result"]["panes"][0]["focused"] = json!(false);
        panes["result"]["panes"][1]["focused"] = json!(true);
        let current = focused_agent(&panes, "tab-a").unwrap();
        assert_eq!(current.client, 0);
        assert_eq!(current.session.unwrap().id, "claude-one");

        panes["result"]["panes"][1]["agent_session"] = serde_json::Value::Null;
        let current = focused_agent(&panes, "tab-a").unwrap();
        assert_eq!(current.client, 0);
        assert!(current.session.is_none());
        assert!(focused_agent(&panes, "tab-b").is_none());
    }

    #[test]
    fn focus_subscription_uses_pane_updates_without_a_cli_query() {
        let event = json!({"event":"pane_updated","data":{"pane":{
            "pane_id":"pane-a","tab_id":"tab-a","focused":true,"agent":"codex",
            "agent_session":{"agent":"codex","kind":"id","value":"session-a"}
        }}});
        let focused =
            focus_from_event(&event, Some("workspace-a"), Some("tab-a"), "source").unwrap();
        assert_eq!(focused.pane_id, "pane-a");
        assert_eq!(focused.session.unwrap().id, "session-a");
        assert!(focus_from_event(&event, Some("workspace-a"), Some("tab-b"), "source").is_none());

        let (send, updates) = mpsc::sync_channel(1);
        let mut tracker = FocusTracker::default();
        assert!(tracker.observe(
            focus_from_event(&event, Some("workspace-a"), Some("tab-a"), "source"),
            &send,
            true,
        ));
        assert_eq!(updates.try_recv().unwrap().pane_id, "pane-a");
        assert!(tracker.observe(
            focus_from_event(&event, Some("workspace-a"), Some("tab-a"), "source"),
            &send,
            true,
        ));
        assert!(updates.try_recv().is_err());
        let mut cleared = event;
        cleared["data"]["pane"]["agent_session"] = serde_json::Value::Null;
        assert!(tracker.observe(
            focus_from_event(&cleared, Some("workspace-a"), Some("tab-a"), "source"),
            &send,
            true,
        ));
        assert!(updates.try_recv().unwrap().session.is_none());
    }

    #[test]
    fn sessions_filter_follows_focused_agent_even_when_session_id_is_missing() {
        let mut monitor = Monitor {
            sessions_mode: true,
            client: 2,
            ..Default::default()
        };
        monitor.sessions.rows = vec![
            crate::sessions::Session {
                id: "codex-one".into(),
                client: "Codex",
                ..Default::default()
            },
            crate::sessions::Session {
                id: "claude-one".into(),
                client: "Claude",
                ..Default::default()
            },
        ];
        monitor.apply_focus(FocusUpdate {
            pane_id: "codex-pane".into(),
            client: 1,
            session: Some(AgentSession {
                client: "Codex",
                id: "codex-one".into(),
            }),
        });
        assert_eq!(monitor.client(), Some("Codex"));
        assert_eq!(monitor.session_rows().len(), 1);
        monitor.client = 2; // A manual All selection lasts until focus changes.
        monitor.apply_focus(FocusUpdate {
            pane_id: "claude-pane".into(),
            client: 0,
            session: None,
        });
        assert_eq!(monitor.client(), Some("Claude"));
        assert_eq!(monitor.session_rows()[0].id, "claude-one");
        assert!(monitor.active_session.is_none());
    }

    #[test]
    fn gateway_filter_follows_focus_and_keeps_manual_selection_until_focus_moves() {
        let mut monitor = Monitor {
            client: 2,
            ..Default::default()
        };
        monitor.apply_focus(FocusUpdate {
            pane_id: "codex-pane".into(),
            client: 1,
            session: None,
        });
        assert_eq!(monitor.client(), Some("Codex"));
        monitor.client = 2;
        monitor.apply_focus(FocusUpdate {
            pane_id: "codex-pane".into(),
            client: 1,
            session: Some(AgentSession {
                client: "Codex",
                id: "another-session".into(),
            }),
        });
        assert_eq!(monitor.client(), None);
        monitor.apply_focus(FocusUpdate {
            pane_id: "claude-pane".into(),
            client: 0,
            session: None,
        });
        assert_eq!(monitor.client(), Some("Claude"));
    }

    #[test]
    fn transient_missing_agent_id_does_not_hide_current_session() {
        let mut previous = None;
        let mut missing = 0;
        let first = AgentSession {
            client: "Codex",
            id: "first".into(),
        };
        let second = AgentSession {
            client: "Codex",
            id: "second".into(),
        };
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, Some(first.clone())),
            Some(Some(first.clone()))
        );
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, None),
            None
        );
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, None),
            None
        );
        assert_eq!(previous, Some(first));
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, Some(second.clone())),
            Some(Some(second.clone()))
        );
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, None),
            None
        );
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, None),
            None
        );
        assert_eq!(
            stable_agent_session(&mut previous, &mut missing, None),
            Some(None)
        );
        assert_eq!(previous, None);
    }

    #[test]
    fn active_session_stays_first_across_sorts() {
        let mut m = Monitor {
            client: 2,
            active_session: Some(AgentSession {
                client: "Codex",
                id: "current".into(),
            }),
            ..Default::default()
        };
        m.sessions.rows = vec![
            crate::sessions::Session {
                id: "older".into(),
                client: "Codex",
                updated: 20,
                tokens: crate::sessions::Tokens {
                    input: 500,
                    known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            crate::sessions::Session {
                id: "current".into(),
                client: "Codex",
                updated: 10,
                ..Default::default()
            },
        ];
        assert_eq!(m.session_rows()[0].id, "current");
        m.sessions_sort_tokens = true;
        assert_eq!(m.session_rows()[0].id, "current");
    }

    #[test]
    fn home_shows_current_session_independently_of_gateway_and_history() {
        let mut m = Monitor {
            source_pane: Some("w1:p1".into()),
            active_session: Some(AgentSession {
                client: "Claude",
                id: "current-session".into(),
            }),
            client: 1,
            ..Default::default()
        };
        m.sessions.rows.push(crate::sessions::Session {
            id: "current-session".into(),
            client: "Claude",
            project: "/work/a-very-long-project-name".into(),
            tokens: crate::sessions::Tokens {
                input: 800,
                output: 200,
                read: 300,
                write: 100,
                known: true,
                cache_known: true,
            },
            ..Default::default()
        });
        let home = m
            .content(28)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(home.contains("SESSION / ALL TIME"));
        assert!(home.find("TOKENS").unwrap() < home.find("SESSION / ALL TIME").unwrap());
        assert!(home.contains(&digits("1000")[0]));
        assert!(home.contains("↑ Input 800  ·  ↓ Output 200"));
        assert!(home.contains("↺ Read 300  ·  Write 100"));
        assert!(home.contains("37.5%"));
        assert!(home.starts_with("TOKENS"));
        assert!(home.contains("Reading gateway usage"));
        assert!(
            m.content(28).iter().all(|line| line.width() <= 28),
            "overflow: {:?}",
            m.content(28)
                .iter()
                .filter(|line| line.width() > 28)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );

        m.sessions_mode = true;
        m.sessions_refreshed = Some(Instant::now());
        let history = m
            .content(28)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(history.contains("SESSION HISTORY"));
        assert!(history.contains("No local sessions for this client"));
        assert_eq!(history.matches(&digits("1000")[0]).count(), 1);
        m.client = 2;
        let all_history = m
            .content(28)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_history.contains("No other sessions"));
    }

    #[test]
    fn visual_view_keeps_exact_metrics_and_fits_narrow_panes() {
        let mut m = Monitor {
            visual_mode: true,
            source_pane: Some("w1:p1".into()),
            active_session: Some(AgentSession {
                client: "Codex",
                id: "current".into(),
            }),
            client: 1,
            refreshed: Some(Instant::now()),
            sessions_refreshed: Some(Instant::now()),
            ..Default::default()
        };
        m.sessions.rows.push(crate::sessions::Session {
            id: "current".into(),
            client: "Codex",
            project: "/work/project".into(),
            tokens: crate::sessions::Tokens {
                input: 800,
                output: 200,
                read: 300,
                write: 100,
                known: true,
                cache_known: true,
            },
            ..Default::default()
        });
        m.sessions.rows.push(crate::sessions::Session {
            id: "previous".into(),
            client: "Codex",
            project: "/work/project".into(),
            tokens: crate::sessions::Tokens {
                input: 80,
                output: 20,
                read: 30,
                write: 10,
                known: true,
                cache_known: true,
            },
            ..Default::default()
        });
        m.snapshot.rows.push(crate::usage::Row {
            day: m.snapshot.today(),
            hour: 12,
            client: "Codex".into(),
            provider: "test".into(),
            name: "Test provider".into(),
            kind: "generation".into(),
            totals: Totals {
                calls: 2,
                success: 1,
                failed: 1,
                input: 400,
                output: 100,
                cache_read: 200,
                cache_write: 50,
                cache_input: 400,
                cache_hits: 200,
                speed_output: 100,
                speed_ms: 2000,
                speed_samples: 1,
                ..Default::default()
            },
            model: "test-model".into(),
        });
        for width in [28, 36, 44] {
            let home = m.content(width);
            let text = home
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.find("TOKENS").unwrap() < text.find("SESSION / ALL TIME").unwrap());
            assert!(
                text.find("Calls / unknown").unwrap() < text.find("SESSION / ALL TIME").unwrap()
            );
            assert!(text.find("streams").unwrap() < text.find("SESSION / ALL TIME").unwrap());
            assert!(text.contains(&digits("500")[0]));
            for expected in [
                &digits("1000")[0],
                "I 800  O 200",
                "R 300 W 100",
                "37.5%",
                "500",
                "400",
                "100",
                "50.0%",
                "50.0 tok/s",
                "✓ 1",
                "× 1",
                "Test provider",
            ] {
                assert!(text.contains(expected), "missing {expected}: {text}");
            }
            assert!(home.iter().all(|line| line.width() <= width as usize));
            m.sessions_mode = true;
            let history = m.content(width);
            let text = history
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            for expected in ["SESSION HISTORY", "previous", "I 80  O 20", "R 30 W 10"] {
                assert!(text.contains(expected), "missing {expected}: {text}");
            }
            assert!(history.iter().all(|line| line.width() <= width as usize));
            m.sessions_mode = false;
        }
        let mut terminal = ratatui::Terminal::new(TestBackend::new(48, 30)).unwrap();
        terminal.draw(|frame| m.draw(frame)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "T")
        );
    }

    #[test]
    fn graphical_gateway_strips_keep_all_values_at_narrow_widths() {
        let totals = Totals {
            calls: 2,
            unknown: 1,
            input: 1536,
            output: 80,
            cache_read: 1536,
            cache_write: 0,
            cache_input: 125_000,
            cache_hits: 1500,
            speed_output: 196,
            speed_ms: 5000,
            speed_samples: 2,
            ..Default::default()
        };
        for width in [28, 36, 44] {
            let lines = gateway_cache_meter(&totals, false, width);
            let cache = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(cache.contains("HIT"));
            assert!(cache.contains("1.2%"));
            assert!(cache.contains("R 1536 W 0") || cache.contains("1536 / 0"));
            assert!(lines.iter().all(|line| line.width() <= width as usize));
        }
        let mut m = Monitor {
            visual_mode: true,
            refreshed: Some(Instant::now()),
            ..Default::default()
        };
        m.snapshot.rows.push(crate::usage::Row {
            day: m.snapshot.today(),
            hour: 12,
            client: "Claude".into(),
            provider: "test".into(),
            name: "Test".into(),
            kind: "generation".into(),
            totals,
            model: "test".into(),
        });
        let text = m
            .content(40)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for expected in [
            "I 1536  O 80",
            "● Calls / unknown",
            "2 / 1",
            "1.2%",
            "R 1536 W 0",
            "39.2 tok/s",
            "2 measured streams",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        assert!(!text.contains("Cache read / write"));
        assert!(!text.contains("Measured streams"));
    }

    #[test]
    fn charts_show_gateway_calls_and_current_session_token_activity() {
        let mut m = Monitor {
            source_pane: Some("w1:p1".into()),
            active_session: Some(AgentSession {
                client: "Claude",
                id: "current".into(),
            }),
            chart_mode: true,
            ..Default::default()
        };
        m.snapshot.offset = 0;
        let noon = chrono::NaiveDate::parse_from_str(&m.snapshot.today(), "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let mut session = crate::sessions::Session {
            id: "current".into(),
            client: "Claude",
            ..Default::default()
        };
        session.activity.insert(noon, 120);
        m.sessions.rows.push(session);
        m.snapshot.rows.push(crate::usage::Row {
            day: m.snapshot.today(),
            hour: 12,
            client: "Claude".into(),
            provider: "test".into(),
            name: "Test".into(),
            kind: "generation".into(),
            totals: Totals {
                calls: 2,
                ..Default::default()
            },
            model: "test".into(),
        });
        assert_eq!(m.session_hours_today()[12], 120);
        let text = m
            .content(42)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("GATEWAY / REQUESTS BY HOUR"));
        assert!(text.contains("SESSION / TOKENS BY HOUR"));
        assert!(text.contains("120"));
        assert!(!text.contains("PROVIDERS / TODAY"));
    }

    #[test]
    fn scrollbar_track_and_provider_header_are_mouse_targets() {
        let body = Rect::new(2, 4, 42, 20);
        assert_eq!(scrollbar_target(body, 47, 47, 4, 100), Some(0));
        assert_eq!(scrollbar_target(body, 47, 47, 23, 100), Some(100));
        assert_eq!(scrollbar_target(body, 47, 46, 23, 100), None);
        let mut m = Monitor {
            refreshed: Some(Instant::now()),
            ..Default::default()
        };
        let header = m
            .content(body.width)
            .iter()
            .position(|line| line.to_string().starts_with("PROVIDERS / TODAY"))
            .unwrap() as u16;
        m.scroll = header;
        assert!(m.provider_header_hit(body, body.x + 2, body.y));
        assert!(!m.provider_header_hit(body, body.x + 2, body.y + 1));
    }
    #[test]
    fn sidepane_sessions_render_and_filter_without_proxy_usage() {
        let mut m = Monitor {
            sessions_mode: true,
            sessions_refreshed: Some(Instant::now()),
            client: 2,
            ..Default::default()
        };
        m.sessions.rows = vec![
            crate::sessions::Session {
                id: "codex-session-123".into(),
                client: "Codex",
                project: "/work/project".into(),
                updated: 200,
                tokens: crate::sessions::Tokens {
                    input: 900,
                    output: 100,
                    read: 700,
                    known: true,
                    cache_known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            crate::sessions::Session {
                id: "claude-session-456".into(),
                client: "Claude",
                updated: 100,
                tokens: crate::sessions::Tokens {
                    input: 10,
                    output: 2,
                    known: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        for (width, height) in [(32, 12), (40, 28), (48, 46)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| m.draw(f)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(
                text.contains("(s)"),
                "Sessions button must fit {width} columns"
            );
            assert!(text.contains("SESSION"), "Session view must be visible");
            assert!(!text.contains("TODAY / USAGE"));
        }
        assert_eq!(m.session_rows().len(), 2);
        m.client = 0;
        assert_eq!(m.session_rows().len(), 1);
        assert_eq!(m.session_rows()[0].client, "Claude");
        m.client = 2;
        m.sessions_sort_tokens = true;
        assert_eq!(m.session_rows()[0].id, "codex-session-123");
        let body = m
            .content(48)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(body.contains("1000 tok"));
        assert!(body.contains("700 read"));
    }
    #[test]
    fn cache_and_request_rate_display_matching_sample_totals() {
        let mut m = Monitor {
            refreshed: Some(Instant::now()),
            source_pane: Some("w1:p1".into()),
            active_session: Some(AgentSession {
                client: "Claude",
                id: "current".into(),
            }),
            ..Default::default()
        };
        m.sessions.rows.push(crate::sessions::Session {
            id: "current".into(),
            client: "Claude",
            ..Default::default()
        });
        m.snapshot.rows.push(crate::usage::Row {
            hour: 0,
            model: "test".into(),
            day: m.snapshot.today(),
            client: "Claude".into(),
            provider: "p".into(),
            name: "P".into(),
            kind: "generation".into(),
            totals: Totals {
                calls: 2,
                success: 2,
                input: 200,
                output: 100,
                cache_hits: 120448,
                cache_input: 120675,
                speed_output: 100,
                speed_ms: 4000,
                speed_samples: 2,
                ..Default::default()
            },
        });
        let text = m
            .content(48)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("99.8%"));
        assert!(text.contains("↑ Input") && text.contains("↓ Output"));
        assert!(text.contains("Output rate (E2E)"));
        assert!(text.contains("25.0 tok/s"));
        assert!(text.contains("2 measured streams"));
        assert!(text.find("REQUESTS").unwrap() < text.find("SESSION / ALL TIME").unwrap());
        assert!(
            text.find("2 measured streams").unwrap() < text.find("SESSION / ALL TIME").unwrap()
        );
    }
    #[test]
    fn invoking_agent_selects_the_initial_statistics_tab() {
        for agent in ["claude", "claude-code", "Claude Code", "claudecode"] {
            assert_eq!(initial_client(Some(agent)), 0);
        }
        assert_eq!(initial_client(Some("codex")), 1);
        for agent in [None, Some(""), Some("pi"), Some("unknown"), Some("all")] {
            assert_eq!(initial_client(agent), 2);
        }
    }
    #[test]
    fn hourly_chart_uses_six_rows_full_width_and_preserves_zero_hours() {
        for width in [28, 36, 44, 60] {
            let empty = hourly_chart(&[0; 24], width, BLUE);
            assert_eq!(empty.len(), 8);
            assert!(empty.iter().all(|line| line.width() == usize::from(width)));
            assert!(!empty.iter().any(|line| line.to_string().contains('█')));
            let mut hours = [0; 24];
            hours[12] = 10;
            let chart = hourly_chart(&hours, width, BLUE);
            assert!(chart[..6].iter().all(|line| line.to_string().contains('█')));
            assert!(chart.last().unwrap().to_string().contains("23"));
        }
    }
    #[test]
    fn scope_explanation_is_only_in_help() {
        let mut m = Monitor {
            refreshed: Some(Instant::now()),
            ..Default::default()
        };
        let text = |m: &Monitor| {
            m.content(44)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(!text(&m).contains("subscription"));
        m.help = true;
        assert!(text(&m).contains("subscription"));
        assert!(text(&m).contains("gateway totals"));
    }
    #[test]
    fn independent_layout_and_scrolling_at_narrow_sizes() {
        for (width, height) in [(32, 12), (40, 28), (48, 46)] {
            let mut m = Monitor {
                refreshed: Some(Instant::now()),
                ..Default::default()
            };
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| m.draw(f)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("GATEWAY / TODAY"));
            assert!(text.contains("(e)"));
            assert!(text.contains("(c)"));
            assert!(text.contains("(r)"));
            assert!(text.contains("(q)"));
            assert!(!text.contains("e edit ·"));
            assert!(!text.contains("Configurations"));
            m.scroll = u16::MAX;
            terminal.draw(|f| m.draw(f)).unwrap();
            assert!(m.scroll <= m.limit);
        }
    }
    #[test]
    fn health_needs_samples_and_excludes_pending() {
        assert_eq!(rate(&Totals::default()), None);
        assert_eq!(
            rate(&Totals {
                pending: 4,
                ..Default::default()
            }),
            None
        );
        assert_eq!(
            rate(&Totals {
                success: 8,
                failed: 1,
                interrupted: 1,
                pending: 90,
                ..Default::default()
            }),
            Some(80.0)
        );
    }

    #[test]
    fn visual_health_bar_shows_failures_and_interruptions() {
        let totals = Totals {
            success: 20,
            failed: 1,
            interrupted: 1,
            pending: 90,
            ..Default::default()
        };
        let bar = health_meter(&totals, 40);
        assert_eq!(bar.width(), 40);
        assert_eq!(bar.spans[1].style.fg, Some(GREEN));
        assert_eq!(bar.spans[2].style.fg, Some(RED));
        assert_eq!(bar.spans[3].style.fg, Some(GOLD));
        assert!(!bar.spans[2].content.is_empty());
        assert!(!bar.spans[3].content.is_empty());
        let empty = health_meter(
            &Totals {
                pending: 3,
                ..Default::default()
            },
            40,
        );
        assert!(!empty.to_string().contains('█'));
    }

    #[test]
    fn unavailable_tokens_are_not_displayed_as_zero() {
        let t = Totals {
            calls: 4,
            unknown: 4,
            ..Default::default()
        };
        assert_eq!(token_label(&t), "unknown");
        assert_eq!(
            token_label(&Totals {
                calls: 4,
                unknown: 1,
                input: 100,
                ..Default::default()
            }),
            "100+?"
        );
        assert_eq!(token_label(&Totals::default()), "0");
        let mut m = Monitor {
            refreshed: Some(Instant::now()),
            ..Default::default()
        };
        m.snapshot.rows.push(crate::usage::Row {
            hour: 1,
            model: "m".into(),
            day: m.snapshot.today(),
            client: "Claude".into(),
            provider: "p".into(),
            name: "P".into(),
            kind: "generation".into(),
            totals: t,
        });
        let content = m
            .content(28)
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains("4 calls lack token data"));
        assert!(content.contains("↺ Read") && content.contains("Write"));
        assert!(!content.contains("0 tok"));
        assert!(content.contains("no samples"));
    }
    #[test]
    fn today_excludes_history_and_other_clients_but_counts_compaction() {
        let mut s = Snapshot::default();
        for (day, client, kind, calls) in [
            (s.today(), "Claude", "generation", 10),
            (s.today(), "Claude", "compact", 2),
            ("".into(), "Claude", "generation", 1000),
            (s.today(), "Codex", "generation", 30),
        ] {
            s.rows.push(crate::usage::Row {
                hour: 9,
                model: "model".into(),
                day,
                client: client.into(),
                provider: "host".into(),
                name: "Host".into(),
                kind: kind.into(),
                totals: Totals {
                    calls,
                    success: calls,
                    ..Default::default()
                },
            });
        }
        let m = metrics(&s, Some("Claude"));
        assert_eq!(m.total.calls, 12);
        assert_eq!(m.compact, 2);
        assert_eq!(m.hours[9], 12);
        assert_eq!(metrics(&s, None).total.calls, 42);
    }
}
