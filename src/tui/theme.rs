use super::*;

// Semantic paint tokens. Themes recolor rendered cells without changing their
// text or hit targets.
pub(super) const SURFACE: Color = Color::Rgb(1, 2, 3);
pub(super) const EDGE: Color = Color::Rgb(1, 2, 4);
pub(super) const ACTIVE_EDGE: Color = Color::Rgb(1, 2, 5);
pub(super) const PROVIDER_SELECTION: Color = Color::Rgb(1, 2, 10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Theme {
    #[default]
    Classic,
    Slate,
    Moss,
    Sand,
    Plum,
    Pulse,
    Arctic,
    Ember,
    Orchid,
}

impl Theme {
    pub(super) const ALL: [Self; 9] = [
        Self::Classic,
        Self::Slate,
        Self::Moss,
        Self::Sand,
        Self::Plum,
        Self::Pulse,
        Self::Arctic,
        Self::Ember,
        Self::Orchid,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic / terminal",
            Self::Slate => "Graphite / quiet workspace",
            Self::Moss => "Tundra / framed console",
            Self::Sand => "Paper / light ledger",
            Self::Plum => "Nightfall / soft panels",
            Self::Pulse => "Pulse / instrument panel",
            Self::Arctic => "Arctic / terminal ice blue",
            Self::Ember => "Ember / terminal warm copper",
            Self::Orchid => "Orchid / terminal violet",
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
            Self::Slate => "Graphite canvas · ice blue models / mauve status · ivory headings",
            Self::Moss => "Forest panels · sage models / copper status · parchment headings",
            Self::Sand => "Warm paper · ink blue models / umber status · underlined focus",
            Self::Plum => "Midnight canvas · lilac models / sea glass status · rose headings",
            Self::Pulse => "Blue instruments · cyan models / gold status · crisp readouts",
            Self::Arctic => "Terminal background · ice blue / lilac / mint · arrow focus",
            Self::Ember => "Terminal background · copper / sky blue / honey · arrow focus",
            Self::Orchid => "Terminal background · violet / sea glass / rose · arrow focus",
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
            Self::Classic | Self::Arctic | Self::Ember | Self::Orchid => {
                ["┌", "┐", "└", "┘", "─", "│"]
            }
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

    pub(super) fn selection_symbol(self) -> &'static str {
        if self.terminal_background() {
            "▶"
        } else {
            ""
        }
    }

    pub(super) fn terminal_background(self) -> bool {
        matches!(self, Self::Arctic | Self::Ember | Self::Orchid)
    }

    fn apply_region(self, buffer: &mut ratatui::buffer::Buffer, region: Rect) {
        let palette = self.palette();
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                let cell = &mut buffer[(x, y)];
                // Cursor arrows belong only to the terminal-background themes.
                // Filled highlights remain the selection cue in the original themes.
                if cell.symbol() == "▶"
                    && !self.terminal_background()
                    && (cell.bg == PROVIDER_SELECTION
                        || (cell.fg == ROUTE && cell.modifier.contains(Modifier::BOLD)))
                {
                    let marker = if cell.bg == PROVIDER_SELECTION {
                        " "
                    } else {
                        "●"
                    };
                    cell.set_symbol(marker);
                }
                if self.terminal_background() && cell.bg == PROVIDER_SELECTION {
                    cell.modifier.remove(Modifier::UNDERLINED);
                }
                if cell.symbol() == "▶" && cell.bg == PROVIDER_SELECTION {
                    cell.fg = ROUTE;
                    cell.modifier |= Modifier::BOLD;
                }
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
                let accent_fill = cell.bg == ROUTE;
                if self.terminal_background() {
                    cell.modifier.remove(Modifier::REVERSED);
                    if selected || accent_fill {
                        cell.modifier |= Modifier::UNDERLINED;
                    }
                    if accent_fill {
                        cell.modifier |= Modifier::BOLD;
                    }
                }
                if selected {
                    cell.modifier |= match self {
                        Self::Sand => Modifier::UNDERLINED,
                        _ => Modifier::empty(),
                    };
                }
                let Some(p) = &palette else {
                    cell.fg = match cell.fg {
                        DEFAULT_MODEL | DEFAULT_LABEL | DATA_SECONDARY => WARNING,
                        ENABLED => CONNECTED,
                        FIELD_LABEL => MUTED,
                        color => color,
                    };
                    if cell.bg == SURFACE {
                        cell.bg = Color::Reset;
                    } else if cell.bg == PROVIDER_SELECTION {
                        cell.bg = SELECTION;
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
                    if self.terminal_background() {
                        p.accent
                    } else {
                        p.on_accent
                    }
                } else {
                    p.color(cell.fg)
                };
                cell.bg = if self.terminal_background() {
                    Color::Reset
                } else if cell.bg == SURFACE {
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
        if self.terminal_background() {
            let values = match self {
                Self::Arctic => [
                    0, 0xc6d9e5, 0x8babbf, 0, 0x70cbea, 0, 0x548ba5, 0x9ed9b5, 0xf0c889, 0xf2949b,
                    0xd9f4ff,
                ],
                Self::Ember => [
                    0, 0xe1d0c2, 0xb99a84, 0, 0xedac7d, 0, 0xac7956, 0xbad5a0, 0xf2cf86, 0xf39891,
                    0xffe4c2,
                ],
                Self::Orchid => [
                    0, 0xd5cbe4, 0xa69abb, 0, 0xc4a2ed, 0, 0x8c70ac, 0x9fd6c5, 0xe7c78e, 0xf199b9,
                    0xf0ddff,
                ],
                _ => unreachable!(),
            };
            let mut p = Palette::new(values);
            p.background = Color::Reset;
            p.surface = Color::Reset;
            p.selection = Color::Reset;
            p.on_accent = p.accent;
            p.default_model = match self {
                Self::Arctic => rgb(0xc5b1ef),
                Self::Ember => rgb(0xa6cced),
                Self::Orchid => rgb(0xa5dacf),
                _ => unreachable!(),
            };
            p.label = match self {
                Self::Arctic => rgb(0xe6bc98),
                Self::Ember => rgb(0xb9d1b4),
                Self::Orchid => rgb(0xa5c9e6),
                _ => unreachable!(),
            };
            p.enabled = match self {
                Self::Arctic => rgb(0xa8d9ce),
                Self::Ember => rgb(0xe6cc98),
                Self::Orchid => rgb(0xe2afcf),
                _ => unreachable!(),
            };
            return Some(p);
        }
        // Persisted IDs stay stable so existing user selections remain valid.
        let values = match self {
            Self::Classic => return None,
            // Graphite typography with separate blue model and mauve status roles.
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
            Self::Arctic | Self::Ember | Self::Orchid => unreachable!(),
        };
        let mut palette = Palette::new(values);
        (palette.default_model, palette.enabled) = match self {
            Self::Slate => (rgb(0xa9c8d9), rgb(0xd4b5cf)),
            Self::Moss => (rgb(0xc6d6b0), rgb(0xe4b68c)),
            Self::Sand => (rgb(0x284f70), rgb(0x764b12)),
            Self::Plum => (rgb(0xd1b9e7), rgb(0x94d1c7)),
            Self::Pulse => (rgb(0x7bbeda), rgb(0xeac17e)),
            Self::Classic | Self::Arctic | Self::Ember | Self::Orchid => unreachable!(),
        };
        palette.surface = match self {
            Self::Slate => Color::Rgb(31, 33, 37),
            Self::Moss => Color::Rgb(25, 43, 33),
            Self::Sand => Color::Rgb(249, 244, 231),
            Self::Plum => Color::Rgb(29, 40, 63),
            Self::Pulse => Color::Rgb(22, 35, 48),
            Self::Classic | Self::Arctic | Self::Ember | Self::Orchid => Color::Reset,
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
    Arctic,
    Ember,
    Orchid,
}

impl PulseTheme {
    pub(super) const ALL: [Self; 8] = [
        Self::Pulse,
        Self::Slate,
        Self::Moss,
        Self::Sand,
        Self::Plum,
        Self::Arctic,
        Self::Ember,
        Self::Orchid,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Pulse => "Pulse / blue gray & cyan",
            Self::Slate => "Graphite / quiet workspace",
            Self::Moss => "Tundra / framed console",
            Self::Sand => "Paper / light ledger",
            Self::Plum => "Nightfall / soft panels",
            Self::Arctic => "Arctic / terminal ice blue",
            Self::Ember => "Ember / terminal warm copper",
            Self::Orchid => "Orchid / terminal violet",
        }
    }

    fn design(self) -> Theme {
        match self {
            Self::Pulse => Theme::Pulse,
            Self::Slate => Theme::Slate,
            Self::Moss => Theme::Moss,
            Self::Sand => Theme::Sand,
            Self::Plum => Theme::Plum,
            Self::Arctic => Theme::Arctic,
            Self::Ember => Theme::Ember,
            Self::Orchid => Theme::Orchid,
        }
    }

    fn palette(self) -> Palette {
        self.design()
            .palette()
            .expect("Pulse themes have a palette")
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
        self.apply_region(buffer, buffer.area);
    }

    fn apply_region(self, buffer: &mut ratatui::buffer::Buffer, region: Rect) {
        let p = self.palette();
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                let cell = &mut buffer[(x, y)];
                let accent_text = cell.fg == quick::BG && cell.bg == quick::BLUE;
                let selected = matches!(cell.bg, quick::BLUE | quick::RAIL);
                if cell.fg == quick::RAIL {
                    let symbol = self.design().edge(cell.symbol()).to_owned();
                    cell.set_symbol(&symbol);
                }
                if cell.bg == quick::RAIL {
                    if self == Self::Sand {
                        cell.modifier |= Modifier::UNDERLINED;
                    }
                }
                cell.fg = match cell.fg {
                    quick::INK if cell.modifier.contains(Modifier::BOLD) => p.heading,
                    quick::INK => p.text,
                    quick::SOFT => p.label,
                    quick::BLUE => p.accent,
                    quick::GOLD => p.warning,
                    quick::RED => p.error,
                    quick::GREEN => p.success,
                    quick::METRIC => p.enabled,
                    quick::RAIL => p.border,
                    quick::BG => p.background,
                    Color::Rgb(236, 104, 113) => p.error,
                    Color::Rgb(180, 133, 222) => p.enabled,
                    Color::Rgb(112, 171, 235) => p.accent,
                    Color::Rgb(133, 212, 162) => p.success,
                    Color::Rgb(244, 164, 101) => p.warning,
                    Color::Rgb(239, 214, 111) => p.heading,
                    Color::Rgb(112, 201, 228) => p.default_model,
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
                if self.design().terminal_background() {
                    cell.bg = Color::Reset;
                    cell.modifier.remove(Modifier::REVERSED);
                    if selected {
                        cell.modifier |= Modifier::UNDERLINED;
                    }
                    if accent_text {
                        cell.fg = p.accent;
                        cell.modifier |= Modifier::BOLD;
                    }
                }
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
    default_model: Color,
    enabled: Color,
    label: Color,
}

fn rgb(value: u32) -> Color {
    Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
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
            default_model: accent,
            enabled: success,
            label: muted,
        }
    }

    fn color(&self, color: Color) -> Color {
        match color {
            ROUTE => self.accent,
            SELECTION | PROVIDER_SELECTION => self.selection,
            CONNECTED => self.success,
            WARNING => self.warning,
            ERROR => self.error,
            MUTED => self.muted,
            DEFAULT_MODEL => self.default_model,
            DEFAULT_LABEL | FIELD_LABEL => self.label,
            DATA_SECONDARY => self.enabled,
            ENABLED => self.enabled,
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
                    p.text,
                    p.heading,
                    p.muted,
                    p.accent,
                    p.success,
                    p.warning,
                    p.error,
                    p.default_model,
                    p.enabled,
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

    #[test]
    fn classic_restores_original_default_and_enabled_colors() {
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 3, 1));
        for (x, color) in [DEFAULT_LABEL, DEFAULT_MODEL, ENABLED]
            .into_iter()
            .enumerate()
        {
            buffer[(x as u16, 0)].set_fg(color).set_bg(SURFACE);
        }
        Theme::Classic.apply(&mut buffer);
        assert_eq!(buffer[(0, 0)].fg, WARNING);
        assert_eq!(buffer[(1, 0)].fg, WARNING);
        assert_eq!(buffer[(2, 0)].fg, CONNECTED);
        assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
    }

    #[test]
    fn terminal_themes_never_paint_backgrounds_or_reverse_text() {
        assert_eq!(Theme::default(), Theme::Classic);
        for theme in [Theme::Arctic, Theme::Ember, Theme::Orchid] {
            let palette = theme.palette().unwrap();
            assert_ne!(palette.label, palette.muted);
            assert_ne!(palette.default_model, palette.accent);
            assert_ne!(palette.default_model, palette.enabled);
            let (_temp, mut app) = super::super::tests::persisted_app();
            app.theme = theme;
            for size in [(40, 12), (80, 24), (120, 36)] {
                for settings in [false, true] {
                    app.modal = None;
                    if settings {
                        app.open_appearance();
                    }
                    let mut terminal =
                        Terminal::new(ratatui::backend::TestBackend::new(size.0, size.1)).unwrap();
                    terminal.draw(|frame| app.draw(frame)).unwrap();
                    if !settings {
                        let area =
                            ui_areas(Rect::new(0, 0, size.0, size.1), app.focus, ViewMode::Home)
                                .profiles
                                .unwrap();
                        let buffer = terminal.backend().buffer();
                        for y in area.y..area.bottom() {
                            for x in area.x..area.right() {
                                assert!(
                                    !buffer[(x, y)].modifier.contains(Modifier::UNDERLINED),
                                    "{theme:?}: provider card underline at {x},{y}"
                                );
                            }
                        }
                        assert!(
                            buffer.content.iter().any(|cell| cell.symbol() == "▶"
                                && cell.modifier.contains(Modifier::BOLD))
                        );
                    }
                    assert!(
                        terminal
                            .backend()
                            .buffer()
                            .content
                            .iter()
                            .all(|cell| cell.bg == Color::Reset
                                && !cell.modifier.contains(Modifier::REVERSED)),
                        "{theme:?} {size:?} settings={settings}"
                    );
                }
            }
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
            buffer[(0, 0)]
                .set_symbol("X")
                .set_fg(Color::Black)
                .set_bg(ROUTE);
            buffer[(1, 0)]
                .set_symbol("X")
                .set_fg(DEFAULT_MODEL)
                .set_bg(SELECTION);
            theme.apply(&mut buffer);
            assert_eq!(buffer[(0, 0)].fg, theme.palette().unwrap().accent);
            assert!(buffer.content.iter().all(
                |cell| cell.bg == Color::Reset && cell.modifier.contains(Modifier::UNDERLINED)
            ));
        }
        for theme in [PulseTheme::Arctic, PulseTheme::Ember, PulseTheme::Orchid] {
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
            buffer[(0, 0)]
                .set_symbol("X")
                .set_fg(quick::BG)
                .set_bg(quick::BLUE);
            buffer[(1, 0)]
                .set_symbol("X")
                .set_fg(quick::INK)
                .set_bg(quick::RAIL);
            theme.apply(&mut buffer);
            assert!(buffer.content.iter().all(
                |cell| cell.bg == Color::Reset && cell.modifier.contains(Modifier::UNDERLINED)
            ));
            assert_eq!(buffer[(0, 0)].fg, theme.palette().accent);
        }
    }

    #[test]
    fn home_text_roles_survive_selection_wrapping_and_disabled_providers() {
        use ratatui::backend::TestBackend;
        let (_temp, mut app) = super::super::tests::persisted_app();
        let profile: Profile = toml::from_str(
            "name='command_goat'\nbase_url='https://api.example/v1'\napi_format='openai-chat'\ndefault_model='deepseek/deepseek-v4.1-flash[1m]'\nenabled_models=['another-model']",
        ).unwrap();
        app.config.profiles = BTreeMap::from([("command_goat".into(), profile)]);
        app.view_mode = ViewMode::Home;
        app.profile_idx = 0;
        for theme in Theme::ALL {
            app.theme = theme;
            let default_color = theme.palette().map_or(WARNING, |p| p.default_model);
            let enabled_color = theme.palette().map_or(CONNECTED, |p| p.enabled);
            for (width, height) in [(40, 12), (80, 24), (120, 36)] {
                for selected in [false, true] {
                    app.home_all_selected = !selected;
                    app.profile_offset = 0;
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal.draw(|frame| app.draw(frame)).unwrap();
                    let cells = &terminal.backend().buffer().content;
                    if width >= 80 || selected {
                        assert!(
                            cells
                                .iter()
                                .any(|c| c.fg == default_color && c.symbol() == "d"),
                            "{theme:?} {width} selected={selected}"
                        );
                    }
                    assert!(
                        cells
                            .iter()
                            .any(|c| c.fg == enabled_color && c.symbol() == "2"),
                        "{theme:?} {width} selected={selected}"
                    );
                    assert!(
                        cells
                            .iter()
                            .all(|c| !matches!(c.fg, DEFAULT_MODEL | ENABLED | EDGE | ACTIVE_EDGE))
                    );
                    if selected && let Ok(directory) = std::env::var("CCSW_UI_PREVIEW_DIR") {
                        std::fs::create_dir_all(&directory).unwrap();
                        let cells: Vec<_> = cells.iter().map(|c| serde_json::json!({"text": c.symbol(), "fg": format!("{:?}", c.fg), "bg": format!("{:?}", c.bg), "bold": c.modifier.contains(Modifier::BOLD), "underline": c.modifier.contains(Modifier::UNDERLINED)})).collect();
                        std::fs::write(std::path::Path::new(&directory).join(format!("{theme:?}-{width}-home.json")), serde_json::to_vec(&serde_json::json!({"width": width, "height": height, "cells": cells})).unwrap()).unwrap();
                    }
                }
            }
            app.config.profiles.get_mut("command_goat").unwrap().enabled = false;
            let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let disabled_color = theme.palette().map_or(MUTED, |p| p.muted);
            let mut found_disabled = false;
            for y in 0..36 {
                let text: String = (0..120).map(|x| buffer[(x, y)].symbol()).collect();
                if let Some(x) = text.find("provider disabled") {
                    found_disabled = true;
                    let x = text[..x].chars().count();
                    assert_eq!(buffer[(x as u16, y)].fg, disabled_color);
                    assert!(!buffer[(x as u16, y)].modifier.contains(Modifier::BOLD));
                }
            }
            assert!(found_disabled, "{theme:?}: missing disabled provider");
            app.config.profiles.get_mut("command_goat").unwrap().enabled = true;
        }
    }

    #[test]
    fn pulse_readouts_and_headings_follow_every_palette() {
        for theme in PulseTheme::ALL {
            let p = theme.palette();
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 5, 1));
            for (x, color) in [
                quick::INK,
                quick::SOFT,
                quick::METRIC,
                quick::GREEN,
                quick::GOLD,
            ]
            .into_iter()
            .enumerate()
            {
                buffer[(x as u16, 0)]
                    .set_symbol("X")
                    .set_fg(color)
                    .set_bg(quick::BG);
            }
            buffer[(0, 0)].modifier = Modifier::BOLD;
            theme.apply(&mut buffer);
            for (x, color) in [p.heading, p.label, p.enabled, p.success, p.warning]
                .into_iter()
                .enumerate()
            {
                assert_eq!(buffer[(x as u16, 0)].fg, color, "{theme:?}");
                assert_eq!(buffer[(x as u16, 0)].bg, p.background);
            }
            // The preview must use the Pulse palette even over a light main UI.
            let (_temp, mut app) = super::super::tests::persisted_app();
            app.theme = Theme::Sand;
            app.open_appearance();
            if let Some(Modal::Appearance(form)) = &mut app.modal {
                form.pulse_selected = true;
                form.pulse_theme = theme;
            }
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|c| c.fg == p.enabled && c.bg == p.background && c.symbol() == "4"),
                "{theme:?}"
            );
            assert!(buffer.content.iter().all(|c| c.fg != quick::METRIC));
            if let Ok(directory) = std::env::var("CCSW_UI_PREVIEW_DIR") {
                std::fs::create_dir_all(&directory).unwrap();
                let cells: Vec<_> = buffer.content.iter().map(|c| serde_json::json!({"text": c.symbol(), "fg": format!("{:?}", c.fg), "bg": format!("{:?}", c.bg), "bold": c.modifier.contains(Modifier::BOLD), "underline": c.modifier.contains(Modifier::UNDERLINED)})).collect();
                std::fs::write(
                    std::path::Path::new(&directory).join(format!("Pulse-{theme:?}-preview.json")),
                    serde_json::to_vec(
                        &serde_json::json!({"width": 120, "height": 36, "cells": cells}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
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
        self.usage.active = false;
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
    let compact = area.height < 14;
    let columns = if compact { 2 } else { 1 };
    let start = inner.y + if compact { 1 } else { 2 };
    let end = refresh_row(area)
        .y
        .saturating_sub(if area.height >= 20 { 2 } else { 0 });
    let available = end.saturating_sub(start);
    let row_height = if gallery(area) && available as usize >= count * 2 {
        2
    } else {
        1
    };
    let visible_rows = usize::from(available / row_height);
    if visible_rows == 0 {
        return vec![];
    }
    let selected = if form.pulse_selected {
        PulseTheme::ALL
            .iter()
            .position(|theme| *theme == form.pulse_theme)
            .unwrap_or(0)
    } else {
        Theme::ALL
            .iter()
            .position(|theme| *theme == form.theme)
            .unwrap_or(0)
    };
    let offset = (selected / columns).saturating_sub(visible_rows - 1);
    (0..count)
        .filter_map(|index| {
            let row = index / columns;
            if row < offset || row >= offset + visible_rows {
                return None;
            }
            let column = index % columns;
            Some((
                index,
                Rect::new(
                    inner.x + column as u16 * (inner.width / 2),
                    start + (row - offset) as u16 * row_height,
                    if compact {
                        inner.width / 2
                    } else if gallery(area) {
                        inner.width * 44 / 100
                    } else {
                        inner.width
                    },
                    row_height,
                ),
            ))
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
    frame.render_widget(panel(" Settings ", true), area);
    frame.render_widget(
        Paragraph::new(if area.width < 90 {
            "UI"
        } else {
            "Appearance [F4]"
        })
        .alignment(Alignment::Center)
        .style(button_style(true, false, false)),
        settings_appearance_button(area),
    );
    frame.render_widget(
        Paragraph::new(if area.width < 90 {
            "Proxy"
        } else {
            "Proxy [P]"
        })
        .alignment(Alignment::Center)
        .style(button_style(false, false, false)),
        settings_proxy_button(area),
    );
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
            format!(" {}  {}", if selected { "▶" } else { " " }, name),
            Style::default()
                .fg(if selected { ROUTE } else { Color::White })
                .add_modifier(Modifier::BOLD),
        )];
        if rect.height > 1 {
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
                PROVIDER_SELECTION
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
    let body = panel_inner(preview);
    let design = if form.pulse_selected {
        form.pulse_theme.design()
    } else {
        form.theme
    };
    if form.pulse_selected {
        frame.render_widget(
            Block::default()
                .title(Span::styled(
                    " Pulse / sample usage ",
                    Style::default()
                        .fg(quick::BLUE)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(quick::RAIL))
                .style(Style::default().fg(quick::INK).bg(quick::BG)),
            preview,
        );
        frame.render_widget(Paragraph::new(quick::preview_lines(body.width)), body);
        form.pulse_theme.apply_region(frame.buffer_mut(), preview);
    } else {
        frame.render_widget(panel(" Preview / providers & status ", true), preview);
        let profile = Profile {
            name: "command_goat".into(),
            enabled: true,
            base_url: "https://api.example/v1".into(),
            models_url: None,
            api_format: ApiFormat::OpenaiChat,
            credential: Credential::None,
            default_model: "deepseek/deepseek-v4.1-flash[1m]".into(),
            aliases: RoleModels::default(),
            subagent_model: None,
            fallback_models: vec![],
            enabled_models: vec![],
            disabled_models: vec![],
            models: vec![],
        };
        let mut lines = home_profile_lines("command_goat", &profile, 2, body.width, false);
        if let Some(endpoint) = lines
            .iter()
            .position(|line| line.to_string().trim_start().starts_with("Endpoint:"))
        {
            lines.truncate(endpoint);
        }
        lines.extend([
            Line::raw(""),
            Line::styled(
                " ◆ Default model / selected",
                Style::default()
                    .fg(DEFAULT_MODEL)
                    .bg(SELECTION)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(vec![
                Span::styled(" ● Connected", Style::default().fg(CONNECTED)),
                Span::styled("  ! 2 retries", Style::default().fg(WARNING)),
            ]),
            Line::styled(" × Failed request", Style::default().fg(ERROR)),
            Line::from(vec![
                Span::styled(" ○ Disabled  ", Style::default().fg(MUTED)),
                Span::styled(
                    "Enter",
                    Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" open", Style::default().fg(MUTED)),
            ]),
        ]);
        frame.render_widget(Paragraph::new(lines), body);
        design.apply_region(frame.buffer_mut(), preview);
    }
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
