use super::*;
impl App {
    pub(super) fn load_client_config(&self) -> Result<Config> {
        if self.pi_enabled {
            crate::pi::native::load(&self.pi_home).map(|mut config| {
                config.usage_refresh_secs = self.config.usage_refresh_secs;
                config
            })
        } else {
            config::load_client(&self.paths.config, self.config_client())
        }
    }

    pub(super) fn update_client_config(
        &self,
        edit: impl FnOnce(&mut Config) -> Result<()>,
    ) -> Result<Config> {
        if self.pi_enabled {
            crate::pi::native::update(&self.pi_home, edit).map(|mut config| {
                config.usage_refresh_secs = self.config.usage_refresh_secs;
                config
            })
        } else {
            config::update_client(&self.paths.config, self.config_client(), edit)
        }
    }

    pub(super) fn update_client_profile(
        &self,
        id: &str,
        original: &Profile,
        edited: &Profile,
    ) -> Result<Config> {
        if !self.pi_enabled {
            return config::update_client_profile(
                &self.paths.config,
                self.config_client(),
                id,
                original,
                edited,
            );
        }
        self.update_client_config(|config| {
            let current = config
                .profiles
                .get(id)
                .context("Provider was removed; reload before editing")?;
            let merged = config::merge_profile(original, edited, current)?;
            config.profiles.insert(id.into(), merged);
            Ok(())
        })
    }

    pub(super) fn footer_uses_short_labels(&self, area: Rect, compact: bool) -> bool {
        let controls: Vec<_> = footer_controls(Rect::new(0, 0, 200, 1), compact, self.view_mode)
            .into_iter()
            .chain(
                (provider_workspace(self.screen) && self.view_mode != ViewMode::Home)
                    .then_some((FooterControl::Quit, Rect::default())),
            )
            .filter(|(control, _)| match control {
                FooterControl::DeleteProfile => {
                    !self.home_all_selected && self.selected_profile().is_some()
                }
                FooterControl::Proxy => !self.grok_enabled,
                _ => true,
            })
            .collect();
        let width: usize = controls
            .iter()
            .map(|(control, _)| {
                let (label, _) = self.footer_control_style(*control, compact, false);
                UnicodeWidthStr::width(label.as_str())
            })
            .sum();
        width + controls.len().saturating_sub(1) > usize::from(area.width)
    }

    pub(super) fn client_footer_controls(
        &self,
        area: Rect,
        compact: bool,
    ) -> Vec<(FooterControl, Rect)> {
        if area.width == 0 || area.height == 0 {
            return vec![];
        }
        // Collect the complete control list before filtering and measuring labels.
        let template_area = Rect::new(0, area.y, if area.width < 55 { 54 } else { 200 }, 1);
        let controls: Vec<_> = footer_controls(template_area, compact, self.view_mode)
            .into_iter()
            .chain(
                (provider_workspace(self.screen) && self.view_mode != ViewMode::Home)
                    .then_some((FooterControl::Quit, Rect::default())),
            )
            .filter(|(control, _)| {
                *control != FooterControl::DeleteProfile
                    || (!self.home_all_selected && self.selected_profile().is_some())
            })
            .filter(|(control, _)| !self.grok_enabled || *control != FooterControl::Proxy)
            .map(|(control, _)| {
                let (label, _) = self.footer_control_style(
                    control,
                    compact,
                    self.footer_uses_short_labels(area, compact),
                );
                (control, UnicodeWidthStr::width(label.as_str()) as u16)
            })
            .collect();
        let count = controls.len() as u16;
        let label_width: u16 = controls.iter().map(|(_, width)| *width).sum();
        let roomy = label_width + count * 2 + count.saturating_sub(1) * 2 <= area.width;
        let padding = if roomy { 2 } else { 0 };
        let gap = if roomy { 2 } else { 1 };
        let mut x = area.x;
        controls
            .into_iter()
            .filter_map(|(control, width)| {
                let width = width + padding;
                if x + width > area.right() {
                    return None;
                }
                let rect = Rect::new(x, area.y, width, 1);
                x += width + gap;
                Some((control, rect))
            })
            .collect()
    }

    pub(super) fn handle_pi_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        if key.code == KeyCode::F(2) && !self.codex_navigation_blocked() {
            self.select_client_tab(self.client_tab().next());
            return Ok(Some(false));
        }
        if !self.pi_enabled
            || self
                .provider_editor
                .as_ref()
                .is_some_and(|e| e.search_active)
        {
            return Ok(None);
        }
        match key.code {
            KeyCode::Char('i') => {
                self.config = self.load_client_config()?;
                self.provider_editor = None;
                self.profile_idx = self
                    .profile_idx
                    .min(self.config.profiles.len().saturating_sub(1));
                self.model_idx = 0;
                self.status = crate::pi::native::description(&self.pi_home, &self.config);
            }
            KeyCode::Char('p') => self.apply_pi(),
            KeyCode::Char('s') => {
                self.status =
                    crate::pi::native::description(&self.pi_home, &self.load_client_config()?)
            }
            KeyCode::Char('D') => {
                self.status = "Pi files are edited directly; no connection to disconnect".into();
            }
            KeyCode::Char(' ' | 'A' | 'C') => {
                self.status_error = false;
                self.status =
                    "Pi lists every configured model; use x to delete, p to set the default".into();
            }
            _ => return Ok(None),
        }
        Ok(Some(false))
    }
    pub(super) fn apply_pi(&mut self) {
        let global = (self.view_mode == ViewMode::AllEnabled)
            .then(|| self.all_managed_models().get(self.model_idx).cloned())
            .flatten();
        let Some(profile) = global
            .as_ref()
            .map(|entry| entry.profile_id.clone())
            .or_else(|| self.selected_profile_id())
        else {
            self.set_error("Select a Pi provider");
            return;
        };
        let model = global
            .map(|entry| entry.model.id.clone())
            .or_else(|| self.selected_model().map(|m| m.id.clone()));
        let model = model
            .or_else(|| {
                self.config
                    .profiles
                    .get(&profile)
                    .map(|p| p.default_model.clone())
            })
            .unwrap_or_default();
        match crate::pi::native::set_default(&self.pi_home, &profile, &model) {
            Ok(()) => {
                if let Ok(config) = self.load_client_config() {
                    self.config = config;
                    self.refresh_editor_preserving_selection();
                }
                self.status_error = false;
                self.status = "Pi default saved to settings.json · open /model in Pi".into();
            }
            Err(error) => self.set_error(format!("Could not set Pi default: {error:#}")),
        }
    }
    pub(super) fn sync_pi_after_edit(&mut self) {
        if self.pi_enabled
            && let Err(error) = proxy::prune_pi_routes(&self.paths, &self.pi_home)
        {
            self.set_error(format!("Pi saved, but proxy cleanup failed: {error:#}"));
        }
    }

    pub(super) fn toggle_pi_proxy(&mut self) {
        let selected = (self.view_mode == ViewMode::AllEnabled)
            .then(|| self.all_managed_models().get(self.model_idx).cloned())
            .flatten();
        let Some(id) = selected
            .as_ref()
            .map(|entry| entry.profile_id.clone())
            .or_else(|| self.selected_profile_id())
        else {
            self.set_error("Select a Pi provider to toggle its proxy API");
            return;
        };
        let model = selected
            .map(|entry| entry.model.id)
            .or_else(|| self.selected_model().map(|model| model.id.clone()))
            .or_else(|| {
                self.config
                    .profiles
                    .get(&id)
                    .map(|profile| profile.default_model.clone())
            })
            .unwrap_or_default();
        let result = (|| -> Result<Option<String>> {
            if crate::pi::native::proxy_endpoint(&self.pi_home, &id)?.is_some() {
                crate::pi::native::set_proxy(&self.pi_home, &id, &model, None)?;
                proxy::remove_pi_route(&self.paths, &self.pi_home, &id)?;
                return Ok(None);
            }
            let (plan, url, token) = proxy::prepare_pi_route(&self.paths, &self.pi_home, &id)?;
            proxy::apply_pi_route(&self.paths, &plan)?;
            if let Err(error) =
                crate::pi::native::set_proxy(&self.pi_home, &id, &model, Some((&url, &token)))
            {
                proxy::restore_pi_route(&self.paths, &plan)
                    .context("Pi proxy setup failed and route rollback failed")?;
                return Err(error);
            }
            Ok(Some(url))
        })();
        match result {
            Ok(endpoint) => {
                if let Ok(config) = self.load_client_config() {
                    self.config = config;
                    self.refresh_editor_preserving_selection();
                }
                self.status_error = false;
                self.status = match endpoint {
                    Some(url) => format!("Pi proxy API enabled · {url} · open /model in Pi"),
                    None => "Pi proxy API disabled · direct provider restored".into(),
                };
            }
            Err(error) => self.set_error(format!("Could not toggle Pi proxy API: {error:#}")),
        }
    }
}
