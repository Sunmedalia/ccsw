use super::*;
use crate::codex as service;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

enum Update {
    Progress(String),
    Select(String),
    Login(Option<String>, String),
    Done(std::result::Result<String, String>),
}
#[derive(Clone)]
enum Input {
    BrowserLogin,
    DeviceLogin,
    Import,
    ImportFile,
    ImportFileName(std::path::PathBuf),
    Rename(String),
    Reasoning,
    Disconnect,
}
pub(super) struct CodexUi {
    pub enabled: bool,
    pub accounts: bool,
    selected: usize,
    pending_selection: Option<String>,
    chosen_account: Option<String>,
    live_id: Option<String>,
    live_message: String,
    pub busy: bool,
    refreshing: bool,
    pub cancel: Arc<AtomicBool>,
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    input: Option<Input>,
    field: String,
    status_view: bool,
    help_scroll: u16,
    scroll_max: std::cell::Cell<u16>,
    message: String,
}
impl Default for CodexUi {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            enabled: false,
            accounts: false,
            selected: 0,
            pending_selection: None,
            chosen_account: None,
            live_id: None,
            live_message: "s inspect local login".into(),
            busy: false,
            refreshing: false,
            cancel: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
            input: None,
            field: String::new(),
            status_view: false,
            help_scroll: 0,
            scroll_max: std::cell::Cell::new(200),
            message: String::new(),
        }
    }
}
impl App {
    pub(super) fn open_codex_accounts(&mut self) {
        if !self.codex_ui.accounts {
            let _ = self.handle_codex_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        }
    }
    pub(super) fn chatgpt_provider_lines(&self, width: u16) -> Vec<Line<'static>> {
        let selected = matches!(
            self.config.codex.active,
            Some(service::Selection::Account { .. })
        );
        let name = self
            .chosen_codex_account()
            .and_then(|id| self.config.codex.accounts.get(&id))
            .map(|account| account.name.as_str())
            .unwrap_or("Select an account");
        let mut lines = wrap_styled_segments(
            vec![
                (
                    (if selected { " ● " } else { " ○ " }).into(),
                    Style::default().fg(if selected { CONNECTED } else { MUTED }),
                ),
                (
                    "ChatGPT Account".into(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                ("  [ChatGPT]".into(), Style::default().fg(ROUTE)),
            ],
            width,
        );
        for (text, color) in [
            (
                format!(
                    "     Account: {name}   {} saved",
                    self.config.codex.accounts.len()
                ),
                WARNING,
            ),
            ("     Provider: ChatGPT".into(), MUTED),
            ("     Credential: Saved Codex login".into(), MUTED),
        ] {
            lines.extend(wrap_styled_segments(
                vec![(text, Style::default().fg(color))],
                width,
            ));
        }
        lines.push(Line::raw(""));
        lines
    }

    fn apply_codex_account(&mut self) {
        let Some(id) = self.chosen_codex_account() else {
            self.set_error("No saved account selected · Enter Account, import a login, then press Space to select");
            return;
        };
        self.codex_job(move |paths, _, _| {
            service::accounts::activate(&paths, &id)?;
            Ok("Codex account applied · restart CLI / Codex App and open a new chat".into())
        });
    }

    fn codex_job(
        &mut self,
        work: impl FnOnce(AppPaths, Arc<AtomicBool>, Sender<Update>) -> Result<String> + Send + 'static,
    ) {
        if self.codex_ui.busy {
            self.codex_ui.message = "Another Codex operation is running · Esc cancels login".into();
            return;
        }
        self.codex_ui.busy = true;
        self.codex_ui.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.codex_ui.cancel.clone();
        let sender = self.codex_ui.sender.clone();
        let paths = self.paths.clone();
        self.codex_ui.message = "Working…".into();
        self.status = "Codex: working…".into();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(paths, cancel, sender.clone())
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Codex operation failed unexpectedly")))
            .map_err(|error| format!("{error:#}"));
            let (id, message) = service::accounts::live_login()
                .unwrap_or_else(|e| (None, format!("Cannot read local login: {e}")));
            let _ = sender.send(Update::Login(id, message));
            let _ = sender.send(Update::Done(result));
        });
    }
    pub(super) fn poll_codex(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.codex_ui.receiver.try_recv() {
            changed = true;
            match update {
                Update::Progress(message) => {
                    self.codex_ui.message = if message.starts_with("Code: ") {
                        format!("{message}\nStep 2: Open the URL, sign in, and enter the code.")
                    } else {
                        message
                    };
                }
                Update::Select(id) => self.codex_ui.pending_selection = Some(id),
                Update::Login(id, message) => {
                    self.codex_ui.live_id = id;
                    self.codex_ui.live_message = message;
                }
                Update::Done(result) => {
                    let refreshing = std::mem::take(&mut self.codex_ui.refreshing);
                    self.codex_ui.busy = false;
                    self.status_error = result.is_err();
                    self.codex_ui.message = result.unwrap_or_else(|e| e);
                    if !refreshing {
                        self.status = self.codex_ui.message.clone();
                    }
                    if let Ok(config) =
                        config::load_client(&self.paths.config, config::Client::Codex)
                    {
                        if refreshing {
                            self.config.codex.accounts = config.codex.accounts;
                        } else {
                            self.config = config;
                        }
                    }
                    if let Some(id) = self.codex_ui.pending_selection.take() {
                        self.codex_ui.selected = self
                            .config
                            .codex
                            .accounts
                            .keys()
                            .position(|key| key == &id)
                            .unwrap_or(self.codex_ui.selected);
                    }
                    self.codex_ui.selected = self
                        .codex_ui
                        .selected
                        .min(self.config.codex.accounts.len().saturating_sub(1));
                }
            }
        }
        changed
    }
    fn chosen_codex_account(&self) -> Option<String> {
        self.codex_ui
            .chosen_account
            .clone()
            .or_else(|| match &self.config.codex.active {
                Some(service::Selection::Account { id }) => Some(id.clone()),
                _ => None,
            })
            .filter(|id| self.config.codex.accounts.contains_key(id))
    }

    fn selected_codex_account(&self) -> Option<String> {
        if !self.codex_ui.accounts
            && let Some(service::Selection::Account { id }) = &self.config.codex.active
            && self.config.codex.accounts.contains_key(id)
        {
            return Some(id.clone());
        }
        self.config
            .codex
            .accounts
            .keys()
            .nth(self.codex_ui.selected)
            .cloned()
    }
    pub(super) fn apply_codex(&mut self) {
        if self.view_mode == ViewMode::Home
            && (self.home_all_selected || self.config.profiles.is_empty())
        {
            self.apply_codex_account();
            return;
        }
        let Some(profile) = self.selected_profile_id() else {
            self.set_error("Select a provider and model to apply to Codex");
            return;
        };
        let model = self.selected_model().map(|m| canonical_model_id(&m.id));
        self.codex_job(move |paths, _, _| {
            service::apply(&paths, &profile, model.as_deref(), None)?;
            Ok("Codex API applied · restart CLI / ChatGPT App and open a new chat".into())
        });
    }
    fn codex_input(&mut self, input: Input, initial: String) {
        if self.codex_ui.busy {
            return;
        }
        self.codex_ui.input = Some(input);
        self.codex_ui.field = initial;
    }
    pub(super) fn codex_navigation_blocked(&self) -> bool {
        self.codex_ui.busy || self.codex_ui.status_view || self.codex_ui.input.is_some()
    }
    pub(super) fn handle_codex_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        if self.codex_ui.status_view {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    self.codex_ui.status_view = false;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.codex_ui.help_scroll =
                        (self.codex_ui.help_scroll + 1).min(self.codex_ui.scroll_max.get())
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => {
                    self.codex_ui.help_scroll =
                        (self.codex_ui.help_scroll + 5).min(self.codex_ui.scroll_max.get())
                }
                KeyCode::PageUp => {
                    self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(5)
                }
                _ => {}
            }
            return Ok(Some(false));
        }
        if let Some(input) = self.codex_ui.input.clone() {
            match key.code {
                KeyCode::Esc => self.codex_ui.input = None,
                KeyCode::Backspace => {
                    self.codex_ui.field.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.codex_ui.field.clear()
                }
                KeyCode::Char(c) => self.codex_ui.field.push(c),
                KeyCode::Enter => {
                    let mut text = self.codex_ui.field.trim().to_owned();
                    if matches!(input, Input::ImportFile) {
                        if text.is_empty() {
                            return Ok(Some(false));
                        }
                        self.codex_ui.input =
                            Some(Input::ImportFileName(std::path::PathBuf::from(text)));
                        self.codex_ui.field.clear();
                        return Ok(Some(false));
                    }
                    if text.is_empty() && matches!(input, Input::Rename(_)) {
                        return Ok(Some(false));
                    }
                    if text.is_empty()
                        && matches!(
                            input,
                            Input::BrowserLogin
                                | Input::DeviceLogin
                                | Input::Import
                                | Input::ImportFileName(_)
                        )
                    {
                        text = "ChatGPT".into();
                    }
                    self.codex_ui.input = None;
                    self.codex_job(move |paths, cancel, sender| match input {
                        Input::BrowserLogin | Input::DeviceLogin => {
                            let device = matches!(input, Input::DeviceLogin);
                            let id = service::accounts::login(
                                &paths,
                                &text,
                                device,
                                &cancel,
                                |message| {
                                    let _ = sender.send(Update::Progress(message));
                                },
                            )?;
                            let _ = sender.send(Update::Select(id));
                            Ok("Login saved and highlighted · Space select · p apply".into())
                        }
                        Input::Import => {
                            let id = service::accounts::import(&paths, &text, None)?;
                            let _ = sender.send(Update::Select(id));
                            Ok("Account imported and highlighted · Space select · p apply".into())
                        }
                        Input::ImportFile => unreachable!("path handled before starting job"),
                        Input::ImportFileName(path) => {
                            let id = service::accounts::import(&paths, &text, Some(&path))?;
                            let _ = sender.send(Update::Select(id));
                            Ok("Account imported and highlighted · Space select · p apply".into())
                        }
                        Input::Rename(id) => {
                            service::accounts::rename(&paths, &id, &text)?;
                            let _ = sender.send(Update::Select(id));
                            Ok("Account name updated".into())
                        }
                        Input::Reasoning => {
                            service::validate_reasoning(&text)?;
                            config::update(&paths.config, |c| {
                                c.codex.reasoning_effort = Some(text);
                                Ok(())
                            })?;
                            Ok("Reasoning saved · p apply to Codex".into())
                        }
                        Input::Disconnect => {
                            if text != "disconnect" {
                                anyhow::bail!("Disconnect cancelled");
                            }
                            service::disconnect(&paths)?;
                            Ok("Previous Codex settings restored · restart clients".into())
                        }
                    });
                }
                _ => {}
            }
            return Ok(Some(false));
        }
        if !self.codex_ui.enabled {
            return Ok(None);
        }
        if key.code == KeyCode::Enter
            && !self.codex_ui.accounts
            && self.view_mode == ViewMode::Home
            && (self.home_all_selected || self.config.profiles.is_empty())
        {
            self.open_codex_accounts();
            return Ok(Some(false));
        }
        match key.code {
            KeyCode::F(3) => {
                self.codex_ui.accounts = !self.codex_ui.accounts;
                if self.codex_ui.accounts
                    && let Some(service::Selection::Account { id }) = &self.config.codex.active
                {
                    self.codex_ui.selected = self
                        .config
                        .codex
                        .accounts
                        .keys()
                        .position(|v| v == id)
                        .unwrap_or(0);
                }
                if self.codex_ui.accounts {
                    let sender = self.codex_ui.sender.clone();
                    std::thread::spawn(move || {
                        let (id, message) = service::accounts::live_login()
                            .unwrap_or_else(|e| (None, format!("Cannot read local login: {e}")));
                        let _ = sender.send(Update::Login(id, message));
                    });
                }
                return Ok(Some(false));
            }
            KeyCode::Char('?') => {
                self.open_help();
                return Ok(Some(false));
            }
            KeyCode::Char('g') if !self.codex_ui.accounts => {
                self.codex_input(
                    Input::Reasoning,
                    self.config
                        .codex
                        .reasoning_effort
                        .clone()
                        .unwrap_or("medium".into()),
                );
                return Ok(Some(false));
            }
            KeyCode::Char('s') => {
                self.codex_ui.status_view = true;
                self.codex_ui.help_scroll = 0;
                self.codex_job(|paths, _, _| service::status(&paths));
                return Ok(Some(false));
            }
            KeyCode::Char('D') => {
                self.codex_input(Input::Disconnect, String::new());
                return Ok(Some(false));
            }
            _ => {}
        }
        if !self.codex_ui.accounts {
            return Ok(None);
        }
        match key.code {
            KeyCode::Esc if self.codex_ui.busy => {
                self.codex_ui.cancel.store(true, Ordering::Relaxed)
            }
            KeyCode::Esc => self.codex_ui.accounts = false,
            KeyCode::Down | KeyCode::Char('j') => {
                self.codex_ui.selected = (self.codex_ui.selected + 1)
                    .min(self.config.codex.accounts.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.codex_ui.selected = self.codex_ui.selected.saturating_sub(1)
            }
            KeyCode::PageDown => {
                self.codex_ui.help_scroll = self
                    .codex_ui
                    .help_scroll
                    .saturating_add(3)
                    .min(self.codex_ui.scroll_max.get())
            }
            KeyCode::PageUp => {
                self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(3)
            }
            KeyCode::Char('r') if !self.codex_ui.busy => {
                if let Some(id) = self.selected_codex_account() {
                    self.codex_ui.help_scroll = 0;
                    let previous_status = self.status.clone();
                    self.codex_job(move |paths, _, _| {
                        service::accounts::refresh(&paths, &id)?;
                        Ok("Usage refreshed · cached until your next refresh".into())
                    });
                    self.codex_ui.refreshing = true;
                    self.codex_ui.message = "Refreshing usage… cached data shown below".into();
                    self.status = previous_status;
                }
            }
            KeyCode::Char('e') => {
                if let Some(id) = self.selected_codex_account() {
                    let name = self.config.codex.accounts[&id].name.clone();
                    self.codex_input(Input::Rename(id), name);
                }
            }
            KeyCode::Char('b') => self.codex_input(Input::BrowserLogin, String::new()),
            KeyCode::Char('d') => self.codex_input(Input::DeviceLogin, String::new()),
            KeyCode::Char('i') => self.codex_input(Input::Import, String::new()),
            KeyCode::Char('I') => self.codex_input(Input::ImportFile, String::new()),
            KeyCode::Char(' ') => {
                self.codex_ui.chosen_account = self.selected_codex_account();
                self.codex_ui.message = "Account selected · p / Apply Codex to apply".into();
            }
            KeyCode::Char('p') => {
                self.apply_codex_account();
            }
            _ => {}
        }
        Ok(Some(false))
    }
    pub(super) fn codex_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<bool> {
        if !self.codex_ui.enabled || self.modal.is_some() {
            return Ok(false);
        }
        if self.codex_ui.status_view {
            if matches!(mouse.kind, MouseEventKind::ScrollDown) {
                self.codex_ui.help_scroll =
                    (self.codex_ui.help_scroll + 1).min(self.codex_ui.scroll_max.get());
            }
            if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                self.codex_ui.help_scroll = self.codex_ui.help_scroll.saturating_sub(1);
            }
            return Ok(true);
        }
        if self.codex_ui.input.is_some() {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let popup = account_input_area(area);
                if let Some(index) = modal_button_rects(popup, 2)
                    .iter()
                    .position(|rect| contains(*rect, mouse.column, mouse.row))
                {
                    self.handle_codex_key(KeyEvent::new(
                        if index == 0 {
                            KeyCode::Enter
                        } else {
                            KeyCode::Esc
                        },
                        KeyModifiers::NONE,
                    ))?;
                }
            }
            return Ok(true);
        }
        if !self.codex_ui.accounts {
            return Ok(false);
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if mouse.row == area.y + 1 && mouse.column < area.x + 14 {
                self.handle_codex_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
                return Ok(true);
            }
            for (key, _, rect) in account_buttons(area) {
                if mouse.column >= rect.x && mouse.column < rect.right() && mouse.row == rect.y {
                    self.handle_codex_key(KeyEvent::new(
                        if key == '\u{1b}' {
                            KeyCode::Esc
                        } else {
                            KeyCode::Char(key)
                        },
                        KeyModifiers::NONE,
                    ))?;
                    return Ok(true);
                }
            }
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.codex_ui.selected = (self.codex_ui.selected + 1)
                    .min(self.config.codex.accounts.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => {
                self.codex_ui.selected = self.codex_ui.selected.saturating_sub(1)
            }
            MouseEventKind::Down(MouseButton::Left) if mouse.row > area.y + 4 => {
                let rows = Layout::vertical([
                    Constraint::Length(4),
                    Constraint::Percentage(40),
                    Constraint::Min(3),
                    Constraint::Length(3),
                ])
                .split(area);
                let visible = rows[1].height.saturating_sub(2) as usize;
                let offset = self
                    .codex_ui
                    .selected
                    .saturating_sub(visible.saturating_sub(1));
                if mouse.row >= rows[1].bottom().saturating_sub(1) {
                    return Ok(true);
                }
                let index = (mouse.row - rows[1].y - 1) as usize + offset;
                if index < self.config.codex.accounts.len() {
                    self.codex_ui.selected = index;
                }
            }
            _ => {}
        }
        Ok(true)
    }
    pub(super) fn draw_codex_accounts(&self, frame: &mut ratatui::Frame, area: Rect) {
        let login_busy = self.codex_ui.busy && !self.codex_ui.refreshing;
        let rows = Layout::vertical([
            Constraint::Length(if login_busy { 3 } else { 4 }),
            if login_busy {
                Constraint::Length(0)
            } else {
                Constraint::Percentage(40)
            },
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ‹ Back (Esc) ",
                    self.footer_control_style(FooterControl::Back, false, false)
                        .1,
                ),
                Span::styled(
                    "  ChatGPT Account",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])),
            Rect::new(rows[0].x, rows[0].y + 1, rows[0].width, 1),
        );
        self.draw_client_tabs(frame, rows[0]);
        frame.render_widget(
            Paragraph::new(self.codex_ui.live_message.as_str()).wrap(Wrap { trim: false }),
            Rect::new(
                rows[0].x,
                rows[0].y + 2,
                rows[0].width,
                rows[0].height.saturating_sub(2),
            ),
        );
        let items = self
            .config
            .codex
            .accounts
            .iter()
            .map(|(id, account)| {
                let active = self.config.codex.active
                    == Some(service::Selection::Account { id: id.clone() });
                let chosen = self.chosen_codex_account().as_ref() == Some(id);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if chosen { "● " } else { "○ " },
                        Style::default().fg(if chosen { ROUTE } else { MUTED }),
                    ),
                    Span::styled(&account.name, Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {}", account.email), Style::default().fg(MUTED)),
                    Span::styled(
                        if active {
                            "  Applied"
                        } else if self.codex_ui.live_id.as_ref() == Some(id) {
                            "  Local login"
                        } else {
                            ""
                        },
                        Style::default().fg(CONNECTED),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(self.codex_ui.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(" ChatGPT accounts ", true))
                .highlight_style(
                    Style::default()
                        .fg(Color::White)
                        .bg(SELECTION)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(" "),
            rows[1],
            &mut state,
        );
        let details = self.selected_codex_account()
            .and_then(|id| self.config.codex.accounts.get(&id).map(|account| (id, account)))
            .map(|(id, account)| {
                let applied = self.config.codex.active == Some(service::Selection::Account { id: id.clone() });
                let local = self.codex_ui.live_id.as_ref() == Some(&id);
                let state = if account.error.is_some() { "Refresh failed · cached data" } else if account.refreshed_at.is_some() { "Last check succeeded" } else { "Saved · not checked" };
                format!("Status: {state}{}{}\n{}", if applied { " · Applied" } else { "" }, if local { " · Local login" } else { "" }, service::accounts::cached_summary(account))
            })
            .unwrap_or("Add an account: Browser / Device to sign in, or Import / File to reuse a saved login.".into());
        let details = if login_busy {
            format!("Esc / Back cancels login\n{}", self.codex_ui.message)
        } else if self.codex_ui.refreshing {
            format!("Refreshing usage…\n{details}")
        } else if self.codex_ui.message.is_empty() {
            details
        } else {
            format!("{details}\n\n{}", self.codex_ui.message)
        };
        let inner = panel_inner(rows[2]);
        let display = usage_display_lines(&details, inner.width);
        let line_count: usize = display
            .iter()
            .map(|line| {
                line.width()
                    .max(1)
                    .div_ceil(usize::from(inner.width).max(1))
            })
            .sum();
        let max_scroll = line_count
            .saturating_sub(usize::from(inner.height))
            .min(u16::MAX as usize) as u16;
        self.codex_ui.scroll_max.set(max_scroll);
        frame.render_widget(
            Paragraph::new(display)
                .scroll((
                    if login_busy {
                        0
                    } else {
                        self.codex_ui.help_scroll.min(max_scroll)
                    },
                    0,
                ))
                .block(panel(
                    if login_busy {
                        " Login progress · Esc cancel "
                    } else {
                        " Account · r refresh · PgUp/PgDn "
                    },
                    false,
                ))
                .wrap(Wrap { trim: false }),
            rows[2],
        );
        for (key, label, rect) in account_buttons(area) {
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(button_style(
                        false,
                        (self.codex_ui.busy && key != '\u{1b}')
                            || (matches!(key, 'e' | 'p' | 'r')
                                && self.selected_codex_account().is_none()),
                        false,
                    )),
                rect,
            );
        }
        self.draw_codex_overlay(frame, area);
    }
    pub(super) fn draw_codex_overlay(&self, frame: &mut ratatui::Frame, area: Rect) {
        if !self.codex_ui.enabled {
            return;
        }
        let title = self.codex_ui.input.as_ref().map(|input| match input {
            Input::BrowserLogin => " Browser login · Account label ",
            Input::DeviceLogin => " Device login · Account label ",
            Input::Import => "Import current login · Account label",
            Input::ImportFileName(_) => "Import auth.json · Account label",
            Input::Rename(_) => "Edit account · Display name",
            Input::ImportFile => "Import auth.json · File path",
            Input::Reasoning => "Reasoning: none/minimal/low/medium/high/xhigh",
            Input::Disconnect => "Type disconnect to restore previous configuration",
        });
        if let Some(title) = title {
            let popup = account_input_area(area);
            let login = matches!(
                self.codex_ui.input,
                Some(
                    Input::BrowserLogin
                        | Input::DeviceLogin
                        | Input::Import
                        | Input::ImportFileName(_)
                )
            );
            let description = match self.codex_ui.input {
                Some(Input::BrowserLogin) => {
                    "Optional label, e.g. Personal.\nNext: sign in through your browser."
                }
                Some(Input::DeviceLogin) => {
                    "Optional label, e.g. Personal.\nNext: get a code. No password here."
                }
                Some(Input::Import | Input::ImportFileName(_)) => {
                    "Optional label, e.g. Personal.\nSave this login to your account list."
                }
                Some(Input::ImportFile) => {
                    "Full path to an auth.json file.\nNext: choose an account label."
                }
                Some(Input::Rename(_)) => {
                    "Edit the display name only.\nEmail and plan come from your login."
                }
                _ => "Enter a value, then confirm. Ctrl+U clears the field.",
            };
            frame.render_widget(Clear, popup);
            frame.render_widget(panel(title, true), popup);
            let inner = panel_inner(popup);
            frame.render_widget(
                Paragraph::new(description).wrap(Wrap { trim: false }),
                Rect::new(inner.x, inner.y, inner.width, 2),
            );
            frame.render_widget(
                Paragraph::new(
                    if login || matches!(self.codex_ui.input, Some(Input::Rename(_))) {
                        format!(
                            "Name: {}",
                            if self.codex_ui.field.is_empty() {
                                "ChatGPT (default)"
                            } else {
                                &self.codex_ui.field
                            }
                        )
                    } else {
                        format!("> {}", self.codex_ui.field)
                    },
                )
                .style(Style::default().fg(ROUTE).bg(SELECTION)),
                Rect::new(inner.x, inner.y + 2, inner.width, 1),
            );
            draw_modal_buttons(
                frame,
                popup,
                &[
                    match self.codex_ui.input {
                        Some(Input::DeviceLogin) => "Get code",
                        Some(Input::BrowserLogin) => "Open browser",
                        Some(Input::Import | Input::ImportFileName(_)) => "Import (Enter)",
                        Some(Input::ImportFile) => "Next (Enter)",
                        Some(Input::Rename(_)) => "Save (Enter)",
                        _ => "Confirm (Enter)",
                    },
                    "Cancel (Esc)",
                ],
            );
        } else if self.codex_ui.status_view {
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(Clear, popup);
            let text = self.codex_ui.message.as_str();
            let title = " Codex Status · ↑↓ scroll · Esc close ";
            let max_scroll = text
                .lines()
                .map(|line| {
                    use unicode_width::UnicodeWidthStr;
                    line.width()
                        .max(1)
                        .div_ceil(popup.width.saturating_sub(2).max(1) as usize)
                })
                .sum::<usize>()
                .saturating_sub(popup.height.saturating_sub(2) as usize)
                .min(u16::MAX as usize) as u16;
            self.codex_ui.scroll_max.set(max_scroll);
            frame.render_widget(
                Paragraph::new(text)
                    .block(panel(title, true))
                    .wrap(Wrap { trim: false })
                    .scroll((self.codex_ui.help_scroll.min(max_scroll), 0)),
                popup,
            );
        }
    }
}

fn usage_display_lines(details: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in details.lines() {
        if let Some((label, rest)) = line.split_once(": ")
            && let Some((value, reset)) = rest.split_once("% used · ")
            && let Ok(percent) = value.parse::<f64>()
        {
            let cells = usize::from(width.saturating_sub(12).clamp(4, 28));
            let filled = ((percent.clamp(0.0, 100.0) / 100.0) * cells as f64).round() as usize;
            let color = if percent >= 90.0 {
                ERROR
            } else if percent >= 70.0 {
                WARNING
            } else {
                ROUTE
            };
            lines.push(Line::styled(
                label.to_owned(),
                Style::default().fg(ROUTE).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(vec![
                Span::styled("━".repeat(filled), Style::default().fg(color)),
                Span::styled("─".repeat(cells - filled), Style::default().fg(MUTED)),
                Span::styled(format!(" {percent}% used"), Style::default().fg(color)),
            ]));
            lines.push(Line::styled(reset.to_owned(), Style::default().fg(MUTED)));
        } else {
            let color = if line.starts_with("Refreshing") {
                ROUTE
            } else if line.starts_with("Last refresh") || line.starts_with("Workspace") {
                MUTED
            } else {
                Color::Reset
            };
            lines.push(Line::styled(line.to_owned(), Style::default().fg(color)));
        }
    }
    lines
}

fn account_input_area(area: Rect) -> Rect {
    centered_rect(
        area.width.saturating_sub(4).min(72),
        area.height.saturating_sub(2).min(8),
        area,
    )
}

// Shared hit regions and rendering keep mouse actions aligned at every width.
fn account_buttons(area: Rect) -> Vec<(char, &'static str, Rect)> {
    let labels = [
        ('i', "Import (i)"),
        ('I', "File (I)"),
        ('b', "Browser (b)"),
        ('d', "Device (d)"),
        ('e', "Rename (e)"),
        ('r', "Refresh (r)"),
        ('p', "Apply (p)"),
        ('\u{1b}', "Back (Esc)"),
    ];
    let natural: u16 = labels
        .iter()
        .map(|(_, label)| UnicodeWidthStr::width(*label) as u16)
        .sum();
    let roomy = natural + labels.len() as u16 * 2 + (labels.len() as u16 - 1) * 2 <= area.width;
    let padding = if roomy { 2 } else { 0 };
    let gap = if roomy { 2 } else { 1 };
    let mut x = area.x;
    let mut y = area.bottom().saturating_sub(3);
    labels
        .into_iter()
        .map(|(key, label)| {
            let width = UnicodeWidthStr::width(label) as u16 + padding;
            if x + width > area.right() {
                x = area.x;
                y += 1;
            }
            let rect = Rect::new(x, y, width, 1);
            x += width + gap;
            (key, label, rect)
        })
        .collect()
}

#[cfg(test)]
mod login_ui_tests {
    use super::*;

    #[test]
    fn login_buttons_open_correct_prompts_and_progress_can_be_cancelled() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        app.select_client_tab(ClientTab::Codex);
        app.codex_ui.accounts = true;
        for width in [40, 80, 120] {
            let area = Rect::new(0, 0, width, 24);
            let buttons = account_buttons(area);
            assert_eq!(buttons.len(), 8);
            for (key, label, rect) in &buttons {
                assert!(rect.right() <= area.right());
                assert!(usize::from(rect.width) >= UnicodeWidthStr::width(*label));
                if matches!(key, 'b' | 'd') {
                    app.codex_mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: rect.x,
                            row: rect.y,
                            modifiers: KeyModifiers::NONE,
                        },
                        area,
                    )
                    .unwrap();
                    assert!(matches!(
                        (&app.codex_ui.input, key),
                        (Some(Input::BrowserLogin), 'b') | (Some(Input::DeviceLogin), 'd')
                    ));
                    let cancel = modal_button_rects(account_input_area(area), 2)[1];
                    app.codex_mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: cancel.x,
                            row: cancel.y,
                            modifiers: KeyModifiers::NONE,
                        },
                        area,
                    )
                    .unwrap();
                    assert!(app.codex_ui.input.is_none());
                }
            }
        }
        app.codex_ui.busy = true;
        app.codex_ui
            .sender
            .send(Update::Progress(
                "Code: TEST-CODE\nOpen: https://example.invalid".into(),
            ))
            .unwrap();
        app.poll_codex();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| app.draw_codex_accounts(frame, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("TEST-CODE"));
        app.handle_codex_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.codex_ui.cancel.load(Ordering::Relaxed));
        app.codex_ui
            .sender
            .send(Update::Done(Err("Login cancelled".into())))
            .unwrap();
        app.poll_codex();
        assert!(!app.codex_ui.busy);
        assert_eq!(app.codex_ui.message, "Login cancelled");
    }
    #[test]
    fn file_import_asks_for_label_and_rename_preserves_identity() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        let id = "0123456789abcdef0123456789abcdef";
        let account = service::accounts::Account {
            name: "Original".into(),
            email: "person@example.invalid".into(),
            plan: Some("plus".into()),
            ..Default::default()
        };
        config::update(&app.paths.config, |config| {
            config.codex.accounts.insert(id.into(), account.clone());
            Ok(())
        })
        .unwrap();
        app.select_client_tab(ClientTab::Codex);
        app.codex_ui.accounts = true;
        app.handle_codex_key(KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE))
            .unwrap();
        app.codex_ui.field = "/tmp/example-auth.json".into();
        app.handle_codex_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(app.codex_ui.input, Some(Input::ImportFileName(_))));
        assert!(!app.codex_ui.busy);
        app.handle_codex_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        app.handle_codex_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(app.codex_ui.input, Some(Input::Rename(_))));
        assert_eq!(app.codex_ui.field, "Original");
        app.codex_ui.field = "Personal".into();
        app.handle_codex_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        for _ in 0..200 {
            app.poll_codex();
            if !app.codex_ui.busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.codex_ui.busy);
        let saved = &app.config.codex.accounts[id];
        let mut expected = account;
        expected.name = "Personal".into();
        assert_eq!(saved, &expected);
    }
    #[test]
    fn usage_bars_and_refresh_keep_account_list_visible() {
        let lines = usage_display_lines("codex 5h: 75% used · resets in 30 min", 40);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].spans[0].style.fg, Some(WARNING));
        assert!(lines[1].spans[2].content.contains("75% used"));
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        app.select_client_tab(ClientTab::Codex);
        app.codex_ui.accounts = true;
        app.codex_ui.busy = true;
        app.codex_ui.refreshing = true;
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| app.draw_codex_accounts(frame, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("ChatGPT accounts"));
        assert!(text.contains("Refreshing usage"));
        assert!(!text.contains("Login progress"));
        let controls = account_buttons(Rect::new(0, 0, 120, 30));
        assert_eq!(controls[0].2.x, 0);
        let gap = controls[1].2.x - controls[0].2.right();
        for pair in controls.windows(2) {
            assert_eq!(pair[1].2.x - pair[0].2.right(), gap);
        }
    }
}
