use crate::app::state::{ImageViewerApp, PendingVideoPlayerAction};
use crate::ui::components::{MediaPreview, MpvPreview};
use std::path::{Path, PathBuf};

fn should_close_embedded_preview(
    preview: &MediaPreview,
    is_owner: bool,
    selected_path: Option<&Path>,
) -> bool {
    is_owner
        && preview.is_video()
        && !preview.is_detached()
        && !selected_path.is_some_and(|path| {
            preview
                .path()
                .is_some_and(|playing| super::selection::selected_paths_match(playing, path))
        })
}

fn cancel_stale_preview_request(
    pending: &mut Option<PendingVideoPlayerAction>,
    active_tab_id: usize,
    selected_path: Option<&Path>,
) {
    let is_stale = matches!(pending,
        Some(PendingVideoPlayerAction::Preview { path, tab_id })
            if *tab_id == active_tab_id
                && !selected_path.is_some_and(|selected| {
                    super::selection::selected_paths_match(path, selected)
                })
    );
    if is_stale {
        *pending = None;
    }
}

impl ImageViewerApp {
    pub(super) fn close_embedded_preview_for_selection_change(&mut self) {
        let active_tab_id = self.tab_manager.active().id;
        let is_owner = self.media_preview_owner_tab_id == Some(active_tab_id);
        let selected_path = self.selected_file.as_ref().map(|file| file.path.as_path());
        // A queued launch must not become valid again after A -> B -> A.
        cancel_stale_preview_request(
            &mut self.pending_video_player_action,
            active_tab_id,
            selected_path,
        );
        if self
            .media_preview
            .as_ref()
            .is_some_and(|preview| should_close_embedded_preview(preview, is_owner, selected_path))
        {
            self.destroy_media_preview();
        }
    }
}

/// Replace rather than retarget: only one embedded playback session may exist.
pub(super) fn replace_video_preview(
    preview: &mut Option<MediaPreview>,
    owner_tab_id: &mut Option<usize>,
    path: PathBuf,
    tab_id: usize,
    volume: f32,
) {
    if let Some(mut previous) = preview.take() {
        previous.shutdown();
    }
    *owner_tab_id = None;

    let mut player = MpvPreview::new(path);
    player.play_on_init = true;
    player.show_player = true;
    player.initial_volume = volume;
    *preview = Some(MediaPreview::Video(Box::new(player)));
    *owner_tab_id = Some(tab_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn video_preview() -> MediaPreview {
        MediaPreview::Video(Box::new(MpvPreview::new(PathBuf::from("video.mp4"))))
    }

    #[test]
    fn owner_selection_change_closes_embedded_playback() {
        let preview = video_preview();
        for path in ["other.mp4", "image.png", "folder"] {
            assert!(should_close_embedded_preview(
                &preview,
                true,
                Some(Path::new(path))
            ));
        }
        assert!(should_close_embedded_preview(&preview, true, None));
        assert!(!should_close_embedded_preview(
            &preview,
            true,
            Some(Path::new("video.mp4"))
        ));
    }

    #[test]
    fn another_tabs_selection_never_closes_playback() {
        let preview = video_preview();
        assert!(!should_close_embedded_preview(
            &preview,
            false,
            Some(Path::new("other.mp4"))
        ));
        assert!(!should_close_embedded_preview(&preview, false, None));
    }

    #[test]
    fn detached_and_fullscreen_playback_are_not_closed_by_selection() {
        let mut preview = video_preview();
        preview.set_detached(true);
        assert!(!should_close_embedded_preview(&preview, true, None));
        preview.toggle_maximized();
        assert!(!should_close_embedded_preview(&preview, true, None));
    }

    #[test]
    fn new_playback_replaces_the_previous_session_even_for_the_same_file() {
        let mut preview = Some(video_preview());
        let mut owner = Some(1);
        for (path, tab_id) in [("other.mp4", 2), ("other.mp4", 3), ("video.mp4", 1)] {
            let previous_owner = owner;
            let Some(MediaPreview::Video(previous)) = preview.as_ref() else {
                panic!("expected video preview");
            };
            let previous_state = previous.state.clone();
            {
                let mut state = previous_state.write().expect("state lock");
                state.current_time = 30.0;
                state.duration = 60.0;
                state.is_playing = true;
            }
            replace_video_preview(&mut preview, &mut owner, PathBuf::from(path), tab_id, 0.42);
            let Some(MediaPreview::Video(current)) = preview.as_ref() else {
                panic!("expected video preview");
            };
            assert!(!Arc::ptr_eq(&previous_state, &current.state));
            assert_eq!(current.path, PathBuf::from(path));
            assert!(current.show_player);
            assert!(current.play_on_init);
            assert!(current.is_docked());
            assert_eq!(owner, Some(tab_id));
            // After ownership transfers, browsing in the old tab cannot close it.
            assert!(!should_close_embedded_preview(
                preview.as_ref().expect("preview"),
                owner == previous_owner,
                None,
            ));
            assert_eq!(current.initial_volume, 0.42);
            let state = current.get_state();
            assert_eq!(state.current_time, 0.0);
            assert_eq!(state.duration, 0.0);
            assert!(!state.is_playing);
        }
    }

    #[test]
    fn selection_round_trip_cannot_revive_a_queued_launch() {
        let mut pending = Some(PendingVideoPlayerAction::Preview {
            path: PathBuf::from("video.mp4"),
            tab_id: 1,
        });
        cancel_stale_preview_request(&mut pending, 1, Some(Path::new("other.mp4")));
        assert!(pending.is_none());
        cancel_stale_preview_request(&mut pending, 1, Some(Path::new("video.mp4")));
        assert!(pending.is_none());
    }

    #[test]
    fn clearing_selection_cancels_a_queued_launch_even_before_player_creation() {
        let mut pending = Some(PendingVideoPlayerAction::Preview {
            path: PathBuf::from("video.mp4"),
            tab_id: 1,
        });
        cancel_stale_preview_request(&mut pending, 1, None);
        assert!(pending.is_none());
    }

    #[test]
    fn queued_launch_survives_same_selection_and_browsing_in_another_tab() {
        let mut pending = Some(PendingVideoPlayerAction::Preview {
            path: PathBuf::from("video.mp4"),
            tab_id: 1,
        });
        cancel_stale_preview_request(&mut pending, 1, Some(Path::new("video.mp4")));
        assert!(pending.is_some());
        cancel_stale_preview_request(&mut pending, 2, Some(Path::new("other.mp4")));
        assert!(pending.is_some());
        cancel_stale_preview_request(&mut pending, 2, None);
        assert!(pending.is_some());
    }

    #[test]
    fn selection_does_not_cancel_a_standalone_launch() {
        let mut pending = Some(PendingVideoPlayerAction::StandaloneFile {
            path: PathBuf::from("video.mp4"),
            position: 30.0,
            volume_override: None,
        });
        cancel_stale_preview_request(&mut pending, 1, None);
        assert!(matches!(
            pending,
            Some(PendingVideoPlayerAction::StandaloneFile { .. })
        ));
    }
}
