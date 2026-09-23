//! Read-only monitor, independent of the configuration editor and its sync loop.
use super::*;
use crate::usage::{Query, Reader, Snapshot, Totals};
use chrono::Timelike;
use std::{
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

const BG: Color = Color::Rgb(20, 30, 42);
const INK: Color = Color::Rgb(223, 233, 240);
const SOFT: Color = Color::Rgb(139, 161, 181);
const BLUE: Color = Color::Rgb(123, 190, 218);
const GOLD: Color = Color::Rgb(234, 193, 126);
const RED: Color = Color::Rgb(236, 139, 131);
const GREEN: Color = Color::Rgb(147, 204, 178);
const RAIL: Color = Color::Rgb(48, 67, 84);
const LABEL: &str = "CCSW Pulse";

fn initial_client(agent: Option<&str>) -> usize {
    match agent.unwrap_or("").to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" | "claude code" => 0,
        "codex" => 1,
        _ => 2,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentSession {
    client: &'static str,
    id: String,
}

fn agent_session(pane: &serde_json::Value) -> Option<AgentSession> {
    let pane = &pane["result"]["pane"];
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
    snapshot: Snapshot,
    sessions: crate::sessions::Snapshot,
    sessions_refreshed: Option<Instant>,
    sessions_mode: bool,
    chart_mode: bool,
    visual_mode: bool,
    sessions_sort_tokens: bool,
    source_pane: Option<String>,
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
    Line::from(vec![
        Span::styled(
            format!("{title}{}", "─".repeat(gap)),
            Style::default().fg(SOFT),
        ),
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
        let Some(current) = &self.active_session else {
            out.push(line("◌ Waiting for agent session ID", SOFT));
            out.push(line("Open or resume a session in this pane", SOFT));
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
        if !s.tokens.known {
            out.push(line("Token usage unavailable in local log", SOFT));
            return out;
        }
        out.extend(token_digits(&short(s.tokens.total()), GREEN));
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
        out.push(if unknown {
            line("I/O ░░░░░░░░░░ I ? O ?", SOFT)
        } else {
            compact_token_meter(t.input, t.output, width)
        });
        out.push(pair(
            "● Calls / unknown",
            format!("{} / {}", t.calls, t.unknown),
            width,
            INK,
        ));
        out.extend(gateway_cache_meter(t, unknown, width));
        let speed = if t.speed_ms > 0 {
            format!(
                "{:.1} tok/s",
                t.speed_output as f64 * 1000.0 / t.speed_ms as f64
            )
        } else {
            "—".into()
        };
        out.push(pair(
            &format!("↗ Rate {speed}"),
            format!("{} streams", t.speed_samples),
            width,
            BLUE,
        ));
        let active = self.visual_active_content(width);
        if !active.is_empty() {
            out.push(Line::default());
            out.extend(active);
        }
        out.push(Line::default());
        out.push(section("CALL HEALTH", width));
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
        out.push(Line::default());
        out.push(if self.models {
            section_action("MODELS / TODAY", "[Providers m]", width)
        } else {
            section_action("PROVIDERS / TODAY", "[Models m]", width)
        });
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
        for (name, totals) in entries {
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
                clipped("Open or resume a session in this pane", width.into()),
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
        out.extend(token_digits(&short(s.tokens.total()), GREEN));
        if !suffix.is_empty() {
            out.push(line("+? partial token log", GOLD));
        }
        out.push(pair(
            "Input / Output",
            format!("{} / {}", short(s.tokens.input), short(s.tokens.output)),
            width,
            INK,
        ));
        out.push(pair(
            "Cache read / write",
            if s.tokens.cache_known {
                format!("{} / {}", short(s.tokens.read), short(s.tokens.write))
            } else {
                "— / —".into()
            },
            width,
            SOFT,
        ));
        out.push(pair(
            "Session cache reuse",
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
                line("for the attached agent pane.", INK),
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
                Line::default(),
                line("Output rate: successful", SOFT),
                line("streams, all output tokens /", SOFT),
                line("request start to completion.", SOFT),
                line("Includes first-token wait,", SOFT),
                line("reasoning and network time.", SOFT),
                line("Weighted average today;", SOFT),
                line("not pure model decode speed.", SOFT),
                line("Old/unmeasured calls excluded.", SOFT),
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
        out.push(line(
            if unknown {
                "Input —  ·  Output —".into()
            } else {
                format!("Input {}  ·  Output {}", short(t.input), short(t.output))
            },
            SOFT,
        ));
        out.push(line(
            if t.unknown > 0 {
                format!("{} calls: tokens unknown", t.unknown)
            } else {
                "Input + output · gateway usage".into()
            },
            SOFT,
        ));
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
        out.push(pair(
            "Gateway cache read / write",
            if unknown {
                "— / —".into()
            } else {
                format!("{} / {}", short(t.cache_read), short(t.cache_write))
            },
            width,
            SOFT,
        ));
        out.push(pair(
            "Gateway cache hit",
            if t.cache_input > 0 {
                format!("{:.1}%", 100.0 * t.cache_hits as f64 / t.cache_input as f64)
            } else {
                "—".into()
            },
            width,
            BLUE,
        ));
        out.push(pair(
            "Output rate (E2E)",
            if t.speed_ms > 0 {
                format!(
                    "{:.1} tok/s",
                    t.speed_output as f64 * 1000.0 / t.speed_ms as f64
                )
            } else {
                "—".into()
            },
            width,
            BLUE,
        ));
        out.push(line(
            format!("{} measured streams today", t.speed_samples),
            SOFT,
        ));
        let active = self.active_content(width);
        if !active.is_empty() {
            out.push(Line::default());
            out.extend(active);
        }
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
        for (name, t) in entries {
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
                line("Current: this agent pane.", INK),
                line("Local Claude / Codex logs.", INK),
                line("Each row: whole session", SOFT),
                line("tokens, not today's usage.", SOFT),
                line("Input includes cached tokens.", SOFT),
                line("Cache reuse = read / input.", SOFT),
                line("Writes are not cache hits.", SOFT),
                line("Forks may include inherited", SOFT),
                line("tokens; children are separate.", SOFT),
                line("Current follows agent pane.", SOFT),
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
            Line::default(),
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
            out.push(Line::default());
        }
        if self.sessions.warnings > 0 {
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
            f.render_widget(
                Paragraph::new("CCSW Pulse\nResize pane to 32 × 12\nq to close")
                    .style(Style::default().fg(INK)),
                area,
            );
            return;
        }
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
                    "● Sessions updated {}s ago · refresh 2s",
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
                "● Charts update every 2s · c: Home".into()
            }
        } else if let Some(error) = &self.error {
            format!("! STALE · {error}")
        } else if let Some(note) = &self.notice {
            note.clone()
        } else if let Some(time) = self.refreshed {
            format!("● Updated {}s ago · refresh 2s", time.elapsed().as_secs())
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
) {
    std::thread::spawn(move || {
        let mut reader = crate::sessions::Reader::default();
        loop {
            let snapshot = match crate::sessions::roots() {
                Ok(roots) => reader.read(&roots),
                Err(_) => crate::sessions::Snapshot {
                    rows: vec![],
                    warnings: 1,
                },
            };
            if session_send.send(snapshot).is_err() {
                break;
            }
            if matches!(
                session_requests.recv_timeout(Duration::from_secs(2)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ) {
                break;
            }
        }
    });
}

pub(super) fn run(paths: AppPaths) -> Result<()> {
    let (send, updates) = mpsc::sync_channel(1);
    let (refresh, requests) = mpsc::sync_channel(1);
    let (session_send, session_updates) = mpsc::sync_channel(1);
    let (session_refresh, session_requests) = mpsc::sync_channel(1);
    let source_pane = std::env::var("CCSW_MONITOR_SOURCE_PANE")
        .ok()
        .filter(|id| !id.is_empty());
    let (active_send, active_updates) = mpsc::sync_channel(1);
    if let Some(source) = source_pane.clone() {
        std::thread::spawn(move || {
            let mut previous = None;
            let mut missing = 0;
            loop {
                if let Ok(pane) = herdr(&["pane", "get", &source])
                    && let Some(update) =
                        stable_agent_session(&mut previous, &mut missing, agent_session(&pane))
                    && active_send.send(update).is_err()
                {
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
        client: initial_client(std::env::var("CCSW_MONITOR_CLIENT").ok().as_deref()),
        source_pane,
        ..Default::default()
    };
    if monitor.source_pane.is_some() {
        session_reader_started = true;
        let (session_send, session_requests) = session_worker.take().unwrap();
        spawn_session_reader(session_send, session_requests);
    }
    loop {
        while let Ok(current) = active_updates.try_recv() {
            if monitor.active_session != current {
                if let Some(active) = &current
                    && monitor.client != 2
                    && monitor.client() != Some(active.client)
                {
                    monitor.client = initial_client(Some(active.client));
                }
                monitor.active_session = current;
                if monitor.sessions_mode {
                    monitor.scroll = 0;
                }
            }
        }
        while let Ok(snapshot) = session_updates.try_recv() {
            monitor.sessions = snapshot;
            monitor.sessions_refreshed = Some(Instant::now());
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
        }
        terminal.draw(|f| monitor.draw(f))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let key = match event::read()? {
            Event::Key(k) if k.kind == event::KeyEventKind::Press => Some(k),
            Event::Mouse(m) => {
                let size = terminal.size()?;
                let area = Rect::new(0, 0, size.width, size.height).inner(Margin::new(2, 0));
                let body = content_body(area);
                let dragging = matches!(
                    m.kind,
                    MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left)
                );
                let code = if dragging {
                    if let Some(target) = scrollbar_target(
                        body,
                        area.right().saturating_sub(1),
                        m.column,
                        m.row,
                        monitor.limit,
                    ) {
                        monitor.scroll = target;
                        None
                    } else if m.kind == MouseEventKind::Down(MouseButton::Left)
                        && monitor.provider_header_hit(body, m.column, m.row)
                    {
                        monitor.models = !monitor.models;
                        None
                    } else {
                        match m.kind {
                            MouseEventKind::Down(MouseButton::Left)
                                if m.row == 0 && m.column >= area.right().saturating_sub(4) =>
                            {
                                Some(KeyCode::Char('?'))
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if m.row == 0 && m.column >= area.right().saturating_sub(9) =>
                            {
                                Some(KeyCode::Char('v'))
                            }
                            MouseEventKind::Down(MouseButton::Left) if m.row == 2 => {
                                let tabs = Layout::horizontal([Constraint::Ratio(1, 3); 3])
                                    .split(Rect::new(area.x, 2, area.width, 1));
                                if let Some(i) =
                                    tabs.iter().position(|r| contains(*r, m.column, m.row))
                                {
                                    monitor.client = i;
                                    monitor.scroll = 0;
                                }
                                None
                            }
                            MouseEventKind::Down(MouseButton::Left) => buttons(area)
                                .iter()
                                .position(|r| contains(*r, m.column, m.row))
                                .map(|i| {
                                    [
                                        KeyCode::Char('e'),
                                        KeyCode::Char(if monitor.sessions_mode {
                                            't'
                                        } else {
                                            'c'
                                        }),
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
                    if monitor.sessions_mode && !session_reader_started {
                        session_reader_started = true;
                        let (session_send, session_requests) = session_worker.take().unwrap();
                        spawn_session_reader(session_send, session_requests);
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
    let width = layout["result"]["layout"]["panes"]
        .as_array()
        .and_then(|panes| panes.iter().find(|p| p["pane_id"] == target))
        .and_then(|p| p["rect"]["width"].as_u64())
        .context("Missing pane width")?;
    anyhow::ensure!(
        width >= 76,
        "This pane is too narrow to split; expand it to at least 76 columns"
    );
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
        target,
        "--direction",
        "right",
        "--cwd",
        &cwd.to_string_lossy(),
        "--env",
        &client_env,
        "--env",
        &source_env,
        "--no-focus",
    ])?;
    let id = split["result"]["plugin_pane"]["pane"]["pane_id"]
        .as_str()
        .context("Missing new pane")?;
    herdr(&["pane", "rename", id, LABEL])?;
    // Native plugin panes start directly (no shell echo), with a half-width split.
    // Grow the source pane to retain the monitor's narrow footprint.
    let amount = 0.5 - monitor_width as f64 / width as f64;
    if amount > 0.001 {
        herdr(&[
            "pane",
            "resize",
            "--pane",
            target,
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
        assert!(home.contains("800 / 200"));
        assert!(home.contains("300 / 100"));
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
            "2 streams",
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
        assert!(text.contains("Input 200  ·  Output 100"));
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
        assert!(content.contains("4 calls: tokens unknown"));
        assert!(content.contains("— / —"));
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
