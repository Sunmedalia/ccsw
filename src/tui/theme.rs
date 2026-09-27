use super::*;

// Semantic paint tokens. Only marked panel edges are restyled; chart and text
// glyphs are left intact. This keeps the same hit targets in every theme.
pub(super) const SURFACE: Color = Color::Rgb(1, 2, 3);
pub(super) const EDGE: Color = Color::Rgb(1, 2, 4);
pub(super) const ACTIVE_EDGE: Color = Color::Rgb(1, 2, 5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Theme {
    Classic,
    #[default]
    Slate,
    Moss,
    Sand,
    Plum,
    Pulse,
}

impl Theme {
    pub(super) const ALL: [Self; 6] = [
        Self::Classic,
        Self::Slate,
        Self::Moss,
        Self::Sand,
        Self::Plum,
        Self::Pulse,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic / terminal",
            Self::Slate => "Graphite / quiet workspace",
            Self::Moss => "Tundra / framed console",
            Self::Sand => "Paper / light ledger",
            Self::Plum => "Nightfall / soft panels",
            Self::Pulse => "Pulse / instrument panel",
        }
    }

    pub(super) fn load(paths: &AppPaths) -> Self {
        std::fs::read(paths.state_dir.join("tui-theme.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(self, paths: &AppPaths) -> Result<()> {
        crate::codex::atomic_write(
            &paths.state_dir.join("tui-theme.json"),
            &serde_json::to_vec(&self)?,
        )
    }

    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Classic => "Terminal background · square edges · compact highlights",
            Self::Slate => "Graphite canvas · quiet rails · raised selection",
            Self::Moss => "Forest panels · heavy frames · bold navigation",
            Self::Sand => "Warm paper · ruled frames · underlined selection",
            Self::Plum => "Midnight canvas · rounded panels · soft emphasis",
            Self::Pulse => "Blue instruments · double frames · bright readouts",
        }
    }

    fn edge(self, symbol: &str) -> &str {
        let index = match symbol {
            "┌" => 0,
            "┐" => 1,
            "└" => 2,
            "┘" => 3,
            "─" => 4,
            "│" => 5,
            _ => return symbol,
        };
        (match self {
            Self::Classic => ["┌", "┐", "└", "┘", "─", "│"],
            Self::Slate => ["▏", "▕", "▏", "▕", " ", "│"],
            Self::Moss => ["┏", "┓", "┗", "┛", "━", "┃"],
            Self::Sand => ["┌", "┐", "└", "┘", "─", "│"],
            Self::Plum => ["╭", "╮", "╰", "╯", "─", "│"],
            Self::Pulse => ["╔", "╗", "╚", "╝", "═", "║"],
        })[index]
    }

    pub(super) fn apply(self, buffer: &mut ratatui::buffer::Buffer) {
        self.apply_region(buffer, buffer.area);
    }

    fn apply_region(self, buffer: &mut ratatui::buffer::Buffer, region: Rect) {
        let palette = self.palette();
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                let cell = &mut buffer[(x, y)];
                let edge = matches!(cell.fg, EDGE | ACTIVE_EDGE);
                if edge {
                    let symbol = self.edge(cell.symbol()).to_owned();
                    cell.set_symbol(&symbol);
                    cell.fg = if cell.fg == ACTIVE_EDGE {
                        ROUTE
                    } else {
                        Color::DarkGray
                    };
                }
                let selected = cell.bg == SELECTION;
                if selected {
                    cell.modifier |= match self {
                        Self::Sand => Modifier::UNDERLINED,
                        Self::Moss | Self::Pulse => Modifier::BOLD,
                        _ => Modifier::empty(),
                    };
                }
                let Some(p) = &palette else {
                    if cell.bg == SURFACE {
                        cell.bg = Color::Reset;
                    }
                    continue;
                };
                cell.fg = if matches!(cell.fg, Color::Reset | Color::White)
                    && cell.modifier.contains(Modifier::BOLD)
                {
                    p.heading
                } else if cell.fg == Color::Reset {
                    p.text
                } else if cell.fg == Color::Black {
                    p.on_accent
                } else {
                    p.color(cell.fg)
                };
                cell.bg = if cell.bg == SURFACE {
                    p.surface
                } else if cell.bg == Color::Reset {
                    p.background
                } else {
                    p.color(cell.bg)
                };
            }
        }
    }

    fn palette(self) -> Option<Palette> {
        // Persisted IDs stay stable so existing user selections remain valid.
        let values = match self {
            Self::Classic => return None,
            // Monochrome editorial UI with amber reserved for warnings.
            Self::Slate => [
                0x18191b, 0xc9ced6, 0x9daebb, 0x33363b, 0xe2ded2, 0x18191b, 0x89919b, 0xa9c8b3,
                0xdec08b, 0xe4a8a0, 0xf8f4eb,
            ],
            // Forest surfaces, parchment text and a brass navigation rail.
            Self::Moss => [
                0x18271f, 0xc6d6b0, 0xb6b990, 0x304339, 0xd8c28e, 0x20261c, 0x819889, 0xafd3b5,
                0xe4b68c, 0xe6a79c, 0xf4d59a,
            ],
            // A fully light workspace: ink blue navigation, brown warnings.
            Self::Sand => [
                0xf0e9d9, 0x273d50, 0x565966, 0xd9d2c3, 0x284f70, 0xfffcf4, 0x827663, 0x34634c,
                0x764b12, 0x953d39, 0x643d28,
            ],
            // Deep blue canvas, lilac navigation and sea-glass success states.
            Self::Plum => [
                0x161f32, 0xb9cbed, 0xacaecd, 0x2e3b54, 0xd1b9e7, 0x1c2132, 0x8597b6, 0x94d1c7,
                0xe4c58f, 0xf0abb7, 0xeac5ed,
            ],
            // Match CCSW Pulse while keeping muted and error text readable on selection.
            Self::Pulse => [
                0x141e2a, 0xdfe9f0, 0x8ba1b5, 0x253747, 0x7bbeda, 0x141e2a, 0x304354, 0x93ccb2,
                0xeac17e, 0xec8b83, 0xf1f5f7,
            ],
        };
        let mut palette = Palette::new(values);
        palette.surface = match self {
            Self::Slate => Color::Rgb(31, 33, 37),
            Self::Moss => Color::Rgb(25, 43, 33),
            Self::Sand => Color::Rgb(249, 244, 231),
            Self::Plum => Color::Rgb(29, 40, 63),
            Self::Pulse => Color::Rgb(22, 35, 48),
            Self::Classic => Color::Reset,
        };
        Some(palette)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PulseTheme {
    #[default]
    Pulse,
    Slate,
    Moss,
    Sand,
    Plum,
}

impl PulseTheme {
    pub(super) const ALL: [Self; 5] =
        [Self::Pulse, Self::Slate, Self::Moss, Self::Sand, Self::Plum];

    fn name(self) -> &'static str {
        match self {
            Self::Pulse => "Pulse / blue gray & cyan",
            Self::Slate => "Graphite / quiet workspace",
            Self::Moss => "Tundra / framed console",
            Self::Sand => "Paper / light ledger",
            Self::Plum => "Nightfall / soft panels",
        }
    }

    fn design(self) -> Theme {
        match self {
            Self::Pulse => Theme::Pulse,
            Self::Slate => Theme::Slate,
            Self::Moss => Theme::Moss,
            Self::Sand => Theme::Sand,
            Self::Plum => Theme::Plum,
        }
    }

    fn palette(self) -> Option<Palette> {
        let theme = match self {
            Self::Pulse => return None,
            Self::Slate => Theme::Slate,
            Self::Moss => Theme::Moss,
            Self::Sand => Theme::Sand,
            Self::Plum => Theme::Plum,
        };
        theme.palette()
    }

    pub(super) fn load(paths: &AppPaths) -> Self {
        std::fs::read(paths.state_dir.join("pulse-theme.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(self, paths: &AppPaths) -> Result<()> {
        crate::codex::atomic_write(
            &paths.state_dir.join("pulse-theme.json"),
            &serde_json::to_vec(&self)?,
        )
    }

    pub(super) fn apply(self, buffer: &mut ratatui::buffer::Buffer) {
        let Some(p) = self.palette() else { return };
        for cell in &mut buffer.content {
            if cell.fg == quick::RAIL {
                let symbol = self.design().edge(cell.symbol()).to_owned();
                cell.set_symbol(&symbol);
            }
            if cell.bg == quick::RAIL {
                cell.modifier |= if self == Self::Sand {
                    Modifier::UNDERLINED
                } else {
                    Modifier::BOLD
                };
            }
            cell.fg = match cell.fg {
                quick::INK => p.text,
                quick::SOFT => p.muted,
                quick::BLUE => p.accent,
                quick::GOLD => p.warning,
                quick::RED => p.error,
                quick::GREEN => p.success,
                quick::RAIL => p.border,
                quick::BG => p.background,
                Color::Rgb(236, 104, 113) if self == Self::Sand => p.error,
                Color::Rgb(180, 133, 222) if self == Self::Sand => Color::Rgb(106, 65, 138),
                Color::Rgb(112, 171, 235) if self == Self::Sand => p.accent,
                Color::Rgb(133, 212, 162) if self == Self::Sand => p.success,
                Color::Rgb(244, 164, 101) if self == Self::Sand => Color::Rgb(139, 85, 31),
                Color::Rgb(239, 214, 111) if self == Self::Sand => p.warning,
                Color::Rgb(112, 201, 228) if self == Self::Sand => Color::Rgb(47, 99, 116),
                color => color,
            };
            cell.bg = match cell.bg {
                quick::BG => p.background,
                quick::RAIL => p.selection,
                quick::BLUE => p.accent,
                color => color,
            };
            if cell.fg == p.background && cell.bg == p.accent {
                cell.fg = p.on_accent;
            }
        }
    }
}

struct Palette {
    background: Color,
    surface: Color,
    text: Color,
    muted: Color,
    selection: Color,
    accent: Color,
    on_accent: Color,
    border: Color,
    success: Color,
    warning: Color,
    error: Color,
    heading: Color,
}

impl Palette {
    fn new(values: [u32; 11]) -> Self {
        let [
            background,
            text,
            muted,
            selection,
            accent,
            on_accent,
            border,
            success,
            warning,
            error,
            heading,
        ] = values.map(|v| Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        Self {
            surface: background,
            background,
            text,
            muted,
            selection,
            accent,
            on_accent,
            border,
            success,
            warning,
            error,
            heading,
        }
    }

    fn color(&self, color: Color) -> Color {
        match color {
            ROUTE => self.accent,
            SELECTION => self.selection,
            CONNECTED => self.success,
            WARNING => self.warning,
            ERROR => self.error,
            MUTED => self.muted,
            Color::White => self.text,
            Color::DarkGray => self.border,
            _ => color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("expected explicit RGB")
        };
        [r, g, b]
            .into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(v, weight)| {
                let v = f64::from(v) / 255.0;
                weight
                    * if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }

    #[test]
    fn complete_palettes_have_readable_text_and_expected_backgrounds() {
        for theme in [
            Theme::Slate,
            Theme::Moss,
            Theme::Sand,
            Theme::Plum,
            Theme::Pulse,
        ] {
            let p = theme.palette().unwrap();
            let backgrounds = [p.background, p.surface, p.selection];
            for bg in backgrounds {
                for fg in [
                    p.text, p.heading, p.muted, p.accent, p.success, p.warning, p.error,
                ] {
                    let a = luminance(fg);
                    let b = luminance(bg);
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{theme:?}: {fg:?} on {bg:?} = {contrast}");
                }
            }
            let a = luminance(p.accent);
            let b = luminance(p.on_accent);
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 3, 1));
            buffer[(1, 0)].set_fg(Color::Black).set_bg(ROUTE);
            buffer[(2, 0)].set_style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );
            theme.apply(&mut buffer);
            assert_eq!(buffer[(0, 0)].bg, p.background);
            assert_eq!(buffer[(0, 0)].fg, p.text);
            assert_eq!(buffer[(1, 0)].bg, p.accent);
            assert_eq!(buffer[(1, 0)].fg, p.on_accent);
            assert_eq!(buffer[(2, 0)].fg, p.heading);
            assert_ne!(p.heading, p.text);
        }
    }

    #[test]
    fn graphite_uses_consistent_canvas_in_main_and_pulse_views() {
        let mut main = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
        main[(1, 0)].set_bg(SELECTION);
        Theme::Slate.apply(&mut main);
        assert_eq!(main[(0, 0)].bg, Theme::Slate.palette().unwrap().background);
        assert_eq!(main[(1, 0)].bg, Theme::Slate.palette().unwrap().selection);

        let mut pulse = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
        pulse[(0, 0)].set_fg(quick::INK).set_bg(quick::BG);
        pulse[(1, 0)].set_fg(quick::BG).set_bg(quick::BLUE);
        PulseTheme::Slate.apply(&mut pulse);
        assert_eq!(pulse[(0, 0)].bg, Theme::Slate.palette().unwrap().background);
        assert_eq!(pulse[(1, 0)].bg, Theme::Slate.palette().unwrap().accent);
        assert_eq!(pulse[(1, 0)].fg, Theme::Slate.palette().unwrap().on_accent);
    }
}

#[derive(Clone)]
pub(super) struct Appearance {
    pub theme: Theme,
    pub pulse_theme: PulseTheme,
    pub pulse_selected: bool,
    pub refresh_selected: bool,
    pub usage_refresh_secs: u64,
    pub original_usage_refresh_secs: u64,
}

impl App {
    pub(super) fn open_appearance(&mut self) {
        self.modal = Some(Modal::Appearance(Appearance {
            theme: self.theme,
            pulse_theme: PulseTheme::load(&self.paths),
            pulse_selected: false,
            refresh_selected: false,
            usage_refresh_secs: self.config.usage_refresh_secs,
            original_usage_refresh_secs: self.config.usage_refresh_secs,
        }));
    }

    pub(super) fn appearance_key(&mut self, form: &mut Appearance, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => return Ok(true),
            KeyCode::Enter | KeyCode::Char('s') => {
                if form.usage_refresh_secs != form.original_usage_refresh_secs {
                    let value = form.usage_refresh_secs;
                    crate::config::try_update(&self.paths.config, |config| {
                        if config.usage_refresh_secs != form.original_usage_refresh_secs {
                            anyhow::bail!(
                                "usage refresh interval changed in another instance; reopen settings"
                            );
                        }
                        config.usage_refresh_secs = value;
                        Ok(())
                    })?;
                    self.config.usage_refresh_secs = value;
                    form.original_usage_refresh_secs = value;
                }
                form.theme.save(&self.paths)?;
                form.pulse_theme.save(&self.paths)?;
                self.theme = form.theme;
                self.status = format!(
                    "Settings saved · Usage refresh every {}s",
                    form.usage_refresh_secs
                );
                self.status_error = false;
                return Ok(true);
            }
            KeyCode::Char('c')
                if key.modifiers.is_empty() && !self.pi_enabled && !self.codex_ui.enabled =>
            {
                if self.grok_enabled {
                    self.open_grok_preferences(Some(form.clone()));
                    return Ok(true);
                }
                self.open_preferences();
                if let Some(Modal::Preferences(preferences)) = self.modal.as_mut() {
                    preferences.return_appearance = Some(form.clone());
                    preferences.return_theme = Some(form.theme);
                    preferences.return_pulse_theme = Some(form.pulse_theme);
                    preferences.return_pulse_selected = form.pulse_selected;
                    return Ok(true);
                }
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('p') => {
                let current = if form.refresh_selected {
                    2
                } else if form.pulse_selected {
                    1
                } else {
                    0
                };
                let next = (current + if key.code == KeyCode::BackTab { 2 } else { 1 }) % 3;
                form.pulse_selected = next == 1;
                form.refresh_selected = next == 2;
            }
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h') => {
                if form.refresh_selected {
                    form.usage_refresh_secs = form.usage_refresh_secs.saturating_sub(1).max(1);
                } else if form.pulse_selected {
                    let index = PulseTheme::ALL
                        .iter()
                        .position(|t| *t == form.pulse_theme)
                        .unwrap_or(0);
                    form.pulse_theme = PulseTheme::ALL
                        [(index + PulseTheme::ALL.len() - 1) % PulseTheme::ALL.len()];
                } else {
                    let index = Theme::ALL
                        .iter()
                        .position(|t| *t == form.theme)
                        .unwrap_or(0);
                    form.theme = Theme::ALL[(index + Theme::ALL.len() - 1) % Theme::ALL.len()];
                }
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l') => {
                if form.refresh_selected {
                    form.usage_refresh_secs = (form.usage_refresh_secs + 1).min(60);
                } else if form.pulse_selected {
                    let index = PulseTheme::ALL
                        .iter()
                        .position(|t| *t == form.pulse_theme)
                        .unwrap_or(0);
                    form.pulse_theme = PulseTheme::ALL[(index + 1) % PulseTheme::ALL.len()];
                } else {
                    let index = Theme::ALL
                        .iter()
                        .position(|t| *t == form.theme)
                        .unwrap_or(0);
                    form.theme = Theme::ALL[(index + 1) % Theme::ALL.len()];
                }
            }
            _ => {}
        }
        Ok(false)
    }
}

fn gallery(area: Rect) -> bool {
    area.width >= 64 && area.height >= 20
}

pub(super) fn target_tabs(area: Rect) -> [Rect; 3] {
    let inner = panel_inner(area);
    let widths = [
        inner.width / 3,
        inner.width / 3,
        inner.width - 2 * (inner.width / 3),
    ];
    let mut x = inner.x;
    widths.map(|width| {
        let rect = Rect::new(x, inner.y, width, 1);
        x += width;
        rect
    })
}

pub(super) fn rows(area: Rect, form: &Appearance) -> Vec<(usize, Rect)> {
    let inner = panel_inner(area);
    let count = if form.pulse_selected {
        PulseTheme::ALL.len()
    } else {
        Theme::ALL.len()
    };
    let roomy = gallery(area);
    (0..count)
        .map(|index| {
            let compact = area.height < 14;
            let column = if compact { index % 2 } else { 0 };
            let row = if compact { index / 2 } else { index };
            (
                index,
                Rect::new(
                    inner.x + column as u16 * (inner.width / 2),
                    inner.y + if compact { 1 } else { 2 } + row as u16 * if roomy { 2 } else { 1 },
                    if compact {
                        inner.width / 2
                    } else if roomy {
                        inner.width * 44 / 100
                    } else {
                        inner.width
                    },
                    if roomy { 2 } else { 1 },
                ),
            )
        })
        .collect()
}

pub(super) fn refresh_row(area: Rect) -> Rect {
    let inner = panel_inner(area);
    Rect::new(inner.x, area.bottom().saturating_sub(3), inner.width, 1)
}

pub(super) fn refresh_buttons(area: Rect) -> [Rect; 2] {
    let row = refresh_row(area);
    [
        Rect::new(row.right().saturating_sub(9), row.y, 3, 1),
        Rect::new(row.right().saturating_sub(3), row.y, 3, 1),
    ]
}

pub(super) fn draw(
    frame: &mut ratatui::Frame,
    area: Rect,
    form: &Appearance,
    client_settings: Option<&str>,
) {
    frame.render_widget(panel(" Settings / appearance & usage ", true), area);
    let inner = panel_inner(area);
    for (index, rect) in target_tabs(area).into_iter().enumerate() {
        let selected = match index {
            0 => !form.pulse_selected && !form.refresh_selected,
            1 => form.pulse_selected,
            _ => form.refresh_selected,
        };
        frame.render_widget(
            Paragraph::new(["CCSW UI", "Pulse pane", "Refresh"][index])
                .alignment(Alignment::Center)
                .style(button_style(selected, false, false)),
            rect,
        );
    }
    if area.height >= 14 {
        frame.render_widget(
            Paragraph::new("Tab target · ↑↓ theme / interval · Enter save · Esc cancel")
                .style(Style::default().fg(MUTED)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }
    for (index, rect) in rows(area, form) {
        let (selected, name, description) = if form.pulse_selected {
            let theme = PulseTheme::ALL[index];
            (
                theme == form.pulse_theme,
                theme.name(),
                theme.design().description(),
            )
        } else {
            let theme = Theme::ALL[index];
            (theme == form.theme, theme.name(), theme.description())
        };
        let name = if gallery(area) || area.height < 14 {
            name.split(" / ").next().unwrap_or(name)
        } else {
            name
        };
        let mut lines = vec![Line::styled(
            format!(" {}  {}", if selected { "●" } else { "○" }, name),
            Style::default()
                .fg(if selected { ROUTE } else { Color::White })
                .add_modifier(Modifier::BOLD),
        )];
        if gallery(area) {
            lines.push(Line::styled(
                format!(
                    "    {}",
                    description.split(" · ").nth(1).unwrap_or(description)
                ),
                Style::default().fg(MUTED),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(if selected {
                SELECTION
            } else {
                SURFACE
            })),
            rect,
        );
    }
    let row = refresh_row(area);
    frame.render_widget(Clear, row);
    frame.render_widget(
        Paragraph::new(format!(
            " {}Usage refresh  {}s",
            if form.refresh_selected { "● " } else { "" },
            form.usage_refresh_secs
        ))
        .style(
            Style::default()
                .fg(if form.refresh_selected { ROUTE } else { MUTED })
                .bg(SURFACE),
        ),
        row,
    );
    for (index, rect) in refresh_buttons(area).into_iter().enumerate() {
        frame.render_widget(
            Paragraph::new(if index == 0 { " − " } else { " + " }).style(button_style(
                form.refresh_selected,
                false,
                false,
            )),
            rect,
        );
    }
    if area.height >= 20 {
        frame.render_widget(
            Paragraph::new("Live preview · saves both themes and the 1–60s refresh interval")
                .style(Style::default().fg(MUTED)),
            Rect::new(inner.x, row.y - 2, inner.width, 1),
        );
    }
    let buttons = if let Some(label) = client_settings {
        vec!["Save", label, "Cancel"]
    } else {
        vec!["Save", "Cancel"]
    };
    draw_modal_buttons(frame, area, &buttons);
}

// Render after the surrounding UI has been themed, so the Pulse preview is
// independent of the selected CCSW theme and all previews use real components.
pub(super) fn draw_preview(frame: &mut ratatui::Frame, area: Rect, form: &Appearance) {
    if !gallery(area) {
        return;
    }
    let inner = panel_inner(area);
    let split = inner.width * 44 / 100;
    let preview = Rect::new(
        inner.x + split + 1,
        inner.y + 2,
        inner.width - split - 1,
        12,
    );
    frame.render_widget(Clear, preview);
    frame.render_widget(panel(" Preview / sample usage ", true), preview);
    let body = panel_inner(preview);
    let lines = vec![
        Line::styled(" TODAY / GATEWAY", Style::default().fg(MUTED)),
        Line::styled(
            " 128,400 TOKENS",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(vec![
            Span::styled(" ━━━━━━━━━━━", Style::default().fg(ROUTE)),
            Span::styled("━━━━", Style::default().fg(WARNING)),
        ]),
        Line::raw(" Input 96K    Output 32.4K"),
        Line::raw(""),
        Line::styled(
            " Provider       Calls     Tokens",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            " › Primary        128     98.2K",
            Style::default().fg(ROUTE).bg(SELECTION),
        ),
        Line::raw("   Fallback        42     30.2K"),
        Line::raw(""),
        Line::from(vec![
            Span::styled(" ● Connected", Style::default().fg(CONNECTED)),
            Span::styled("  ! 2 retries", Style::default().fg(WARNING)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), body);
    let design = if form.pulse_selected {
        form.pulse_theme.design()
    } else {
        form.theme
    };
    design.apply_region(frame.buffer_mut(), preview);
    if area.height >= 24 {
        let info = Rect::new(preview.x, preview.bottom() + 1, preview.width, 3);
        frame.render_widget(Clear, info);
        frame.render_widget(
            Paragraph::new(design.description().replace(" · ", "\n"))
                .style(Style::default().fg(MUTED).bg(SURFACE)),
            info,
        );
        form.theme.apply_region(frame.buffer_mut(), info);
    }
}
