use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientTab {
    Claude,
    Codex,
    Pi,
    Grok,
    Usage,
    Settings,
}

impl ClientTab {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Pi,
            Self::Pi => Self::Grok,
            Self::Grok => Self::Usage,
            Self::Usage => Self::Settings,
            Self::Settings => Self::Claude,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
            Self::Grok => "Grok",
            Self::Usage => "Usage",
            Self::Settings => "Settings",
        }
    }
}

pub(super) fn client_tabs(area: Rect) -> [(ClientTab, Rect); 6] {
    let mut x = area.x.saturating_add(1);
    let compact = area.width < 60;
    [
        (ClientTab::Claude, 11),
        (ClientTab::Codex, 9),
        (ClientTab::Pi, 5),
        (ClientTab::Grok, 8),
        (ClientTab::Usage, 8),
        (ClientTab::Settings, 12),
    ]
    .map(|(tab, width)| {
        let width = if compact {
            tab.label().len() as u16
        } else {
            width
        };
        let right = area.right().saturating_sub(1);
        let rect = Rect::new(
            x,
            area.y,
            width.min(right.saturating_sub(x)),
            u16::from(area.height > 0),
        );
        x = x.saturating_add(width + 1);
        (tab, rect)
    })
}
impl App {
    pub(super) fn client_tab(&self) -> ClientTab {
        if matches!(self.modal, Some(Modal::Appearance(_) | Modal::Proxy(_))) {
            return ClientTab::Settings;
        }
        if matches!(self.modal, Some(Modal::Help(_))) && self.help_return.is_some() {
            return ClientTab::Settings;
        }
        if self.usage.active {
            return ClientTab::Usage;
        }
        self.config_tab()
    }
    pub(super) fn config_tab(&self) -> ClientTab {
        if self.grok_enabled {
            ClientTab::Grok
        } else if self.pi_enabled {
            ClientTab::Pi
        } else if self.codex_ui.enabled {
            ClientTab::Codex
        } else {
            ClientTab::Claude
        }
    }
    pub(super) fn select_client_tab(&mut self, tab: ClientTab) {
        if (self.modal.is_some()
            && !matches!(self.modal, Some(Modal::Appearance(_) | Modal::Proxy(_))))
            || self.codex_navigation_blocked()
            || self.grok_auth.busy
            || tab == self.client_tab()
        {
            return;
        }
        if matches!(self.modal, Some(Modal::Appearance(_) | Modal::Proxy(_))) {
            self.modal = None;
        }
        if tab == ClientTab::Settings {
            self.open_appearance();
            return;
        }
        if tab == ClientTab::Usage {
            self.open_usage();
            return;
        }
        if self.usage.active && tab == self.config_tab() {
            self.usage.active = false;
            return;
        }
        if self.background.sync_running
            || self.background.proxy_running
            || self.background.queued_sync.is_some()
        {
            self.set_error("Wait for the current client operation to finish");
            return;
        }
        let client = match tab {
            ClientTab::Claude => config::Client::Claude,
            ClientTab::Codex => config::Client::Codex,
            ClientTab::Pi => config::Client::Pi,
            ClientTab::Grok => config::Client::Grok,
            ClientTab::Settings | ClientTab::Usage => unreachable!(),
        };
        let mut config = match if tab == ClientTab::Pi {
            crate::pi::native::load(&self.pi_home)
        } else {
            config::load_client(&self.paths.config, client)
        } {
            Ok(c) => c,
            Err(e) => {
                self.set_error(format!("Could not load client configuration: {e}"));
                return;
            }
        };
        if tab == ClientTab::Pi {
            config.usage_refresh_secs = self.config.usage_refresh_secs;
        }
        self.config = config;
        self.usage.active = false;
        self.cache = discovery::load_cache(&self.client_cache_path(client));
        self.background = Background::default();
        self.proxy_status = None;
        self.provider_editor = None;
        self.profile_idx = 0;
        self.model_idx = 0;
        self.profile_offset = 0;
        self.model_offset = 0;
        self.all_models_filter = Default::default();
        self.home_all_selected = false;
        self.codex_ui.home_models = tab == ClientTab::Codex && !self.config.profiles.is_empty();
        self.pi_enabled = tab == ClientTab::Pi;
        self.grok_enabled = tab == ClientTab::Grok;
        self.grok_auth.home_selected = false;
        self.grok_auth.page = None;
        if self.grok_enabled {
            self.load_grok_auth_status();
        }
        self.codex_ui.enabled = tab == ClientTab::Codex;
        self.return_home();
        self.initialize_background();
        self.status = match tab {
            ClientTab::Claude => "Claude · p sync · F2 next tab",
            ClientTab::Codex => "Codex · Account / API providers · p use · ? help",
            ClientTab::Pi => "Pi · direct API or proxy · p default · P proxy API · i reload",
            ClientTab::Grok => "Grok · o OAuth · i import · p connect · s status · D disconnect",
            ClientTab::Settings | ClientTab::Usage => unreachable!(),
        }
        .into();
        if tab == ClientTab::Pi {
            self.status = crate::pi::native::description(&self.pi_home, &self.config);
        }
    }
    pub(super) fn draw_client_tabs(&self, frame: &mut ratatui::Frame, area: Rect) {
        for (tab, rect) in client_tabs(area) {
            let selected = self.client_tab() == tab;
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(ROUTE)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White).bg(SELECTION)
            };
            let label = if tab == ClientTab::Grok && rect.width < 8 {
                "Grok"
            } else {
                tab.label()
            };
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                rect,
            );
        }
    }
}

impl App {
    pub(super) fn config_client(&self) -> config::Client {
        match self.config_tab() {
            ClientTab::Claude => config::Client::Claude,
            ClientTab::Codex => config::Client::Codex,
            ClientTab::Pi => config::Client::Pi,
            ClientTab::Grok => config::Client::Grok,
            ClientTab::Settings | ClientTab::Usage => unreachable!(),
        }
    }
    pub(super) fn client_cache_path(&self, client: config::Client) -> std::path::PathBuf {
        match client {
            config::Client::Claude => self.paths.cache.clone(),
            config::Client::Codex => self.paths.cache.with_file_name("codex-models.json"),
            config::Client::Pi => self.paths.cache.with_file_name("pi-models.json"),
            config::Client::Grok => self.paths.cache.with_file_name("grok-models.json"),
        }
    }
}
