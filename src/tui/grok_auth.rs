use super::*;
use crate::grok::{
    auth::{self, Action},
    usage,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

enum Update {
    Progress(String),
    Done(Result<auth::Status, String>),
    Usage(u64, Result<usage::Snapshot, String>),
}
pub(super) struct AuthUi {
    pub busy: bool,
    pub home_selected: bool,
    pub page: Option<AccountPage>,
    cancel: Arc<AtomicBool>,
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    status: auth::Status,
    message: String,
    progress: Vec<String>,
    usage: Option<usage::Snapshot>,
    usage_error: Option<String>,
    usage_refreshing: bool,
    usage_request: u64,
    usage_waking: bool,
}
impl Default for AuthUi {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            busy: false,
            home_selected: false,
            page: None,
            cancel: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
            status: Default::default(),
            message: "Browser or Device code signs in through Grok.".into(),
            progress: vec![],
            usage: None,
            usage_error: None,
            usage_refreshing: false,
            usage_request: 0,
            usage_waking: false,
        }
    }
}
impl Drop for AuthUi {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Clone)]
pub(super) struct AccountPage {
    pub selected: usize,
    pub model: FormField,
    pub confirm_logout: bool,
    pub scroll: u16,
}
const ACTIONS: [&str; 7] = [
    "Browser (b)",
    "Device code (d)",
    "Use OAuth (u)",
    "Refresh (r)",
    "Wake (w)",
    "Sign out (x)",
    "Back (Esc)",
];
pub(super) fn account_actions(screen: Rect) -> Vec<Rect> {
    let natural: u16 = ACTIONS
        .iter()
        .map(|label| UnicodeWidthStr::width(*label) as u16)
        .sum();
    let roomy = natural + ACTIONS.len() as u16 * 2 + (ACTIONS.len() as u16 - 1) * 2 <= screen.width;
    let padding = if roomy { 2 } else { 0 };
    let gap = if roomy { 2 } else { 1 };
    let mut x = screen.x;
    let mut y = screen.bottom().saturating_sub(3);
    ACTIONS
        .iter()
        .map(|label| {
            let width = UnicodeWidthStr::width(*label) as u16 + padding;
            if x + width > screen.right() {
                x = screen.x;
                y += 1;
            }
            let rect = Rect::new(x, y, width, 1);
            x += width + gap;
            rect
        })
        .collect()
}
fn confirmation_area(screen: Rect) -> Rect {
    centered_rect(
        screen.width.saturating_sub(4).min(72),
        screen.height.saturating_sub(2).min(8),
        screen,
    )
}
impl App {
    pub(super) fn home_grok_oauth_selected(&self) -> bool {
        self.grok_enabled
            && self.view_mode == ViewMode::Home
            && self.home_all_selected
            && self.grok_auth.home_selected
    }
    pub(super) fn grok_oauth_provider_lines(&self, width: u16) -> Vec<Line<'static>> {
        let status = &self.grok_auth.status;
        let native_default = self
            .config
            .grok
            .preferences
            .default
            .as_deref()
            .filter(|model| {
                !model.starts_with("ccsw::")
                    && !self.config.grok.imports.values().any(|key| key == model)
            });
        let active = status.saved && native_default.is_some();
        let mut lines = wrap_styled_segments(
            vec![
                (
                    (if active { " ● " } else { " ○ " }).into(),
                    Style::default().fg(if active { CONNECTED } else { MUTED }),
                ),
                (
                    "Grok OAuth Account".into(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                ("  [OAuth]".into(), Style::default().fg(ROUTE)),
            ],
            width,
        );
        for text in [
            format!(
                "     Account: {}",
                status.email.as_deref().unwrap_or(if status.saved {
                    "Saved native login"
                } else {
                    "Not signed in"
                })
            ),
            format!(
                "     {}",
                if active {
                    "Native model selected"
                } else {
                    "Enter to configure"
                }
            ),
            "     Credential: Grok OAuth".into(),
        ] {
            lines.extend(wrap_styled_segments(
                vec![(text, Style::default().fg(MUTED))],
                width,
            ));
        }
        lines.push(Line::raw(""));
        lines
    }
    pub(super) fn grok_auth_status_description(&self) -> String {
        self.grok_auth.status.description()
    }
    pub(super) fn load_grok_auth_status(&mut self) {
        self.grok_auth.status = auth::status(&self.grok_home).unwrap_or_default();
        self.reconcile_grok_usage();
    }
    fn reconcile_grok_usage(&mut self) {
        let account = usage::account(&self.grok_home).ok().flatten();
        if self
            .grok_auth
            .usage
            .as_ref()
            .is_some_and(|snapshot| Some(&snapshot.account) != account.as_ref())
        {
            self.grok_auth.usage = None;
            self.grok_auth.usage_error = None;
        }
        if !self.grok_auth.status.saved {
            self.grok_auth.usage = None;
        }
    }
    fn refresh_grok_usage(&mut self) {
        self.refresh_grok_auth();
        self.reconcile_grok_usage();
        if self.grok_auth.busy || self.grok_auth.usage_refreshing {
            return;
        }
        if !self.grok_auth.status.saved {
            self.grok_auth.usage_error = Some("Sign in to view account usage".into());
            return;
        }
        self.grok_auth.usage_request = self.grok_auth.usage_request.wrapping_add(1);
        self.grok_auth.usage_refreshing = true;
        self.grok_auth.usage_error = None;
        self.spawn_grok_usage();
    }
    fn wake_grok_usage(&mut self) -> Result<()> {
        if self.grok_auth.busy || self.grok_auth.usage_refreshing {
            return Ok(());
        }
        let id = usage::account(&self.grok_home)?
            .ok_or_else(|| anyhow::anyhow!("Sign in before waking this account"))?;
        self.grok_auth.usage_request = self.grok_auth.usage_request.wrapping_add(1);
        self.grok_auth.usage_refreshing = true;
        self.grok_auth.usage_waking = true;
        self.grok_auth.usage_error = None;
        self.spawn_grok_wake(id);
        Ok(())
    }
    #[cfg(not(test))]
    fn spawn_grok_wake(&self, id: String) {
        let home = self.grok_home.clone();
        let sender = self.grok_auth.sender.clone();
        let request = self.grok_auth.usage_request;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                usage::wake(&home, &id)?;
                usage::fetch(&home)
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Grok wake worker stopped")))
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Update::Usage(request, result));
        });
    }
    #[cfg(test)]
    fn spawn_grok_wake(&self, _: String) {}
    #[cfg(not(test))]
    fn spawn_grok_usage(&self) {
        let home = self.grok_home.clone();
        let sender = self.grok_auth.sender.clone();
        let request = self.grok_auth.usage_request;
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| usage::fetch(&home)))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Grok usage worker stopped")))
                    .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Update::Usage(request, result));
        });
    }
    // UI tests inject results through the channel; no fixture credentials go online.
    #[cfg(test)]
    fn spawn_grok_usage(&self) {}
    pub(super) fn open_grok_auth(&mut self) {
        self.refresh_grok_auth();
        let default = self
            .config
            .grok
            .preferences
            .default
            .as_deref()
            .filter(|model| crate::grok::validate_oauth_model(&self.grok_home, model).is_ok())
            .unwrap_or("grok-build");
        self.grok_auth.page = Some(AccountPage {
            selected: 1,
            model: field("Native model", default),
            confirm_logout: false,
            scroll: 0,
        });
        self.refresh_grok_usage();
    }
    fn refresh_grok_auth(&mut self) {
        match auth::status(&self.grok_home) {
            Ok(status) => {
                self.grok_auth.status = status;
                self.grok_auth.message = "Use OAuth selects a native model. Explicit model API keys still take precedence.".into();
            }
            Err(error) => {
                self.grok_auth.status = Default::default();
                self.grok_auth.message = format!("Cannot inspect local login: {error:#}");
            }
        }
    }
    fn start_grok_auth(&mut self, action: Action) {
        if self.grok_auth.busy {
            return;
        }
        self.grok_auth.usage_request = self.grok_auth.usage_request.wrapping_add(1);
        self.grok_auth.usage_refreshing = false;
        self.grok_auth.usage_waking = false;
        self.grok_auth.usage = None;
        self.grok_auth.usage_error = None;
        self.grok_auth.busy = true;
        self.grok_auth.cancel = Arc::new(AtomicBool::new(false));
        self.grok_auth.progress.clear();
        self.grok_auth.message = if action == Action::Logout {
            "Signing out…"
        } else {
            "Waiting for Grok authorization… · Esc cancels"
        }
        .into();
        let cancel = self.grok_auth.cancel.clone();
        let sender = self.grok_auth.sender.clone();
        let home = self.grok_home.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                auth::run(action, &home, &cancel, |text| {
                    let _ = sender.send(Update::Progress(text));
                })
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Grok authorization worker stopped")))
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(Update::Done(result));
        });
    }
    pub(super) fn cancel_grok_auth(&mut self) {
        if self.grok_auth.busy {
            self.grok_auth.cancel.store(true, Ordering::Relaxed);
            self.grok_auth.message = "Cancelling Grok authorization…".into();
        }
    }
    pub(super) fn poll_grok_auth(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.grok_auth.receiver.try_recv() {
            changed = true;
            match update {
                Update::Usage(request, result) => {
                    if request != self.grok_auth.usage_request {
                        continue;
                    }
                    let waking = std::mem::take(&mut self.grok_auth.usage_waking);
                    self.grok_auth.usage_refreshing = false;
                    self.load_grok_auth_status();
                    match result {
                        Ok(snapshot)
                            if usage::account(&self.grok_home).ok().flatten().as_ref()
                                == Some(&snapshot.account) =>
                        {
                            if waking {
                                self.grok_auth.message = "Wake complete · usage refreshed".into();
                            }
                            self.grok_auth.usage = Some(snapshot);
                            self.grok_auth.usage_error = None;
                        }
                        Ok(_) => {
                            self.grok_auth.usage_error = Some(
                                "Account changed during refresh; press r to reload usage".into(),
                            )
                        }
                        Err(error) => self.grok_auth.usage_error = Some(error),
                    }
                }
                Update::Progress(text) => {
                    if !self.grok_auth.progress.contains(&text) {
                        if self.grok_auth.progress.len() == 8 {
                            self.grok_auth.progress.remove(0);
                        }
                        if text.starts_with("Device code:") {
                            self.grok_auth.progress.insert(0, text);
                        } else {
                            self.grok_auth.progress.push(text);
                        }
                    }
                }
                Update::Done(result) => {
                    let succeeded = result.is_ok();
                    self.grok_auth.busy = false;
                    self.grok_auth.progress.clear();
                    match result {
                        Ok(status) => {
                            self.grok_auth.message = if status.saved { "OAuth login saved by Grok · Use OAuth selects the native startup model" } else { "Grok signed out · API provider configurations retained" }.into();
                            self.grok_auth.status = status;
                            self.status_error = false;
                        }
                        Err(error) => {
                            self.status_error = true;
                            self.grok_auth.message = error;
                            self.grok_auth.status =
                                auth::status(&self.grok_home).unwrap_or_default();
                        }
                    }
                    self.status = self.grok_auth.message.clone();
                    self.reconcile_grok_usage();
                    if succeeded && self.grok_auth.status.saved {
                        let message = self.grok_auth.message.clone();
                        self.refresh_grok_usage();
                        self.grok_auth.message = message;
                    }
                }
            }
        }
        changed
    }
    pub(super) fn grok_auth_key(
        &mut self,
        dialog: &mut AccountPage,
        key: KeyEvent,
    ) -> Result<bool> {
        if self.grok_auth.busy {
            if key.code == KeyCode::Esc
                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                self.cancel_grok_auth();
            } else if key.code == KeyCode::PageDown {
                dialog.scroll = dialog.scroll.saturating_add(1);
            } else if key.code == KeyCode::PageUp {
                dialog.scroll = dialog.scroll.saturating_sub(1);
            }
            return Ok(false);
        }
        if dialog.confirm_logout {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    dialog.confirm_logout = false;
                    self.start_grok_auth(Action::Logout);
                }
                KeyCode::Esc | KeyCode::Char('n') => dialog.confirm_logout = false,
                _ => {}
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Esc => return Ok(true),
            KeyCode::Tab | KeyCode::Down => {
                dialog.selected = (dialog.selected + 1) % 8;
                return Ok(false);
            }
            KeyCode::BackTab | KeyCode::Up => {
                dialog.selected = (dialog.selected + 7) % 8;
                return Ok(false);
            }
            KeyCode::PageDown => {
                dialog.scroll = dialog.scroll.saturating_add(1);
                return Ok(false);
            }
            KeyCode::PageUp => {
                dialog.scroll = dialog.scroll.saturating_sub(1);
                return Ok(false);
            }
            _ => {}
        }
        if dialog.selected == 0 {
            if key.code == KeyCode::Enter {
                dialog.selected = 1;
            } else {
                handle_form_key(std::slice::from_mut(&mut dialog.model), &mut 0, key);
            }
            return Ok(false);
        }
        let selected = match key.code {
            KeyCode::Char('b') => 1,
            KeyCode::Char('d') => 2,
            KeyCode::Char('u') => 3,
            KeyCode::Char('r' | 's') => 4,
            KeyCode::Char('w') => 5,
            KeyCode::Char('x') => 6,
            KeyCode::Char('q') => 7,
            KeyCode::Enter => dialog.selected,
            _ => return Ok(false),
        };
        dialog.scroll = 0;
        match selected {
            1 => self.start_grok_auth(Action::Browser),
            2 => self.start_grok_auth(Action::Device),
            3 => {
                let model = dialog.model.value.trim().to_owned();
                crate::grok::validate_oauth_model(&self.grok_home, &model)?;
                self.config = self.update_client_config(|c| {
                    c.grok.preferences.default = Some(model);
                    Ok(())
                })?;
                // Always use the explicit native default, not the selected API provider.
                match crate::grok::conflicts(&self.paths, &self.grok_home) {
                    Ok(conflicts) if !conflicts.is_empty() => {
                        self.modal = Some(Modal::Grok(Box::new(grok::Dialog::Reconnect {
                            conflicts,
                            preferred: self.config.grok.preferences.default.clone(),
                            scroll: 0,
                        })));
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                    _ => {}
                }
                crate::grok::apply(&self.paths, &self.grok_home, &self.config, None, false)?;
                self.grok_auth.message =
                    "Native OAuth model selected · restart Grok to load the startup default".into();
                self.status = self.grok_auth.message.clone();
                self.status_error = false;
            }
            4 => self.refresh_grok_usage(),
            5 => self.wake_grok_usage()?,
            6 => dialog.confirm_logout = true,
            7 => return Ok(true),
            _ => {}
        }
        Ok(false)
    }
    pub(super) fn grok_auth_page_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut page) = self.grok_auth.page.take() else {
            return Ok(());
        };
        let result = self.grok_auth_key(&mut page, key);
        if let Err(error) = &result {
            self.grok_auth.message = format!("{error:#}");
            self.status_error = true;
        }
        if !matches!(&result, Ok(true)) {
            self.grok_auth.page = Some(page);
        }
        result.map(|_| ())
    }
    pub(super) fn grok_auth_page_mouse(&mut self, mouse: MouseEvent, screen: Rect) -> Result<()> {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            return self.grok_auth_page_key(KeyEvent::new(
                if mouse.kind == MouseEventKind::ScrollUp {
                    KeyCode::PageUp
                } else {
                    KeyCode::PageDown
                },
                KeyModifiers::NONE,
            ));
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(());
        }
        if mouse.row == screen.y + 1 && mouse.column < screen.x + 18 {
            return self.grok_auth_page_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        let area = account_page_rows(screen, self.grok_auth.busy)[2];
        let confirm = self
            .grok_auth
            .page
            .as_ref()
            .is_some_and(|a| a.confirm_logout);
        if confirm {
            if let Some(i) = modal_button_rects(confirmation_area(screen), 2)
                .iter()
                .position(|r| contains(*r, mouse.column, mouse.row))
            {
                self.grok_auth_page_key(KeyEvent::new(
                    KeyCode::Char(if i == 0 { 'y' } else { 'n' }),
                    KeyModifiers::NONE,
                ))?;
            }
        } else if let Some(i) = account_actions(screen)
            .iter()
            .position(|r| contains(*r, mouse.column, mouse.row))
        {
            if self.grok_auth.busy {
                if i == 6 {
                    self.cancel_grok_auth();
                }
                return Ok(());
            }
            if let Some(page) = self.grok_auth.page.as_mut() {
                page.selected = i + 1;
            }
            self.grok_auth_page_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
        } else {
            let inner = panel_inner(area);
            if mouse.row == inner.y
                && contains(inner, mouse.column, mouse.row)
                && let Some(page) = self.grok_auth.page.as_mut()
            {
                page.selected = 0;
            }
        }
        Ok(())
    }
    pub(super) fn draw_grok_accounts(&self, frame: &mut ratatui::Frame, screen: Rect) {
        let rows = account_page_rows(screen, self.grok_auth.busy);
        self.draw_client_tabs(frame, rows[0]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ‹ Back (Esc) ",
                    self.footer_control_style(FooterControl::Back, false, false)
                        .1,
                ),
                Span::styled(
                    " Grok OAuth Accounts ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])),
            Rect::new(rows[0].x, rows[0].y + 1, rows[0].width, 1),
        );
        frame.render_widget(
            Paragraph::new(self.grok_auth.status.description()).wrap(Wrap { trim: false }),
            Rect::new(
                rows[0].x,
                rows[0].y + 2,
                rows[0].width,
                rows[0].height.saturating_sub(2),
            ),
        );
        let items = if self.grok_auth.status.saved {
            vec![ListItem::new(Line::from(vec![
                Span::styled("● ", Style::default().fg(ROUTE)),
                Span::styled(
                    "Grok account",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  {}",
                        self.grok_auth
                            .status
                            .email
                            .as_deref()
                            .unwrap_or("Saved native login")
                    ),
                    Style::default().fg(MUTED),
                ),
                Span::styled("  Local login", Style::default().fg(CONNECTED)),
            ]))]
        } else {
            vec![ListItem::new(Line::styled(
                "No saved account · Browser / Device code to sign in",
                Style::default().fg(MUTED),
            ))]
        };
        let mut state =
            ListState::default().with_selected(self.grok_auth.status.saved.then_some(0));
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(" Grok accounts ", true))
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
        let Some(page) = &self.grok_auth.page else {
            return;
        };
        frame.render_widget(
            panel(
                if self.grok_auth.busy {
                    " Login progress · Esc cancel "
                } else {
                    " Account configuration · PgUp/PgDn "
                },
                false,
            ),
            rows[2],
        );
        let inner = panel_inner(rows[2]);
        let show_model = inner.height > 1 || page.selected == 0;
        if show_model {
            draw_fields(
                frame,
                Rect::new(inner.x, inner.y, inner.width, 1),
                std::slice::from_ref(&page.model),
                0,
                page.selected == 0 && !self.grok_auth.busy,
            );
        }
        let mut lines = Vec::new();
        if self.grok_auth.busy {
            lines.extend(self.grok_auth.progress.iter().cloned().map(Line::raw));
            lines.push(Line::raw(self.grok_auth.message.clone()));
        } else {
            if self.grok_auth.usage_refreshing {
                lines.push(Line::styled(
                    if self.grok_auth.usage_waking {
                        "Waking account… · consumes a little quota"
                    } else {
                        "Refreshing usage… cached data remains visible"
                    },
                    Style::default().fg(ROUTE),
                ));
            }
            if let Some(error) = &self.grok_auth.usage_error {
                lines.push(Line::styled(
                    format!(
                        "Usage refresh failed: {error}{}",
                        if self.grok_auth.usage.is_some() {
                            " · showing cached data"
                        } else {
                            ""
                        }
                    ),
                    Style::default().fg(WARNING),
                ));
            }
            if let Some(snapshot) = &self.grok_auth.usage {
                if inner.height <= 2 {
                    lines.extend(
                        snapshot
                            .summary()
                            .lines()
                            .map(|line| Line::raw(line.to_owned())),
                    );
                } else {
                    lines.extend(codex::usage_display_lines(&snapshot.summary(), inner.width));
                }
                lines.push(Line::raw(""));
                lines.push(Line::raw(self.grok_auth.message.clone()));
            } else {
                lines.push(Line::raw(self.grok_auth.message.clone()));
                lines.push(Line::styled(
                    "Account usage not loaded · r refresh",
                    Style::default().fg(MUTED),
                ));
            }
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((page.scroll, 0)),
            Rect::new(
                inner.x,
                inner.y + u16::from(show_model),
                inner.width,
                inner.height.saturating_sub(u16::from(show_model)),
            ),
        );
        for (i, rect) in account_actions(screen).into_iter().enumerate() {
            frame.render_widget(
                Paragraph::new(if self.grok_auth.busy && i == 6 {
                    "Cancel/Esc"
                } else {
                    ACTIONS[i]
                })
                .alignment(Alignment::Center)
                .style(button_style(
                    page.selected == i + 1,
                    (self.grok_auth.busy && i != 6)
                        || (matches!(i, 3 | 4)
                            && (self.grok_auth.usage_refreshing || !self.grok_auth.status.saved)),
                    i == 5,
                )),
                rect,
            );
        }
        if page.confirm_logout {
            let area = confirmation_area(screen);
            frame.render_widget(Clear, area);
            frame.render_widget(panel(" Sign out of Grok? ", true), area);
            let inner = panel_inner(area);
            frame.render_widget(Paragraph::new("Grok will clear its cached login credentials.\nAPI provider configurations are retained.\n\nEnter/y confirms · n/Esc cancels").wrap(Wrap { trim: false }), Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(1)));
            draw_modal_buttons(frame, area, &["Sign out", "Cancel"]);
        }
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn usage_refresh_keeps_account_list_cache_and_ignores_stale_account_results() {
        let (temp, mut app) = super::super::tests::persisted_app();
        app.grok_home = temp.path().join("grok");
        std::fs::create_dir_all(&app.grok_home).unwrap();
        let auth = app.grok_home.join("auth.json");
        std::fs::write(
            &auth,
            r#"{"auth_mode":"oidc","key":"SECRET","user_id":"one","email":"account@example.com"}"#,
        )
        .unwrap();
        app.select_client_tab(ClientTab::Grok);
        app.open_grok_auth();
        assert!(app.grok_auth.usage_refreshing);
        let snapshot = usage::Snapshot {
            account: usage::account(&app.grok_home).unwrap().unwrap(),
            credits: usage::Credits {
                percent: Some(75.0),
                period: Some("Weekly".into()),
                prepaid_cents: Some(1234),
                ..Default::default()
            },
            fetched_at: 1,
        };
        app.grok_auth
            .sender
            .send(Update::Usage(
                app.grok_auth.usage_request,
                Ok(snapshot.clone()),
            ))
            .unwrap();
        assert!(app.poll_grok_auth());
        assert!(!app.grok_auth.usage_refreshing);
        app.grok_auth.page.as_mut().unwrap().selected = 5;
        app.grok_auth_page_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.grok_auth.usage_waking && app.grok_auth.usage_refreshing);
        assert!(app.grok_auth.usage.is_some());
        let wake_request = app.grok_auth.usage_request;
        app.grok_auth_page_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.grok_auth.usage_request, wake_request);
        let mut waking_terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
        waking_terminal.draw(|frame| app.draw(frame)).unwrap();
        let waking_text: String = waking_terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(waking_text.contains("Waking account") && waking_text.contains("75% used"));
        assert!(waking_text.contains("Wake (w)"));
        app.grok_auth
            .sender
            .send(Update::Usage(wake_request, Ok(snapshot.clone())))
            .unwrap();
        app.poll_grok_auth();
        assert!(!app.grok_auth.usage_waking && !app.grok_auth.usage_refreshing);
        assert!(app.grok_auth.message.contains("Wake complete"));
        app.refresh_grok_usage();
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("account@example.com") && text.contains("Grok accounts"));
        assert!(text.contains("Refreshing usage") && text.contains("75% used"));
        assert!(text.contains("━") && text.contains("25.0%") && text.contains("$12.34"));
        assert!(!text.contains("SECRET"));
        let mut small = Terminal::new(TestBackend::new(40, 12)).unwrap();
        app.grok_auth.usage_refreshing = false;
        small.draw(|frame| app.draw(frame)).unwrap();
        let small_text: String = small
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(small_text.contains("75.0% used"));
        app.grok_auth.usage_refreshing = true;

        app.grok_auth
            .sender
            .send(Update::Usage(
                app.grok_auth.usage_request,
                Err("Service unavailable".into()),
            ))
            .unwrap();
        app.poll_grok_auth();
        assert!(app.grok_auth.usage.is_some());
        assert_eq!(
            app.grok_auth.usage_error.as_deref(),
            Some("Service unavailable")
        );
        app.refresh_grok_usage();
        let stale_request = app.grok_auth.usage_request;
        std::fs::write(&auth, r#"{"auth_mode":"oidc","key":"NEW","user_id":"two"}"#).unwrap();
        app.grok_auth
            .sender
            .send(Update::Usage(stale_request, Ok(snapshot)))
            .unwrap();
        app.poll_grok_auth();
        assert!(app.grok_auth.usage.is_none());
        assert!(
            app.grok_auth
                .usage_error
                .as_deref()
                .unwrap()
                .contains("Account changed")
        );
        app.grok_auth.usage_request += 1;
        app.grok_auth
            .sender
            .send(Update::Usage(stale_request, Err("stale error".into())))
            .unwrap();
        app.poll_grok_auth();
        assert!(
            !app.grok_auth
                .usage_error
                .as_deref()
                .unwrap()
                .contains("stale error")
        );
    }
}
