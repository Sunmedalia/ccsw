use super::*;

impl App {
    pub(super) fn profile_ids(&self) -> Vec<String> {
        self.config.profiles.keys().cloned().collect()
    }

    pub(super) fn selected_profile_id(&self) -> Option<String> {
        if self.view_mode == ViewMode::AllEnabled
            || (self.view_mode == ViewMode::Home && self.home_all_selected)
        {
            return None;
        }
        self.profile_ids().get(self.profile_idx).cloned()
    }

    pub(super) fn selected_profile(&self) -> Option<&Profile> {
        self.selected_profile_id()
            .and_then(|id| self.config.profiles.get(&id))
    }

    pub(super) fn home_profile_item_heights(&self, panel: Rect) -> Vec<usize> {
        let mut heights = vec![
            all_enabled_lines(
                self.config
                    .profiles
                    .values()
                    .filter(|profile| profile.enabled)
                    .count(),
                self.all_enabled_model_count(),
                self.all_managed_models().len(),
                panel.width.saturating_sub(2),
            )
            .len(),
        ];
        if self.codex_ui.enabled {
            heights[0] = self
                .chatgpt_provider_lines(panel.width.saturating_sub(2))
                .len();
        }
        heights.extend(self.profile_ids().iter().map(|id| {
            let profile = &self.config.profiles[id];
            let discovered = self
                .cache
                .profiles
                .get(id)
                .map(|cached| cached.models.as_slice())
                .unwrap_or_default();
            home_profile_lines(
                id,
                profile,
                discovery::active_models(profile, discovered).len(),
                panel.width.saturating_sub(2),
            )
            .len()
        }));
        heights
    }

    pub(super) fn home_selected_index(&self) -> usize {
        if self.home_all_selected || self.config.profiles.is_empty() {
            0
        } else {
            self.profile_idx.saturating_add(1)
        }
    }

    pub(super) fn select_home_index(&mut self, index: usize) {
        if index == 0 {
            self.home_all_selected = true;
            self.model_idx = 0;
        } else {
            self.home_all_selected = false;
            self.profile_idx = (index - 1).min(self.config.profiles.len().saturating_sub(1));
            self.model_idx = self.default_model_index();
        }
        self.model_offset = 0;
    }

    pub(super) fn models(&self) -> Vec<ModelEntry> {
        let Some(id) = self.selected_profile_id() else {
            return vec![];
        };
        let profile = &self.config.profiles[&id];
        let discovered = self
            .cache
            .profiles
            .get(&id)
            .map(|cached| cached.models.as_slice())
            .unwrap_or_default();
        discovery::active_models(profile, discovered)
    }

    pub(super) fn catalog_models(&self) -> Vec<ModelEntry> {
        let Some(id) = self.selected_profile_id() else {
            return vec![];
        };
        self.catalog_models_for(&id)
    }

    pub(super) fn catalog_models_for(&self, id: &str) -> Vec<ModelEntry> {
        let profile = &self.config.profiles[id];
        let discovered = self
            .cache
            .profiles
            .get(id)
            .map(|cached| cached.models.as_slice())
            .unwrap_or_default();
        discovery::configured_models(profile, discovered)
    }

    pub(super) fn all_enabled_model_count(&self) -> usize {
        self.config
            .profiles
            .iter()
            .map(|(profile_id, profile)| {
                let discovered = self
                    .cache
                    .profiles
                    .get(profile_id)
                    .map(|cached| cached.models.as_slice())
                    .unwrap_or_default();
                discovery::active_models(profile, discovered).len()
            })
            .sum()
    }

    pub(super) fn all_managed_models(&self) -> Vec<GlobalModelRef> {
        let mut models = Vec::new();
        for (profile_id, profile) in self
            .config
            .profiles
            .iter()
            .filter(|(_, profile)| profile.enabled)
        {
            let discovered = self
                .cache
                .profiles
                .get(profile_id)
                .map(|cached| cached.models.as_slice())
                .unwrap_or_default();
            let active = discovery::active_models(profile, discovered)
                .into_iter()
                .map(|model| canonical_model_id(&model.id))
                .collect::<BTreeSet<_>>();
            let Some(editor) = self.create_route_editor_for(profile_id.clone()) else {
                continue;
            };
            for catalog_model in &editor.catalog {
                let mut model = catalog_model.clone();
                model.id = editor.effective_id(&model.id);
                models.push(GlobalModelRef {
                    profile_id: profile_id.clone(),
                    profile_name: profile.name.clone(),
                    enabled: active.contains(&canonical_model_id(&model.id)),
                    model,
                });
            }
        }
        models
    }

    pub(super) fn selected_model(&self) -> Option<ModelEntry> {
        if self.view_mode == ViewMode::Provider {
            let fallback;
            let editor = match &self.provider_editor {
                Some(e) => e,
                None => {
                    fallback = self.create_route_editor();
                    fallback.as_ref()?
                }
            };
            let filtered = editor.filtered_indices();
            let &idx = filtered.get(editor.selected)?;
            let model = &editor.catalog[idx];
            let effective = editor.effective_id(&model.id);
            Some(ModelEntry {
                max_output_tokens: model.max_output_tokens,
                context_window: model.context_window,
                id: effective,
                label: model.label.clone(),
                description: model.description.clone(),
            })
        } else if self.view_mode == ViewMode::AllEnabled {
            self.all_managed_models()
                .get(self.model_idx)
                .map(|entry| entry.model.clone())
        } else {
            self.models().get(self.model_idx).cloned()
        }
    }

    pub(super) fn toggle_focus(&mut self) {
        if self.view_mode == ViewMode::Home {
            self.focus = Focus::Profiles;
        } else if self.view_mode == ViewMode::AllEnabled {
            self.focus = Focus::Models;
        } else {
            self.focus = match self.focus {
                Focus::Models => Focus::Details,
                _ => Focus::Models,
            };
        }
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        if self.view_mode == ViewMode::Home && self.focus == Focus::Profiles {
            let len = self.config.profiles.len().saturating_add(1);
            let current = self.home_selected_index();
            let next = ((current as isize + delta).rem_euclid(len as isize)) as usize;
            self.select_home_index(next);
            self.status_error = false;
            self.status = if self.home_all_selected && self.codex_ui.enabled {
                "ChatGPT Account · Enter to import or switch accounts".into()
            } else if self.home_all_selected {
                format!(
                    "All Models · {} models · Enter manage models",
                    self.all_enabled_model_count()
                )
            } else {
                let profile = self.selected_profile().expect("provider selected");
                format!(
                    "Selected {} · Space {} provider · Enter details",
                    profile.name,
                    if profile.enabled { "disable" } else { "enable" }
                )
            };
            return;
        }
        let len = match self.focus {
            Focus::Profiles => self.config.profiles.len(),
            Focus::Models if self.view_mode == ViewMode::AllEnabled => {
                self.all_managed_models().len()
            }
            Focus::Models => self.models().len(),
            Focus::Details => return,
        };
        if len == 0 {
            return;
        }
        let current = match self.focus {
            Focus::Profiles => &mut self.profile_idx,
            Focus::Models => &mut self.model_idx,
            Focus::Details => return,
        };
        *current = ((*current as isize + delta).rem_euclid(len as isize)) as usize;
        if self.focus == Focus::Profiles {
            self.model_idx = self.default_model_index();
            self.model_offset = 0;
            if let Some(name) = self.selected_profile().map(|profile| profile.name.clone()) {
                self.status_error = false;
                self.status = if self.view_mode == ViewMode::Home {
                    format!("Selected {name} · Enter details · e edit · x delete · r fetch models")
                } else {
                    format!("Selected {name} · Enter to choose a model")
                };
            }
        }
        if self.focus == Focus::Models {
            if self.view_mode == ViewMode::AllEnabled {
                if let Some(entry) = self.all_managed_models().get(self.model_idx) {
                    self.status_error = false;
                    self.status = format!(
                        "{} · {} · Space {} · Enter/click again to open provider",
                        entry.profile_name,
                        entry.model.label(),
                        if entry.enabled { "disable" } else { "enable" }
                    );
                }
            } else if let Some(model) = self.selected_model() {
                self.status_error = false;
                self.status = format!("Selected {} · Space toggles availability", model.label());
            }
        }
    }

    pub(super) fn default_model_index(&self) -> usize {
        self.selected_profile()
            .and_then(|profile| {
                self.models()
                    .iter()
                    .position(|model| model.id == profile.default_model)
            })
            .unwrap_or(0)
    }

    pub(super) fn new_profile(&mut self) {
        let mut form = ProfileForm::new();
        if self.client_tab() != ClientTab::Claude {
            form.fields.truncate(7);
        }
        self.modal = Some(Modal::Profile(Box::new(form)));
    }

    pub(super) fn create_route_editor(&self) -> Option<RouteEditor> {
        let profile_id = self.selected_profile_id()?;
        self.create_route_editor_for(profile_id)
    }

    pub(super) fn create_route_editor_for(&self, profile_id: String) -> Option<RouteEditor> {
        let profile = self.config.profiles.get(&profile_id)?;
        let references = profile
            .required_model_ids()
            .into_iter()
            .chain(profile.enabled_models.iter().cloned())
            .chain(profile.disabled_models.iter().cloned())
            .collect::<Vec<_>>();
        let one_m = references
            .iter()
            .filter(|id| has_1m_suffix(id))
            .map(|id| canonical_model_id(id))
            .collect();
        let default_model = canonical_model_id(&profile.default_model);
        let mut locked = profile
            .required_model_ids()
            .iter()
            .map(|id| canonical_model_id(id))
            .collect::<BTreeSet<_>>();
        locked.remove(&default_model);
        let catalog = normalize_model_catalog(self.catalog_models_for(&profile_id));
        let default_idx = catalog
            .iter()
            .position(|m| m.id == default_model)
            .unwrap_or(0);
        let selected = if self.model_idx < catalog.len() && self.model_idx > 0 {
            self.model_idx
        } else {
            default_idx
        };
        Some(RouteEditor {
            profile_id,
            original_profile: (*profile).clone(),
            provider_enabled: profile.enabled,
            catalog,
            enabled: profile
                .enabled_models
                .iter()
                .map(|id| canonical_model_id(id))
                .collect(),
            disabled: profile
                .disabled_models
                .iter()
                .map(|id| canonical_model_id(id))
                .collect(),
            locked,
            default_model,
            one_m,
            query: String::new(),
            selected,
            search_active: false,
            status: "Space toggle · 1 context · d default".into(),
        })
    }

    pub(super) fn edit_model(&mut self) {
        let Some(model) = self.selected_model() else {
            return;
        };
        let enabled = self
            .provider_editor
            .as_ref()
            .is_none_or(|editor| editor.is_enabled(&canonical_model_id(&model.id)));
        self.open_add_model_modal();
        if let Some(Modal::Model(form)) = &mut self.modal {
            form.original_model_id = Some(canonical_model_id(&model.id));
            form.fields[4].value = enabled.to_string();
            for (index, value) in [
                (0, canonical_model_id(&model.id)),
                (1, model.label.unwrap_or_default()),
                (2, model.description.unwrap_or_default()),
                (3, has_1m_suffix(&model.id).to_string()),
                (
                    5,
                    model
                        .max_output_tokens
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                ),
                (
                    6,
                    model
                        .context_window
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                ),
            ] {
                form.fields[index].value = value;
                form.fields[index].cursor = form.fields[index].char_count();
            }
        }
    }

    pub(super) fn open_add_model_modal(&mut self) {
        self.reload_for_edit();
        self.status_error = false;
        let cached = self
            .selected_profile_id()
            .and_then(|id| self.cache.profiles.get(&id).map(|c| c.models.clone()))
            .unwrap_or_default();
        let mut form = ModelForm::with_api_models(cached);
        form.original_profile = self.selected_profile().cloned().map(Box::new);
        self.modal = Some(Modal::Model(form));
    }

    pub(super) fn fetch_api_models_for_form(&mut self) {
        let instance = match &mut self.modal {
            Some(Modal::Model(form)) => {
                form.api_status = "Fetching models…".into();
                Some(form.instance)
            }
            _ => return,
        };
        self.start_discovery(instance);
    }

    pub(super) fn init_provider_editor(&mut self) {
        self.provider_editor = self.create_route_editor();
    }

    pub(super) fn ensure_provider_editor(&mut self) -> Option<&mut RouteEditor> {
        if self.provider_editor.is_none() {
            self.provider_editor = self.create_route_editor();
        }
        self.provider_editor.as_mut()
    }

    pub(super) fn commit_provider_editor(&mut self) -> Result<()> {
        let Some(editor) = &self.provider_editor else {
            return Ok(());
        };
        if !editor.provider_enabled {
            self.init_provider_editor();
            anyhow::bail!("Enable the provider on Home before changing model availability");
        }
        let mut profile = editor.original_profile.clone();
        apply_route_editor(&mut profile, editor);
        let id = editor.profile_id.clone();
        match config::update_client_profile(
            &self.paths.config,
            self.config_client(),
            &id,
            &editor.original_profile,
            &profile,
        ) {
            Ok(config) => self.config = config,
            Err(error) => {
                self.reload_for_edit();
                self.init_provider_editor();
                return Err(error);
            }
        }
        self.profile_idx = self
            .profile_ids()
            .iter()
            .position(|candidate| candidate == &id)
            .unwrap_or(0);
        self.refresh_editor_preserving_selection();
        Ok(())
    }

    pub(super) fn enter_provider_view(&mut self) {
        self.view_mode = ViewMode::Provider;
        self.focus = Focus::Models;
        self.init_provider_editor();
        let name = self
            .selected_profile()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.status_error = false;
        self.status = format!("Managing {name} · Space toggles model · Esc back to providers");
    }

    pub(super) fn enter_all_enabled_view(&mut self) {
        if self.codex_ui.enabled {
            self.open_codex_accounts();
            return;
        }
        self.view_mode = ViewMode::AllEnabled;
        self.focus = Focus::Models;
        self.model_idx = self
            .model_idx
            .min(self.all_managed_models().len().saturating_sub(1));
        self.model_offset = 0;
        self.status_error = false;
        self.status = "Space toggle model · Enter/click again to open provider · Esc back".into();
    }

    pub(super) fn return_home(&mut self) {
        self.view_mode = ViewMode::Home;
        self.focus = Focus::Profiles;
        self.status_error = false;
        self.status = "Returned to Providers home".into();
    }

    pub(super) fn open_selected_global_model(&mut self) {
        let Some(selected) = self.all_managed_models().get(self.model_idx).cloned() else {
            return;
        };
        let Some(profile_idx) = self
            .profile_ids()
            .iter()
            .position(|id| id == &selected.profile_id)
        else {
            return;
        };
        self.profile_idx = profile_idx;
        self.home_all_selected = false;
        self.view_mode = ViewMode::Provider;
        self.focus = Focus::Models;
        self.provider_editor = self.create_route_editor_for(selected.profile_id);
        if let Some(editor) = &mut self.provider_editor {
            let wanted = canonical_model_id(&selected.model.id);
            if let Some(index) = editor.catalog.iter().position(|model| model.id == wanted) {
                editor.selected = index;
            }
        }
        self.status_error = false;
        self.status = format!(
            "Viewing {} · {}",
            selected.profile_name,
            selected.model.label()
        );
    }

    pub(super) fn reload_for_edit(&mut self) {
        if !self.paths.config.exists() {
            return;
        }
        let selected = self.selected_profile_id();
        if let Ok(latest) = config::load_client(&self.paths.config, self.config_client()) {
            self.config = latest;
            self.profile_idx = selected
                .and_then(|id| {
                    self.profile_ids()
                        .iter()
                        .position(|candidate| candidate == &id)
                })
                .unwrap_or(0);
        }
    }

    pub(super) fn edit_profile(&mut self) {
        self.reload_for_edit();
        self.status_error = false;
        let Some(id) = self.selected_profile_id() else {
            return;
        };
        let profile = self.config.profiles[&id].clone();
        let mut form = ProfileForm::edit(id, &profile);
        if self.client_tab() != ClientTab::Claude {
            form.fields.truncate(7);
        }
        self.modal = Some(Modal::Profile(Box::new(form)));
    }

    pub(super) fn open_proxy_manager(&mut self) {
        if self.pi_enabled {
            return;
        }
        if self.pi_enabled {
            self.status = "Pi connects directly to providers; no CCSW proxy required".into();
            return;
        }
        let manager = ProxyManager::empty();
        self.modal = Some(Modal::Proxy(manager));
        self.start_proxy_action(ProxyControl::Refresh);
    }

    pub(super) fn open_help(&mut self) {
        let mut help = HelpModal::for_view(self.view_mode);
        help.pi = self.pi_enabled;
        help.codex = self.codex_ui.enabled;
        if help.codex && self.codex_ui.accounts {
            help.section = HelpSection::AllEnabled;
        }
        self.modal = Some(Modal::Help(help));
    }

    pub(super) fn refresh_models(&mut self) {
        self.start_discovery(None);
    }

    pub(super) fn enable_all_models(&mut self) {
        let Some(profile_id) = self.selected_profile_id() else {
            self.set_error("Create a profile before enabling models");
            return;
        };
        let catalog = self.catalog_models();
        if catalog.is_empty() {
            self.set_error("No models are available; fetch or add a model first");
            return;
        }
        let update = config::update_client(&self.paths.config, self.config_client(), |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            let required = profile
                .required_model_ids()
                .iter()
                .map(|id| canonical_model_id(id))
                .collect::<BTreeSet<_>>();
            let existing = profile
                .enabled_models
                .iter()
                .chain(profile.required_model_ids().iter())
                .map(|id| (canonical_model_id(id), id.clone()))
                .collect::<BTreeMap<_, _>>();
            profile.enabled_models = catalog
                .iter()
                .filter_map(|model| {
                    let canonical = canonical_model_id(&model.id);
                    (!required.contains(&canonical)).then(|| {
                        existing
                            .get(&canonical)
                            .cloned()
                            .unwrap_or_else(|| model.id.clone())
                    })
                })
                .collect();
            profile.disabled_models.clear();
            Ok(())
        });
        match update {
            Ok(config) => {
                self.config = config;
                self.model_idx = self.model_idx.min(self.models().len().saturating_sub(1));
                self.status_error = false;
                self.status = format!(
                    "Enabled all {} models in {profile_id} · click Sync all to Claude",
                    self.models().len()
                );
            }
            Err(error) => self.set_error(format!("Could not enable all models: {error:#}")),
        }
    }

    pub(super) fn sync_all_to_claude(&mut self) {
        if self.pi_enabled {
            self.apply_pi();
            return;
        }
        if self.codex_ui.enabled {
            self.apply_codex();
            return;
        }
        let preferred = self
            .selected_profile_id()
            .filter(|id| self.config.profiles[id].enabled);
        self.queue_sync(true, preferred);
    }

    pub(super) fn toggle_selected_provider(&mut self) -> Result<()> {
        let Some(profile_id) = self.selected_profile_id() else {
            return Ok(());
        };
        let enabled = !self.config.profiles[&profile_id].enabled;
        let original = self.config.profiles[&profile_id].clone();
        let mut edited = original.clone();
        edited.enabled = enabled;
        self.config = config::update_client_profile(
            &self.paths.config,
            self.config_client(),
            &profile_id,
            &original,
            &edited,
        )?;
        self.profile_idx = self
            .profile_ids()
            .iter()
            .position(|id| id == &profile_id)
            .unwrap_or(0);
        self.model_idx = 0;
        self.model_offset = 0;
        let action = if enabled { "Enabled" } else { "Disabled" };
        self.status_error = false;
        self.status = format!("{action} provider {profile_id}");
        Ok(())
    }

    pub(super) fn toggle_selected_global_model(&mut self) -> Result<()> {
        let Some(selected) = self.all_managed_models().get(self.model_idx).cloned() else {
            return Ok(());
        };
        let profile = self.toggled_global_model_profile(&selected)?;
        let profile_id = selected.profile_id.clone();
        self.config = config::update_client_profile(
            &self.paths.config,
            self.config_client(),
            &profile_id,
            &self.config.profiles[&profile_id],
            &profile,
        )?;
        let remaining = self.all_managed_models().len();
        self.model_idx = self.model_idx.min(remaining.saturating_sub(1));
        let action = if selected.enabled {
            "Disabled"
        } else {
            "Enabled"
        };
        self.status_error = false;
        self.status = format!(
            "{action} {} · {}",
            selected.profile_name,
            selected.model.label()
        );
        Ok(())
    }

    pub(super) fn toggled_global_model_profile(
        &self,
        selected: &GlobalModelRef,
    ) -> Result<Profile> {
        let mut editor = self
            .create_route_editor_for(selected.profile_id.clone())
            .context("provider no longer exists")?;
        let wanted = canonical_model_id(&selected.model.id);
        let index = editor
            .catalog
            .iter()
            .position(|model| model.id == wanted)
            .with_context(|| format!("model '{}' is no longer configured", selected.model.id))?;
        editor.selected = editor
            .filtered_indices()
            .iter()
            .position(|candidate| *candidate == index)
            .unwrap_or(0);
        editor.toggle_selected();
        let mut profile = self.config.profiles[&selected.profile_id].clone();
        apply_route_editor(&mut profile, &editor);
        Ok(profile)
    }

    pub(super) fn set_selected_as_default(&mut self) {
        if self.view_mode == ViewMode::Provider {
            if let Some(editor) = self.ensure_provider_editor() {
                editor.set_selected_default();
            }
            if let Err(error) = self.commit_provider_editor() {
                self.set_error(format!("Could not save changes: {error:#}"));
                return;
            }
            self.status_error = false;
            let def = self
                .provider_editor
                .as_ref()
                .map(|e| e.default_model.clone())
                .unwrap_or_default();
            self.status = format!("Default model set to {def}");
            return;
        }
        let Some(profile_id) = self.selected_profile_id() else {
            return;
        };
        let Some(model) = self.selected_model() else {
            return;
        };
        let model_id = model.id.clone();
        let old_default = self
            .config
            .profiles
            .get(&profile_id)
            .map(|p| p.default_model.clone());
        let update = config::update_client(&self.paths.config, self.config_client(), |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            if let Some(old) = old_default
                && old != model_id
                && !profile.enabled_models.contains(&old)
            {
                profile.enabled_models.push(old);
            }
            profile.enabled_models.retain(|id| id != &model_id);
            profile
                .disabled_models
                .retain(|id| canonical_model_id(id) != canonical_model_id(&model_id));
            profile.default_model = model_id.clone();
            Ok(())
        });
        match update {
            Ok(config) => {
                self.config = config;
                self.select_model_id(&model_id);
                self.status_error = false;
                self.status = format!("Default model set to {model_id}");
            }
            Err(error) => self.set_error(format!("Could not set default model: {error:#}")),
        }
    }

    pub(super) fn toggle_selected_model_1m(&mut self) {
        if self.view_mode == ViewMode::Provider {
            if let Some(editor) = self.ensure_provider_editor() {
                editor.toggle_selected_1m();
            }
            if let Err(error) = self.commit_provider_editor() {
                self.set_error(format!("Could not save changes: {error:#}"));
                return;
            }
            self.status_error = false;
            self.status = "Toggled 1M context on selected model".into();
            return;
        }
        let Some(profile_id) = self.selected_profile_id() else {
            return;
        };
        let Some(model) = self.selected_model() else {
            return;
        };
        let old_id = model.id.clone();
        let base = canonical_model_id(&old_id);
        let new_id = if has_1m_suffix(&old_id) {
            base.clone()
        } else {
            format!("{base}[1m]")
        };
        let update = config::update_client(&self.paths.config, self.config_client(), |latest| {
            let profile = latest
                .profiles
                .get_mut(&profile_id)
                .context("profile was removed in another CCSW instance")?;
            if profile.default_model == old_id {
                profile.default_model = new_id.clone();
            }
            for role in [
                &mut profile.aliases.opus,
                &mut profile.aliases.sonnet,
                &mut profile.aliases.haiku,
                &mut profile.aliases.fable,
                &mut profile.subagent_model,
            ]
            .into_iter()
            .flatten()
            {
                if *role == old_id {
                    *role = new_id.clone();
                }
            }
            for fb in &mut profile.fallback_models {
                if *fb == old_id {
                    *fb = new_id.clone();
                }
            }
            for em in &mut profile.enabled_models {
                if *em == old_id {
                    *em = new_id.clone();
                }
            }
            for disabled in &mut profile.disabled_models {
                if *disabled == old_id {
                    *disabled = new_id.clone();
                }
            }
            for m in &mut profile.models {
                if m.id == old_id {
                    m.id = new_id.clone();
                    if let Some(label) = &mut m.label {
                        if new_id.ends_with("[1m]") && !label.contains("1M") {
                            *label = format!("{label} · 1M");
                        } else if !new_id.ends_with("[1m]") {
                            *label = label.replace(" · 1M", "").replace(" 1M", "");
                        }
                    }
                }
            }
            Ok(())
        });
        match update {
            Ok(config) => {
                self.config = config;
                self.select_model_id(&new_id);
                self.status_error = false;
                self.status = if has_1m_suffix(&new_id) {
                    format!("1M context enabled for {base}")
                } else {
                    format!("1M context disabled for {base}")
                };
            }
            Err(error) => self.set_error(format!("Could not toggle 1M context: {error:#}")),
        }
    }

    pub(super) fn delete_selected_model(&mut self) {
        let Some(model) = self.selected_model() else {
            return;
        };
        let base_id = canonical_model_id(&model.id);
        let is_custom = self.selected_profile().is_some_and(|profile| {
            profile
                .models
                .iter()
                .any(|entry| canonical_model_id(&entry.id) == base_id)
        });
        if is_custom {
            self.modal = Some(Modal::DeleteModel);
        } else {
            self.set_error(
                "Gateway models cannot be deleted · press Space to enable or disable this model",
            );
        }
    }

    pub(super) fn select_model_id(&mut self, model_id: &str) {
        if let Some(index) = self.models().iter().position(|model| model.id == model_id) {
            self.model_idx = index;
        }
    }

    pub(super) fn set_error(&mut self, message: impl Into<String>) {
        self.status_error = true;
        self.status = message.into();
    }
}
