//! Ancestor grouping shares the existing worker, but has independent ownership per pane/directory.
use super::*;

impl ImageViewerApp {
    pub(crate) fn clear_miller_column_groups(&mut self) {
        let pane = usize::from(
            matches!(
                self.dual_panel_active,
                crate::app::dual_panel::ActivePanel::Right
            ) ^ self.in_inactive_panel_context,
        );
        self.columns
            .grouping
            .retain_ancestors(pane, &HashSet::new());
    }

    pub(crate) fn miller_metadata_projections(
        &mut self,
        ctx: &egui::Context,
        directories: &[PathBuf],
        listings: impl Iterator<Item = Option<Arc<Vec<FileEntry>>>>,
    ) -> Option<Vec<Arc<GroupProjection>>> {
        let GroupMode::Column(column) = self.group_mode else {
            self.clear_miller_column_groups();
            return None;
        };
        let pane = usize::from(
            matches!(
                self.dual_panel_active,
                crate::app::dual_panel::ActivePanel::Right
            ) ^ self.in_inactive_panel_context,
        );
        let listings: Vec<_> = listings.collect();
        let keep = directories
            .iter()
            .zip(&listings)
            .filter(|(_, items)| items.is_some())
            .map(|(path, _)| path.clone())
            .collect();
        self.columns.grouping.retain_ancestors(pane, &keep);
        let busy = self.rectangle_selection_state.is_some()
            || self.is_item_dragging
            || self.external_drop_active
            || self.renaming_state.is_some()
            || self.context_menu.is_open;
        let mut projections = Vec::with_capacity(directories.len());
        for (directory, items) in directories.iter().zip(listings) {
            let Some(items) = items else {
                projections.push(Arc::default());
                continue;
            };
            // Ancestor identity follows its own listing, not navigation inside a descendant.
            let sorting = &mut self.columns.sorting;
            let fingerprint =
                self.miller_columns
                    .grouping
                    .metadata_fingerprint(directory, &items, |items| sorting.fingerprint(items));
            let signature = self.column_group_signature_for(
                column,
                &directory.to_string_lossy(),
                0,
                fingerprint,
            );
            let owner = GroupOwner::Ancestor(pane, directory.clone());
            let cached = self
                .miller_columns
                .grouping
                .metadata_groups(directory, signature);
            let state = &mut self.columns.grouping;
            state.set_owner(owner.clone(), Some(signature));
            if let std::collections::hash_map::Entry::Vacant(entry) = state.jobs.entry(signature) {
                if let Some(cached) = cached {
                    let mut job = GroupJob::new(Arc::new(AtomicBool::new(false)));
                    job.in_flight = false;
                    job.retry_at = cached.deferred.then_some(Instant::now());
                    job.latest = Some(cached);
                    entry.insert(job);
                }
            }
            if state.needs_request(signature, Instant::now(), ctx) {
                if let Some(sender) = &state.sender {
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let request = GroupRequest {
                        signature,
                        items: items.clone(),
                        column,
                        tags: self.tag_assignments_normalized.clone(),
                        definitions: self.tag_definitions.clone(),
                        cancelled: cancelled.clone(),
                        ctx: ctx.clone(),
                    };
                    if sender.try_send(request).is_ok() {
                        state.record_request(signature, cancelled);
                    } else {
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
            }
            let current = self.miller_columns.grouping.metadata_projection(
                directory,
                signature,
                items.clone(),
                column,
                self.group_descending,
                None,
            );
            state.publish_owner(signature, &owner, busy, &current);
            let published = state
                .jobs
                .get(&signature)
                .and_then(|job| job.published.get(&owner))
                .map(|published| published.groups.clone());
            let projection = self.miller_columns.grouping.metadata_projection(
                directory,
                signature,
                items,
                column,
                self.group_descending,
                published,
            );
            state.remember_owner(signature, &owner, &projection);
            projections.push(projection);
        }
        Some(projections)
    }
}
