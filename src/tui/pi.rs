use super::*;
impl App {
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
                self.status = crate::pi::import(&self.paths, false)?.replace('\n', " · ");
                self.config = config::load_client(&self.paths.config, config::Client::Pi)?;
            }
            KeyCode::Char('p') => self.apply_pi(),
            KeyCode::Char('s') => self.status = crate::pi::status(&self.paths)?,
            KeyCode::Char('D') => {
                crate::pi::disconnect(&self.paths)?;
                self.status = "Pi disconnected; managed settings restored".into();
            }
            _ => return Ok(None),
        }
        Ok(Some(false))
    }
    pub(super) fn apply_pi(&mut self) {
        let Some(profile) = self.selected_profile_id() else {
            self.set_error("Select a Pi provider");
            return;
        };
        let model = self.selected_model().map(|m| m.id.clone());
        match crate::pi::apply(&self.paths, &profile, model.as_deref()) {
            Ok(()) => {
                self.status_error = false;
                self.status =
                    "Pi configuration saved · models.json / settings.json · open /model in Pi"
                        .into();
            }
            Err(error) => self.set_error(format!("Pi sync failed: {error:#}")),
        }
    }
    pub(super) fn sync_pi_after_edit(&mut self) {
        if !self.pi_enabled {
            return;
        }
        if let Err(error) = crate::pi::sync_if_connected(&self.paths) {
            self.set_error(format!("Pi pending sync: {error:#} · p retry"));
        }
    }
}
