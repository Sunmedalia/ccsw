use super::*;
mod chart;
mod view;
pub(super) fn page_area(screen: Rect) -> Rect {
    Rect::new(
        screen.x,
        screen.y.saturating_add(1),
        screen.width,
        screen.height.saturating_sub(1),
    )
}
use crate::usage::{Query, Reader, Snapshot, Totals};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

struct UsageRead {
    reader: Reader,
    query: Query,
    result: Result<Option<Snapshot>, String>,
}

#[derive(Default)]
pub(super) struct UsageUi {
    pub active: bool,
    pub page: Option<UsagePage>,
    pub snapshot: Snapshot,
    pub updated: Option<Instant>,
    receiver: Option<mpsc::Receiver<UsageRead>>,
    reader: Option<Reader>,
    query: Option<Query>,
    error: Option<String>,
}

#[derive(Clone)]
pub(super) struct UsagePage {
    day: String,
    follow_today: bool,
    range: usize,
    chart_tokens: bool,
    client: usize,
    provider: Option<String>,
    scroll: u16,
    section: usize,
    limit: std::cell::Cell<u16>,
    offset: std::cell::Cell<usize>,
    clicked: Option<(usize, usize)>,
    table_area: std::cell::Cell<Rect>,
}
impl UsagePage {
    fn range_label(&self) -> &'static str {
        ["1 day", "1 week", "1 month", "All time"][self.range]
    }
    fn start_day(&self) -> Option<String> {
        let days = match self.range {
            0 => 0,
            1 => 6,
            2 => 29,
            _ => return None,
        };
        chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d")
            .ok()?
            .checked_sub_signed(chrono::Duration::days(days))
            .map(|d| d.to_string())
    }
    fn includes(&self, day: &str) -> bool {
        self.range == 3
            || (day <= self.day.as_str()
                && self.start_day().is_some_and(|start| day >= start.as_str()))
    }
    fn range_total(
        &self,
        snapshot: &Snapshot,
        client: Option<&str>,
        provider: Option<&str>,
        kind: &str,
    ) -> Totals {
        let mut total = Totals::default();
        for row in &snapshot.rows {
            if self.includes(&row.day)
                && client.is_none_or(|c| c == row.client)
                && provider.is_none_or(|p| p == row.provider)
                && row.kind == kind
            {
                total.add(&row.totals);
            }
        }
        total
    }
    fn client(&self) -> Option<&'static str> {
        [None, Some("Claude"), Some("Codex"), Some("Pi")][self.client]
    }
    pub fn key(&mut self, key: KeyEvent, snapshot: &Snapshot) -> bool {
        self.clicked = None;
        self.scroll = self.scroll.min(self.limit.get());
        match key.code {
            KeyCode::Esc if self.provider.is_some() => {
                self.provider = None;
                self.section = 0;
                self.scroll = 0;
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(6) => return true,
            KeyCode::Left | KeyCode::Right => {
                if self.range == 3 {
                    return false;
                }
                if let Ok(day) = chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d") {
                    let next = if key.code == KeyCode::Left {
                        day.pred_opt()
                    } else {
                        day.succ_opt()
                    };
                    if let Some(next) = next.filter(|d| d.to_string() <= snapshot.today()) {
                        self.day = next.to_string();
                        self.follow_today = self.day == snapshot.today();
                    }
                }
                self.scroll = 0;
            }
            KeyCode::Char('t') => {
                self.day = snapshot.today();
                self.follow_today = true;
                self.scroll = 0;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.client = (self.client + if key.code == KeyCode::Tab { 1 } else { 3 }) % 4;
                self.provider = None;
                self.scroll = 0;
            }
            KeyCode::Char(c @ '1'..='5') => {
                self.section = (c as u8 - b'1') as usize;
                self.scroll = 0;
            }
            KeyCode::Char('d' | 'w' | 'm' | 'y') => {
                self.range = match key.code {
                    KeyCode::Char('d') => 0,
                    KeyCode::Char('w') => 1,
                    KeyCode::Char('m') => 2,
                    _ => 3,
                };
                self.scroll = 0;
            }
            KeyCode::Char('c') => self.chart_tokens = false,
            KeyCode::Char('v') => self.chart_tokens = true,
            KeyCode::Char('a') => {
                self.provider = None;
                self.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1).min(self.limit.get())
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10).min(self.limit.get()),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.limit.get(),
            _ => {}
        }
        false
    }
}

impl App {
    fn usage_query(&self) -> Query {
        if self.usage.active
            && let Some(page) = &self.usage.page
        {
            match page.start_day() {
                Some(start) => Query::Range {
                    start,
                    end: page.day.clone(),
                },
                None => Query::All,
            }
        } else {
            Query::Summary
        }
    }

    fn accept_usage(&mut self, query: Query, result: Result<Option<Snapshot>, String>) -> bool {
        // A date/range switch can happen while the old reader is still working.
        if self.usage.query.as_ref() != Some(&query) {
            return false;
        }
        // A fast A → B → A switch may return "unchanged" for a snapshot
        // cleared during B. Ask a fresh reader rather than displaying zeros.
        if matches!(result, Ok(None)) && self.usage.updated.is_none() {
            self.usage.reader = None;
            return false;
        }
        match result {
            Ok(snapshot) => {
                if let Some(snapshot) = snapshot {
                    self.usage.snapshot = snapshot;
                }
                self.usage.error = None;
            }
            Err(error) => self.usage.error = Some(error),
        }
        self.usage.updated = Some(Instant::now());
        true
    }

    pub(super) fn poll_usage(&mut self) -> bool {
        if let Some(page) = &mut self.usage.page
            && page.follow_today
        {
            page.day = self.usage.snapshot.today();
        }
        let query = self.usage_query();
        let mut changed = self.usage.query.as_ref() != Some(&query);
        if changed {
            self.usage.query = Some(query.clone());
            self.usage.snapshot.rows.clear();
            self.usage.updated = None;
            self.usage.error = None;
        }
        if let Some(receiver) = &self.usage.receiver {
            match receiver.try_recv() {
                Ok(completed) => {
                    self.usage.receiver = None;
                    self.usage.reader = Some(completed.reader);
                    changed |= self.accept_usage(completed.query, completed.result);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.usage.receiver = None;
                    self.usage.updated = Some(Instant::now());
                    self.usage.error = Some("Usage reader stopped".into());
                    changed = true;
                }
                _ => {}
            }
        }
        if self.usage.receiver.is_none()
            && self
                .usage
                .updated
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        {
            let (sender, receiver) = mpsc::channel();
            let path = self.paths.state_dir.join(crate::usage::FILE);
            let config = self.paths.config.clone();
            let mut reader = self
                .usage
                .reader
                .take()
                .unwrap_or_else(|| Reader::new(path, config));
            std::thread::spawn(move || {
                let result = reader.read(&query).map_err(|_| {
                    "Usage database unavailable; check permissions or disk space".into()
                });
                let _ = sender.send(UsageRead {
                    reader,
                    query,
                    result,
                });
            });
            self.usage.receiver = Some(receiver);
        }
        changed
    }

    pub(super) fn open_usage(&mut self) {
        if self.modal.is_some() || self.codex_navigation_blocked() {
            return;
        }
        self.usage.updated = None;
        self.usage.active = true;
        if self.usage.page.is_some() {
            return;
        }
        self.usage.page = Some(UsagePage {
            day: self.usage.snapshot.today(),
            follow_today: true,
            range: 0,
            chart_tokens: false,
            client: 0,
            provider: None,
            scroll: 0,
            section: 0,
            limit: Default::default(),
            offset: Default::default(),
            clicked: None,
            table_area: Default::default(),
        });
    }

    pub(super) fn usage_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('q')
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return true;
        }
        if key.code == KeyCode::F(2) {
            self.select_client_tab(ClientTab::Claude);
            return false;
        }
        if let Some(mut page) = self.usage.page.take() {
            self.usage_enter(&mut page, key);
            if page.key(key, &self.usage.snapshot) {
                self.usage.active = false;
            }
            if key.code == KeyCode::Char('r') {
                self.usage.updated = None;
            }
            self.usage.page = Some(page);
        }
        false
    }

    pub(super) fn provider_usage_label(&self, tokens: bool) -> String {
        if self.pi_enabled {
            return "Not tracked (direct API)".into();
        }
        if self.usage.error.is_some() {
            return "Unavailable · F6 Usage".into();
        }
        if self.usage.updated.is_none() {
            return "Loading · F6 Usage".into();
        }
        let Some(provider) = self.selected_profile_id() else {
            return "F6 Usage".into();
        };
        let client = Some(if self.codex_ui.enabled {
            "Codex"
        } else {
            "Claude"
        });
        let today = self.usage.snapshot.today();
        let daily = self
            .usage
            .snapshot
            .total(client, Some(&provider), Some(&today), "generation");
        let total = self
            .usage
            .snapshot
            .total(client, Some(&provider), None, "generation");
        if tokens {
            format!(
                "{} today / {} total",
                daily.tokens_label(),
                total.tokens_label()
            )
        } else {
            format!("{} today / {} total · F6", daily.calls, total.calls)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_switches_reject_old_results_and_refresh_immediately() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        assert_eq!(app.usage_query(), Query::Summary);
        app.open_usage();
        let daily = app.usage_query();
        assert!(matches!(daily, Query::Range { .. }));
        app.usage.page.as_mut().unwrap().range = 3;
        let all = app.usage_query();
        assert_eq!(all, Query::All);
        app.usage.query = Some(all.clone());
        assert!(!app.accept_usage(daily, Err("stale error".into())));
        assert!(app.usage.error.is_none());
        assert!(app.usage.updated.is_none());
        assert!(app.accept_usage(all, Ok(Some(Snapshot::default()))));
        assert!(app.usage.updated.is_some());
        app.usage.active = false;
        assert_eq!(app.usage_query(), Query::Summary);
        assert!(app.poll_usage());
        assert!(app.usage.updated.is_none());
        assert!(app.usage.receiver.is_some());
    }
    #[test]
    fn unchanged_result_cannot_reuse_a_cleared_snapshot() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        app.usage.query = Some(Query::Summary);
        assert!(!app.accept_usage(Query::Summary, Ok(None)));
        assert!(app.usage.updated.is_none());
        assert!(app.usage.reader.is_none());
    }
}
