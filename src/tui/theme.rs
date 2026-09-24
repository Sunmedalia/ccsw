use super::*;

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

    fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Slate => "Graphite / terminal bg & ivory",
            Self::Moss => "Tundra / pine & brass",
            Self::Sand => "Paper / parchment & blue ink",
            Self::Plum => "Nightfall / navy & lilac",
            Self::Pulse => "Pulse / blue gray & cyan",
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

    // Translate semantic UI colors at the frame boundary so all pages and
    // dialogs share the palette without mutable process-global theme state.
    pub(super) fn apply(self, buffer: &mut ratatui::buffer::Buffer) {
        let Some(p) = self.palette() else { return };
        for cell in &mut buffer.content {
            // Graphite keeps the terminal's default background; other themes
            // paint their own background.
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
            cell.bg = if cell.bg == Color::Reset {
                p.background
            } else {
                p.color(cell.bg)
            };
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
        if self == Self::Slate {
            palette.background = Color::Reset;
        }
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
            Self::Slate => "Graphite / terminal bg & ivory",
            Self::Moss => "Tundra / pine & brass",
            Self::Sand => "Paper / parchment & blue ink",
            Self::Plum => "Nightfall / navy & lilac",
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
            let backgrounds = if theme == Theme::Slate {
                assert_eq!(p.background, Color::Reset);
                vec![p.selection]
            } else {
                vec![p.background, p.selection]
            };
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
    fn graphite_uses_terminal_background_in_main_and_pulse_views() {
        let mut main = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
        main[(1, 0)].set_bg(SELECTION);
        Theme::Slate.apply(&mut main);
        assert_eq!(main[(0, 0)].bg, Color::Reset);
        assert_eq!(main[(1, 0)].bg, Theme::Slate.palette().unwrap().selection);

        let mut pulse = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
        pulse[(0, 0)].set_fg(quick::INK).set_bg(quick::BG);
        pulse[(1, 0)].set_fg(quick::BG).set_bg(quick::BLUE);
        PulseTheme::Slate.apply(&mut pulse);
        assert_eq!(pulse[(0, 0)].bg, Color::Reset);
        assert_eq!(pulse[(1, 0)].bg, Theme::Slate.palette().unwrap().accent);
        assert_eq!(pulse[(1, 0)].fg, Theme::Slate.palette().unwrap().on_accent);
    }
}

#[derive(Clone)]
pub(super) struct Appearance {
    pub theme: Theme,
    pub pulse_theme: PulseTheme,
    pub pulse_selected: bool,
}

impl App {
    pub(super) fn open_appearance(&mut self) {
        self.modal = Some(Modal::Appearance(Appearance {
            theme: self.theme,
            pulse_theme: PulseTheme::load(&self.paths),
            pulse_selected: false,
        }));
    }

    pub(super) fn appearance_key(&mut self, form: &mut Appearance, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => return Ok(true),
            KeyCode::Enter | KeyCode::Char('s') => {
                form.theme.save(&self.paths)?;
                form.pulse_theme.save(&self.paths)?;
                self.theme = form.theme;
                self.status = "CCSW and Pulse themes saved".into();
                self.status_error = false;
                return Ok(true);
            }
            KeyCode::Char('c')
                if key.modifiers.is_empty() && !self.pi_enabled && !self.codex_ui.enabled =>
            {
                self.open_preferences();
                if let Some(Modal::Preferences(preferences)) = self.modal.as_mut() {
                    preferences.return_theme = Some(form.theme);
                    preferences.return_pulse_theme = Some(form.pulse_theme);
                    preferences.return_pulse_selected = form.pulse_selected;
                    return Ok(true);
                }
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('p') => {
                form.pulse_selected = !form.pulse_selected;
            }
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h') => {
                if form.pulse_selected {
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
                if form.pulse_selected {
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

pub(super) fn rows(area: Rect, form: &Appearance) -> Vec<(usize, Rect)> {
    let inner = panel_inner(area);
    let count = if form.pulse_selected {
        PulseTheme::ALL.len()
    } else {
        Theme::ALL.len()
    };
    (0..count)
        .map(|index| {
            (
                index,
                Rect::new(
                    inner.x,
                    inner.y + if area.height <= 10 { 1 } else { 2 } + index as u16,
                    inner.width,
                    1,
                ),
            )
        })
        .collect()
}

pub(super) fn draw(frame: &mut ratatui::Frame, area: Rect, form: &Appearance, claude: bool) {
    frame.render_widget(panel(" Settings · TUI appearance ", true), area);
    let inner = panel_inner(area);
    frame.render_widget(
        Paragraph::new(if form.pulse_selected {
            "CCSW UI    [Pulse pane] · Tab"
        } else {
            "[CCSW UI]    Pulse pane · Tab"
        })
        .style(Style::default().fg(ROUTE).add_modifier(Modifier::BOLD)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if area.height > 10 {
        frame.render_widget(
            Paragraph::new("Tab: switch target · ↑↓: select · Enter: save")
                .style(Style::default().fg(MUTED)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }
    for (index, rect) in rows(area, form) {
        let (selected, name) = if form.pulse_selected {
            let theme = PulseTheme::ALL[index];
            (theme == form.pulse_theme, theme.name())
        } else {
            let theme = Theme::ALL[index];
            (theme == form.theme, theme.name())
        };
        frame.render_widget(
            Paragraph::new(format!(" {} {}", if selected { "●" } else { "○" }, name)).style(
                if selected {
                    Style::default().fg(ROUTE).bg(SELECTION)
                } else {
                    Style::default().fg(MUTED)
                },
            ),
            rect,
        );
    }
    frame.render_widget(
        Paragraph::new("Pulse pane updates after saving. Esc cancels the preview.")
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(MUTED)),
        Rect::new(
            inner.x,
            inner.y + 9,
            inner.width,
            inner.height.saturating_sub(11),
        ),
    );
    draw_modal_buttons(
        frame,
        area,
        if claude {
            &["Save", "Claude settings (c)", "Cancel"]
        } else {
            &["Save", "Cancel"]
        },
    );
}
