//! Repaint and cache-retune policy after releasing the inactive thumbnail pipeline.
use eframe::egui;
use std::time::{Duration, Instant};

pub(super) fn finish(
    ctx: &egui::Context,
    last_retune: &mut Instant,
    released_any: bool,
    cleared_selection: bool,
) {
    // The release resets cache capacities and budgets even when caches were
    // empty. Keep retuning eligible on return to a thumbnail-capable view.
    let now = Instant::now();
    *last_retune = now.checked_sub(Duration::from_secs(10)).unwrap_or(now);
    // A cache policy change alone does not require another visual pass.
    if released_any || cleared_selection {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn settled_context() -> (egui::Context, Arc<AtomicUsize>) {
        let ctx = egui::Context::default();
        for _ in 0..5 {
            let _ = ctx.run_ui(Default::default(), |_| {});
        }
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = Arc::clone(&requests);
        ctx.set_request_repaint_callback(move |_| {
            captured.fetch_add(1, Ordering::Relaxed);
        });
        (ctx, requests)
    }

    #[test]
    fn repeated_empty_release_does_not_request_repaint_but_allows_retuning() {
        let (ctx, requests) = settled_context();
        let mut last_retune = Instant::now();
        for _ in 0..3 {
            finish(&ctx, &mut last_retune, false, false);
        }
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        assert!(!ctx.has_requested_repaint());
        assert!(last_retune.elapsed() >= Duration::from_secs(10));
    }

    #[test]
    fn resource_or_selection_release_still_requests_repaint() {
        for (released_any, cleared_selection) in [(true, false), (false, true), (true, true)] {
            let (ctx, requests) = settled_context();
            let mut last_retune = Instant::now();
            finish(&ctx, &mut last_retune, released_any, cleared_selection);
            assert!(requests.load(Ordering::Relaxed) > 0);
            assert!(ctx.has_requested_repaint());
            assert!(last_retune.elapsed() >= Duration::from_secs(10));
        }
    }
}
