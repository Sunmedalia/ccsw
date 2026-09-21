//! Read-only monitor, independent of the configuration editor and its sync loop.
use super::*;
use crate::usage::{Query, Reader, Snapshot, Totals};
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

#[derive(Default)]
struct Monitor {
    snapshot: Snapshot,
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
fn line(text: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(Span::styled(text.into(), Style::default().fg(color)))
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
fn buttons(area: Rect) -> Vec<Rect> {
    let constraints = if area.width >= 34 {
        vec![
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Length(6),
            Constraint::Length(6),
        ]
    } else {
        vec![Constraint::Ratio(1, 4); 4]
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

fn hourly_chart(hours: &[i64; 24], width: u16) -> Vec<Line<'static>> {
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
                Style::default().fg(BLUE),
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
    fn content(&self, width: u16) -> Vec<Line<'static>> {
        if self.help {
            return vec![
                section("ABOUT THIS DATA", width),
                Line::default(),
                line("Only this CCSW config's", INK),
                line("gateway traffic is counted.", INK),
                Line::default(),
                line("Not a single Claude session", SOFT),
                line("or subscription quota.", SOFT),
                line("Direct API and subscription", SOFT),
                line("traffic are not tracked.", SOFT),
                Line::default(),
                line("Success rate excludes pending", SOFT),
                line("requests; failures and", SOFT),
                line("interruptions count against it.", SOFT),
                Line::default(),
                line("Chart: calls per hour today.", SOFT),
                line("Height scales to the peak.", SOFT),
                Line::default(),
                line("Cache hit = cached reads /", SOFT),
                line("all input in measured calls.", SOFT),
                line("Cache writes are not hits.", SOFT),
                Line::default(),
                line("Output speed: successful", SOFT),
                line("streams, output tokens /", SOFT),
                line("first output to completion.", SOFT),
                line("Excludes first-token wait.", SOFT),
                line("Weighted average today;", SOFT),
                line("network delay is included.", SOFT),
                line("Old/unmeasured calls excluded.", SOFT),
                Line::default(),
                line("? / Esc to return", BLUE),
            ];
        }
        let m = metrics(&self.snapshot, self.client());
        let t = &m.total;
        let ready = self.refreshed.is_some();
        if !ready {
            return vec![
                section("TODAY / USAGE", width),
                Line::default(),
                line("— Waiting for usage data", SOFT),
                line("Health needs completed requests", SOFT),
            ];
        }
        let unknown = t.calls > 0 && t.unknown == t.calls;
        let value = if !ready || (t.calls > 0 && t.unknown == t.calls) {
            "—".into()
        } else {
            short(t.input + t.output)
        };
        let mut out = vec![
            pair("TOKENS / TODAY", self.snapshot.today(), width, SOFT),
            Line::default(),
        ];
        out.extend(digits(&value).into_iter().map(|s| line(s, BLUE)));
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
            "↑ Input / ↓ Output",
            if unknown {
                "— / —".into()
            } else {
                format!("{} / {}", short(t.input), short(t.output))
            },
            width,
            BLUE,
        ));
        out.push(pair(
            "↺ Cache read / write",
            if unknown {
                "— / —".into()
            } else {
                format!("{} / {}", short(t.cache_read), short(t.cache_write))
            },
            width,
            SOFT,
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
        out.push(pair(
            "Output speed",
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
        out.push(section("24H / REQUESTS", width));
        let max = m.hours.iter().copied().max().unwrap_or(0);
        out.push(pair(
            "Peak hour",
            if max == 0 {
                "—".into()
            } else {
                format!("{max} calls")
            },
            width,
            INK,
        ));
        out.extend(hourly_chart(&m.hours, width));
        out.push(Line::default());
        out.push(section(
            if self.models {
                "MODELS / TODAY"
            } else {
                "PROVIDERS / TODAY"
            },
            width,
        ));
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
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "◈ CCSW ",
                    Style::default().fg(INK).add_modifier(Modifier::BOLD),
                ),
                Span::styled("P U L S E", Style::default().fg(BLUE)),
            ])),
            Rect::new(inner.x, inner.y, inner.width, 1),
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
        let body = Rect::new(
            inner.x,
            inner.y + 4,
            inner.width,
            inner.height.saturating_sub(7),
        );
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
                Rect::new(area.right() - 1, body.y, 1, body.height),
                &mut state,
            );
        }
        let status = if let Some(error) = &self.error {
            format!("! STALE · {error}")
        } else if let Some(note) = &self.notice {
            note.clone()
        } else if let Some(time) = self.refreshed {
            format!("● Updated {}s ago · refresh 2s", time.elapsed().as_secs())
        } else {
            "◌ Reading local usage…".into()
        };
        f.render_widget(
            Paragraph::new(status).style(Style::default().fg(if self.error.is_some() {
                RED
            } else {
                SOFT
            })),
            Rect::new(inner.x, area.bottom() - 3, inner.width, 1),
        );
        for (label, rect) in [
            if inner.width >= 34 {
                "↗ Edit(e)"
            } else {
                "↗(e)"
            },
            if inner.width < 34 {
                "≡(d)"
            } else if self.models {
                "≡ Hosts(d)"
            } else {
                "≡ Models(d)"
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

pub(super) fn run(paths: AppPaths) -> Result<()> {
    let (send, updates) = mpsc::sync_channel(1);
    let (refresh, requests) = mpsc::sync_channel(1);
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
        ..Default::default()
    };
    loop {
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
                let code = match m.kind {
                    MouseEventKind::ScrollDown => Some(KeyCode::Down),
                    MouseEventKind::ScrollUp => Some(KeyCode::Up),
                    MouseEventKind::Down(MouseButton::Left)
                        if m.row == 0 && m.column >= area.right().saturating_sub(4) =>
                    {
                        Some(KeyCode::Char('?'))
                    }
                    MouseEventKind::Down(MouseButton::Left) if m.row == 2 => {
                        let tabs = Layout::horizontal([Constraint::Ratio(1, 3); 3])
                            .split(Rect::new(area.x, 2, area.width, 1));
                        if let Some(i) = tabs.iter().position(|r| contains(*r, m.column, m.row)) {
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
                                KeyCode::Char('d'),
                                KeyCode::Char('r'),
                                KeyCode::Char('q'),
                            ][i]
                        }),
                    _ => None,
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
                KeyCode::Char('d') => {
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
                KeyCode::Char('r') => {
                    let _ = refresh.try_send(());
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
                                .as_ref()
                                == Some(&binary)
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
            let empty = hourly_chart(&[0; 24], width);
            assert_eq!(empty.len(), 8);
            assert!(empty.iter().all(|line| line.width() == usize::from(width)));
            assert!(!empty.iter().any(|line| line.to_string().contains('█')));
            let mut hours = [0; 24];
            hours[12] = 10;
            let chart = hourly_chart(&hours, width);
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
        assert!(text(&m).contains("gateway traffic"));
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
            assert!(text.contains("P U L S E"));
            assert!(text.contains("(e)"));
            assert!(text.contains("(d)"));
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
