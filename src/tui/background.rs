use super::*;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub(super) struct SyncRequest {
    explicit: bool,
    preferred: Option<String>,
}
enum Completion {
    Inspect(Result<sync::Status>, Option<proxy::ProxyStatus>),
    Discover {
        request: u64,
        id: String,
        profile: Box<Profile>,
        form: Option<uuid::Uuid>,
        result: Result<Vec<ModelEntry>>,
    },
    Sync(
        Result<claude_config::ApplyResult>,
        Result<sync::Status>,
        Option<proxy::ProxyStatus>,
    ),
    Proxy(Box<ProxyManager>),
    Panicked,
}
pub(super) struct Background {
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    requests: BTreeMap<String, u64>,
    sequence: u64,
    pub(super) status: sync::Status,
    pub(super) sync_running: bool,
    pub(super) proxy_running: bool,
    pub(super) queued_sync: Option<SyncRequest>,
    due: Instant,
    connected: bool,
}
impl Default for Background {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            requests: BTreeMap::new(),
            sequence: 0,
            status: sync::Status::NotConnected,
            sync_running: false,
            proxy_running: false,
            queued_sync: None,
            due: Instant::now(),
            connected: false,
        }
    }
}
impl Background {
    fn spawn(&self, work: impl FnOnce() -> Completion + Send + 'static) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or(Completion::Panicked);
            let _ = sender.send(result);
        });
    }
}
impl App {
    pub(super) fn initialize_background(&mut self) {
        if self.client_tab() != ClientTab::Claude {
            return;
        }
        let paths = self.paths.clone();
        self.background.spawn(move || {
            let status =
                claude_config::settings_path().and_then(|path| sync::inspect(&paths, &path));
            Completion::Inspect(status, proxy::status(&paths).ok())
        });
    }

    pub(super) fn start_discovery(&mut self, form: Option<uuid::Uuid>) {
        let Some(id) = self.selected_profile_id() else {
            self.set_error("Select a provider before fetching models");
            return;
        };
        if self.background.requests.contains_key(&id) {
            self.status = "A model request is already running for this provider".into();
            return;
        }
        let profile = self.config.profiles[&id].clone();
        if !profile.enabled {
            self.set_error("Enable this provider before fetching models");
            return;
        }
        self.background.sequence += 1;
        let request = self.background.sequence;
        self.background.requests.insert(id.clone(), request);
        self.status_error = false;
        self.status = format!("Fetching models from {}…", profile.name);
        self.background.spawn(move || {
            let result = discovery::discover(&profile);
            Completion::Discover {
                request,
                id,
                profile: Box::new(profile),
                form,
                result,
            }
        });
    }

    pub(super) fn queue_sync(&mut self, explicit: bool, preferred: Option<String>) {
        if self.client_tab() != ClientTab::Claude {
            return;
        }
        if !explicit && !self.background.connected {
            return;
        }
        if let Some(queued) = &mut self.background.queued_sync {
            if explicit {
                queued.explicit = true;
                queued.preferred = preferred;
            }
        } else {
            self.background.queued_sync = Some(SyncRequest {
                explicit,
                preferred,
            });
        }
        self.background.due =
            Instant::now() + Duration::from_millis(if explicit { 0 } else { 200 });
        self.background.status = sync::Status::Pending;
    }

    pub(super) fn start_proxy_action(&mut self, control: ProxyControl) {
        if self.background.proxy_running || self.background.sync_running {
            self.set_error("A proxy operation is in progress; retry when it finishes");
            return;
        }
        let Some(Modal::Proxy(manager)) = &mut self.modal else {
            return;
        };
        manager.message = "Working…".into();
        manager.error = false;
        let mut manager = manager.clone();
        let paths = self.paths.clone();
        self.background.proxy_running = true;
        self.background.spawn(move || {
            manager.activate(&paths, control);
            Completion::Proxy(Box::new(manager))
        });
    }

    pub(super) fn poll_background(&mut self) -> bool {
        let mut changed = false;
        while let Ok(completion) = self.background.receiver.try_recv() {
            changed = true;
            match completion {
                Completion::Inspect(status, proxy) => {
                    self.proxy_status = proxy;
                    match status {
                        Ok(status) => {
                            self.background.connected =
                                matches!(status, sync::Status::Pending | sync::Status::Synced);
                            if !self.background.sync_running
                                && self.background.queued_sync.is_none()
                            {
                                self.background.status = status;
                                if status == sync::Status::Pending {
                                    self.queue_sync(false, None);
                                }
                            }
                        }
                        Err(error) => {
                            self.background.status = sync::Status::Failed;
                            self.set_error(format!("Could not inspect sync state: {error:#}"));
                        }
                    }
                }
                Completion::Discover {
                    request,
                    id,
                    profile,
                    form,
                    result,
                } => {
                    if self.background.requests.get(&id) != Some(&request) {
                        continue;
                    }
                    self.background.requests.remove(&id);
                    if self.config.profiles.get(&id) != Some(profile.as_ref())
                        || self
                            .load_client_config()
                            .ok()
                            .and_then(|config| config.profiles.get(&id).cloned())
                            .as_ref()
                            != Some(profile.as_ref())
                    {
                        continue;
                    }
                    // A result belongs to the form that requested it, never its replacement.
                    let target_form = form.is_some_and(|instance| matches!(&self.modal, Some(Modal::Model(current)) if current.instance == instance));
                    match result {
                        Ok(models) => {
                            let cached = CachedModels {
                                fetched_at: now_epoch(),
                                models: models.clone(),
                            };
                            let persisted = discovery::update_cache(
                                &self.client_cache_path(self.config_client()),
                                |cache| {
                                    cache.profiles.insert(id.clone(), cached.clone());
                                },
                            );
                            self.cache.profiles.insert(id.clone(), cached);
                            if target_form && let Some(Modal::Model(current)) = &mut self.modal {
                                let selected = current
                                    .filtered_api_models()
                                    .get(current.api_selected)
                                    .map(|model| model.id.clone());
                                current.api_models = models.clone();
                                current.api_selected = selected
                                    .and_then(|id| {
                                        current
                                            .filtered_api_models()
                                            .iter()
                                            .position(|model| model.id == id)
                                    })
                                    .unwrap_or(0);
                                current.api_scroll = current.api_selected;
                                current.api_status = format!("{} models available", models.len());
                            }
                            if self.selected_profile_id().as_deref() == Some(&id) {
                                self.refresh_editor_preserving_selection();
                                self.status_error = false;
                                self.status = format!(
                                    "Connection verified · {} models discovered",
                                    models.len()
                                );
                            }
                            if let Err(error) = persisted {
                                let message = format!(
                                    "Models available, but cache could not be saved: {error:#}"
                                );
                                if target_form && let Some(Modal::Model(current)) = &mut self.modal
                                {
                                    current.api_status = message.clone();
                                }
                                self.set_error(message);
                            }
                        }
                        Err(error) => {
                            let message = format!("Model request failed: {error:#}");
                            if target_form && let Some(Modal::Model(current)) = &mut self.modal {
                                current.api_status = message.clone();
                            }
                            if self.selected_profile_id().as_deref() == Some(&id) {
                                self.set_error(message);
                            }
                        }
                    }
                }
                Completion::Sync(result, status, proxy) => {
                    self.background.sync_running = false;
                    self.proxy_status = proxy;
                    match result {
                        Ok(result) => {
                            self.background.connected = true;
                            self.background.status = status.unwrap_or(sync::Status::Pending);
                            self.status_error = false;
                            self.status = format!("Synced {} models to Claude", result.model_count);
                            if self.background.status == sync::Status::Pending {
                                self.queue_sync(false, None);
                            }
                        }
                        Err(error) => {
                            if matches!(
                                status,
                                Ok(sync::Status::Paused | sync::Status::NotConnected)
                            ) {
                                self.background.connected = false;
                            }
                            self.background.status = if matches!(status, Ok(sync::Status::Paused)) {
                                sync::Status::Paused
                            } else {
                                sync::Status::Failed
                            };
                            self.set_error(format!(
                                "Changes saved · sync failed: {error:#} · press p to retry"
                            ));
                        }
                    }
                }
                Completion::Proxy(manager) => {
                    self.background.proxy_running = false;
                    if manager.port_changed {
                        if self.background.connected {
                            self.background.status = sync::Status::Pending;
                        }
                        self.status_error = false;
                        self.status = manager.message.clone();
                    }
                    self.proxy_status = manager.runtime.clone();
                    if let Some(Modal::Proxy(current)) = &mut self.modal
                        && current.instance == manager.instance
                    {
                        let selected = current.selected;
                        *current = *manager;
                        current.selected = selected;
                    }
                }
                Completion::Panicked => {
                    self.background.sync_running = false;
                    self.background.proxy_running = false;
                    self.background.requests.clear();
                    self.background.status = sync::Status::Failed;
                    self.set_error("Background task failed; changes on disk are preserved. Press p to retry sync.");
                }
            }
        }
        if !self.background.sync_running
            && !self.background.proxy_running
            && Instant::now() >= self.background.due
            && let Some(request) = self.background.queued_sync.take()
        {
            changed = true;
            self.background.sync_running = true;
            self.background.status = sync::Status::Syncing;
            let paths = self.paths.clone();
            self.background.spawn(move || {
                let result = claude_config::settings_path().and_then(|settings| {
                    sync::apply(
                        &paths,
                        &settings,
                        request.preferred.as_deref(),
                        request.explicit,
                    )
                });
                let status = claude_config::settings_path()
                    .and_then(|settings| sync::inspect(&paths, &settings));
                Completion::Sync(result, status, proxy::status(&paths).ok())
            });
        }
        changed
    }

    pub(super) fn refresh_editor_preserving_selection(&mut self) {
        let previous = self.provider_editor.as_ref().map(|editor| {
            (
                editor.query.clone(),
                editor.search_active,
                editor.selected_model().map(|model| model.id.clone()),
            )
        });
        self.init_provider_editor();
        if let (Some((query, search, selected)), Some(editor)) =
            (previous, &mut self.provider_editor)
        {
            editor.query = query;
            editor.search_active = search;
            editor.selected = selected
                .and_then(|id| {
                    editor
                        .filtered_indices()
                        .iter()
                        .position(|index| editor.catalog[*index].id == id)
                })
                .unwrap_or(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::tests::persisted_app;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn slow_discovery_keeps_navigation_responsive() {
        let (_temp, mut app) = persisted_app();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        app.config = config::update(&app.paths.config, |config| {
            config.profiles.get_mut("one").unwrap().base_url = format!("http://{address}");
            Ok(())
        })
        .unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 2048];
            assert!(stream.read(&mut bytes).unwrap() > 0);
            accepted_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let body = r#"{"data":[{"id":"fresh-model"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        app.enter_provider_view();
        app.refresh_models();
        accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.view_mode, ViewMode::Home);
        assert!(app.cache.profiles.is_empty());
        release_tx.send(()).unwrap();
        server.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !app.background.requests.is_empty() && Instant::now() < deadline {
            app.poll_background();
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(app.cache.profiles["one"].models[0].id, "fresh-model");
    }

    #[test]
    fn completion_does_not_fill_a_replacement_form_or_apply_stale_provider_results() {
        let (_temp, mut app) = persisted_app();
        app.enter_provider_view();
        app.open_add_model_modal();
        let Some(Modal::Model(old)) = &app.modal else {
            unreachable!()
        };
        let old_instance = old.instance;
        app.open_add_model_modal();
        let profile = app.config.profiles["one"].clone();
        app.background.requests.insert("one".into(), 1);
        app.background
            .sender
            .send(Completion::Discover {
                request: 1,
                id: "one".into(),
                profile: Box::new(profile.clone()),
                form: Some(old_instance),
                result: Ok(vec![ModelEntry {
                    max_output_tokens: None,
                    context_window: None,
                    id: "stale-form-model".into(),
                    label: None,
                    description: None,
                }]),
            })
            .unwrap();
        app.poll_background();
        assert!(matches!(&app.modal, Some(Modal::Model(form)) if form.api_models.is_empty()));
        config::update(&app.paths.config, |config| {
            config.profiles.get_mut("one").unwrap().base_url = "https://changed.invalid".into();
            Ok(())
        })
        .unwrap();
        app.background.requests.insert("one".into(), 2);
        app.background
            .sender
            .send(Completion::Discover {
                request: 2,
                id: "one".into(),
                profile: Box::new(profile),
                form: None,
                result: Ok(vec![ModelEntry {
                    max_output_tokens: None,
                    context_window: None,
                    id: "wrong-endpoint-model".into(),
                    label: None,
                    description: None,
                }]),
            })
            .unwrap();
        app.poll_background();
        assert_eq!(app.cache.profiles["one"].models[0].id, "stale-form-model");
    }

    #[test]
    fn edits_after_connection_coalesce_and_preserve_explicit_sync_preference() {
        let (_temp, mut app) = persisted_app();
        app.background
            .sender
            .send(Completion::Inspect(Ok(sync::Status::Synced), None))
            .unwrap();
        app.poll_background();
        app.enter_provider_view();
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
                .unwrap();
        }
        assert!(app.background.queued_sync.is_some());
        assert!(!app.background.sync_running);
        assert_eq!(app.background.status, sync::Status::Pending);
        app.queue_sync(true, Some("two".into()));
        app.queue_sync(false, None);
        let request = app.background.queued_sync.as_ref().unwrap();
        assert!(request.explicit);
        assert_eq!(request.preferred.as_deref(), Some("two"));
        assert_eq!(config::load(&app.paths.config).unwrap(), app.config);
    }
}
