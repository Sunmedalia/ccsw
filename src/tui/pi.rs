use super::*;
impl App {
    pub(super) fn load_client_config(&self) -> Result<Config> {
        if self.pi_enabled {
            crate::pi::native::load(&self.pi_home)
        } else {
            config::load_client(&self.paths.config, self.config_client())
        }
    }

    pub(super) fn update_client_config(
        &self,
        edit: impl FnOnce(&mut Config) -> Result<()>,
    ) -> Result<Config> {
        if self.pi_enabled {
            crate::pi::native::update(&self.pi_home, edit)
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

    pub(super) fn client_footer_controls(
        &self,
        area: Rect,
        compact: bool,
    ) -> Vec<(FooterControl, Rect)> {
        let mut x = area.x;
        footer_controls(area, compact, self.view_mode)
            .into_iter()
            .filter(|(control, _)| !self.pi_enabled || *control != FooterControl::Proxy)
            .map(|(control, mut rect)| {
                rect.x = x;
                x = x.saturating_add(rect.width).saturating_add(1);
                (control, rect)
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
            KeyCode::Char(' ') => {
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
            Err(error) => self.set_error(format!("Pi sync failed: {error:#}")),
        }
    }
    pub(super) fn sync_pi_after_edit(&mut self) {
        // Pi mutations already save directly to the native files.
    }
}
