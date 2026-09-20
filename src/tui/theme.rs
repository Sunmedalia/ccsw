use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Theme {
    #[default]
    Classic,
    Slate,
    Moss,
    Sand,
    Plum,
}

impl Theme {
    const ALL: [Self; 5] = [
        Self::Classic,
        Self::Slate,
        Self::Moss,
        Self::Sand,
        Self::Plum,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Slate => "Graphite / charcoal & ivory",
            Self::Moss => "Tundra / pine & brass",
            Self::Sand => "Paper / parchment & blue ink",
            Self::Plum => "Nightfall / navy & lilac",
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
            // Explicit foreground/background defaults make the light Paper theme
            // and dark themes independent of the terminal's own color scheme.
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
        };
        Some(Palette::new(values))
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
    fn complete_palettes_have_readable_text_and_explicit_backgrounds() {
        for theme in [Theme::Slate, Theme::Moss, Theme::Sand, Theme::Plum] {
            let p = theme.palette().unwrap();
            for bg in [p.background, p.selection] {
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
}

#[derive(Clone)]
pub(super) struct Appearance {
    pub theme: Theme,
}

impl App {
    pub(super) fn open_appearance(&mut self) {
        self.modal = Some(Modal::Appearance(Appearance { theme: self.theme }));
    }

    pub(super) fn appearance_key(&mut self, form: &mut Appearance, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => return Ok(true),
            KeyCode::Enter | KeyCode::Char('s') => {
                form.theme.save(&self.paths)?;
                self.theme = form.theme;
                self.status = format!("TUI theme saved: {}", self.theme.name());
                self.status_error = false;
                return Ok(true);
            }
            KeyCode::Char('c')
                if key.modifiers.is_empty() && !self.pi_enabled && !self.codex_ui.enabled =>
            {
                self.open_preferences();
                if let Some(Modal::Preferences(preferences)) = self.modal.as_mut() {
                    preferences.return_theme = Some(form.theme);
                    return Ok(true);
                }
            }
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h') => {
                let index = Theme::ALL
                    .iter()
                    .position(|t| *t == form.theme)
                    .unwrap_or(0);
                form.theme = Theme::ALL[(index + Theme::ALL.len() - 1) % Theme::ALL.len()];
            }
            KeyCode::Down
            | KeyCode::Right
            | KeyCode::Char('j')
            | KeyCode::Char('l')
            | KeyCode::Tab => {
                let index = Theme::ALL
                    .iter()
                    .position(|t| *t == form.theme)
                    .unwrap_or(0);
                form.theme = Theme::ALL[(index + 1) % Theme::ALL.len()];
            }
            _ => {}
        }
        Ok(false)
    }
}

pub(super) fn rows(area: Rect) -> Vec<(Theme, Rect)> {
    let inner = panel_inner(area);
    Theme::ALL
        .into_iter()
        .enumerate()
        .map(|(index, theme)| {
            (
                theme,
                Rect::new(inner.x, inner.y + 1 + index as u16, inner.width, 1),
            )
        })
        .collect()
}

pub(super) fn draw(frame: &mut ratatui::Frame, area: Rect, form: &Appearance, claude: bool) {
    frame.render_widget(panel(" Settings · TUI appearance ", true), area);
    let inner = panel_inner(area);
    frame.render_widget(
        Paragraph::new("Theme · arrows / j/k preview · Enter save"),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    for (theme, rect) in rows(area) {
        frame.render_widget(
            Paragraph::new(format!(
                " {} {}",
                if theme == form.theme { "●" } else { "○" },
                theme.name()
            ))
            .style(if theme == form.theme {
                Style::default().fg(ROUTE).bg(SELECTION)
            } else {
                Style::default().fg(MUTED)
            }),
            rect,
        );
    }
    frame.render_widget(Paragraph::new("Complete palettes: background, text, selection and status.\nSaved for all clients. Esc cancels the preview.")
        .wrap(Wrap { trim: false }).style(Style::default().fg(MUTED)),
        Rect::new(inner.x, inner.y + 7, inner.width, inner.height.saturating_sub(9)));
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
