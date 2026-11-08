//! Strict Matroska cover selection; no external images or Windows providers.
use crate::{capture::Session, pixels::Screenshot};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn cover_track(tracks: &Value) -> Option<i64> {
    tracks.as_array()?.iter().find_map(|track| {
        if track["type"] != "video" || track["albumart"] != true || track["external"] == true {
            return None;
        }
        let name = track["metadata"]
            .as_object()?
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("filename"))?
            .1
            .as_str()?;
        if !matches!(name, "cover.jpg" | "cover.jpeg") {
            return None;
        }
        track["id"].as_i64().filter(|id| *id > 0)
    })
}

impl Session<'_> {
    /// A bounded optional probe. The caller restores the normal demuxer on miss/error.
    pub(crate) fn embedded_cover(
        &mut self,
        path: &Path,
        max_side: u32,
        deadline: Instant,
    ) -> Result<Option<Screenshot>> {
        // Do not consume the whole frame-extraction budget on a broken attachment.
        let now = Instant::now();
        let budget = deadline.saturating_duration_since(now) / 2;
        let probe_deadline = now + budget.min(Duration::from_secs(2));
        self.load_tracks(path, probe_deadline)?;
        let Some(id) = cover_track(&self.property("track-list")?) else {
            return Ok(None);
        };
        // Start with an empty video output. Switching tracks in a paused file can
        // leave the previous video frame available when a cover decoder fails.
        self.stop(probe_deadline)?;
        self.load_video(path, Some("lavf"), &id.to_string(), probe_deadline)?;
        ensure!(
            self.property("vid")?.as_i64() == Some(id),
            "Cover track not selected"
        );
        self.request(
            &["screenshot-raw", "video"],
            probe_deadline,
            Some(max_side),
            false,
        )?
        .context("Cover screenshot returned no pixels")
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_exact_embedded_cover_names_are_selected() {
        for name in [
            "cover.jpg",
            "cover.jpeg",
            "cover.png",
            "Cover.jpg",
            "other.jpg",
            "folder/cover.jpg",
        ] {
            let track = json!({"id": 2, "type": "video", "albumart": true,
                "external": false, "metadata": {"filename": name}});
            assert_eq!(
                cover_track(&json!([track])),
                matches!(name, "cover.jpg" | "cover.jpeg").then_some(2)
            );
        }
    }

    #[test]
    fn cover_name_alone_does_not_make_a_video_or_external_image_a_cover() {
        for extra in [
            json!({"albumart": false}),
            json!({"external": true}),
            json!({"type": "audio"}),
            json!({"id": 0}),
            json!({"metadata": {}}),
        ] {
            let mut track = json!({"id": 2, "type": "video", "albumart": true,
                "external": false, "metadata": {"filename": "cover.jpg"}});
            track
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert_eq!(cover_track(&json!([track])), None);
        }
    }
}
