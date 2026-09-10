use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientTab {
    Claude,
    Codex,
    Pi,
}

pub(super) fn client_tabs(area: Rect) -> [(ClientTab, Rect); 3] {
    let mut x = area.x.saturating_add(1);
    [
        (ClientTab::Claude, 15),
        (ClientTab::Codex, 9),
        (ClientTab::Pi, 6),
    ]
    .map(|(tab, width)| {
        let rect = Rect::new(
            x,
            area.y,
            width.min(area.right().saturating_sub(x)),
            u16::from(area.height > 0),
        );
        x = x.saturating_add(width + 1);
        (tab, rect)
    })
}
impl App {
    pub(super) fn client_tab(&self) -> ClientTab {
        if self.pi_enabled {
            ClientTab::Pi
        } else if self.codex_ui.enabled {
            ClientTab::Codex
        } else {
            ClientTab::Claude
        }
    }
    pub(super) fn select_client_tab(&mut self, tab: ClientTab) {
        if self.modal.is_some() || self.codex_navigation_blocked() || tab == self.client_tab() {
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
        };
        let config = match config::load_client(&self.paths.config, client) {
            Ok(c) => c,
            Err(e) => {
                self.set_error(format!("Could not load client configuration: {e}"));
                return;
            }
        };
        self.config = config;
        self.cache = discovery::load_cache(&self.client_cache_path(client));
        self.background = Background::default();
        self.proxy_status = None;
        self.provider_editor = None;
        self.profile_idx = 0;
        self.model_idx = 0;
        self.profile_offset = 0;
        self.model_offset = 0;
        self.home_all_selected = false;
        self.pi_enabled = tab == ClientTab::Pi;
        self.codex_ui.enabled = tab == ClientTab::Codex;
        self.return_home();
        self.initialize_background();
        self.status = match tab {
            ClientTab::Claude => "Claude Code · p sync · F2 next tab",
            ClientTab::Codex => "Codex · F3 API / Accounts · F2 next tab",
            ClientTab::Pi => "Pi · direct API · i import · p sync · s status · D disconnect",
        }
        .into();
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
            let label = match tab {
                ClientTab::Claude => "Claude Code",
                ClientTab::Codex => "Codex",
                ClientTab::Pi => "Pi",
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
        match self.client_tab() {
            ClientTab::Claude => config::Client::Claude,
            ClientTab::Codex => config::Client::Codex,
            ClientTab::Pi => config::Client::Pi,
        }
    }
    pub(super) fn client_cache_path(&self, client: config::Client) -> std::path::PathBuf {
        match client {
            config::Client::Claude => self.paths.cache.clone(),
            config::Client::Codex => self.paths.cache.with_file_name("codex-models.json"),
            config::Client::Pi => self.paths.cache.with_file_name("pi-models.json"),
        }
    }
}
