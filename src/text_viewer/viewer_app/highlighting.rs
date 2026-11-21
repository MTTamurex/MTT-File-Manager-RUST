use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::{bounded, Receiver};
use eframe::egui;

enum LoadState {
    NotRequested,
    Loading {
        dark_mode: bool,
        receiver: Receiver<Option<Arc<egui::text::LayoutJob>>>,
    },
    Ready {
        dark_mode: bool,
        job: Option<Arc<egui::text::LayoutJob>>,
    },
}

pub(super) struct SyntaxHighlighting {
    state: LoadState,
}

impl Default for SyntaxHighlighting {
    fn default() -> Self {
        Self {
            state: LoadState::NotRequested,
        }
    }
}

impl SyntaxHighlighting {
    pub(super) fn update(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        text: Arc<String>,
        is_markdown: bool,
        dark_mode: bool,
    ) {
        if is_markdown {
            return;
        }
        let already_requested = match &self.state {
            LoadState::NotRequested => false,
            LoadState::Loading {
                dark_mode: requested_mode,
                ..
            }
            | LoadState::Ready {
                dark_mode: requested_mode,
                ..
            } => *requested_mode == dark_mode,
        };
        if already_requested {
            return;
        }

        let (reply, receiver) = bounded(1);
        let path = path.to_path_buf();
        let context = ctx.clone();
        let worker = std::thread::Builder::new()
            .name("text-viewer-syntax".into())
            .spawn(move || {
                let job = crate::ui::preview_panel::text_syntax::build_highlight_job(
                    &path, &text, dark_mode,
                )
                .map(Arc::new);
                let _ = reply.send(job);
                context.request_repaint();
            });

        self.state = if worker.is_ok() {
            LoadState::Loading {
                dark_mode,
                receiver,
            }
        } else {
            LoadState::Ready {
                dark_mode,
                job: None,
            }
        };
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let completion = match &self.state {
            LoadState::Loading {
                dark_mode,
                receiver,
            } => match receiver.try_recv() {
                Ok(job) => Some((*dark_mode, job)),
                Err(crossbeam_channel::TryRecvError::Disconnected) => Some((*dark_mode, None)),
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    None
                }
            },
            LoadState::NotRequested | LoadState::Ready { .. } => None,
        };
        if let Some((dark_mode, job)) = completion {
            self.state = LoadState::Ready { dark_mode, job };
        }
    }

    pub(super) fn job_for_range(
        &self,
        range: std::ops::Range<usize>,
    ) -> Option<egui::text::LayoutJob> {
        let job = match &self.state {
            LoadState::Ready { job: Some(job), .. } => job,
            _ => return None,
        };
        crate::ui::preview_panel::text_syntax::slice_layout_job(job, range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_builds_shared_syntax_colors_without_blocking_the_ui() {
        let context = egui::Context::default();
        let source = Arc::new("fn main() {\r\n    let value = 42;\r\n}\r\n".to_string());
        let mut highlighting = SyntaxHighlighting::default();
        highlighting.update(
            &context,
            Path::new("source.rs"),
            Arc::clone(&source),
            false,
            true,
        );
        let LoadState::Loading { receiver, .. } = &highlighting.state else {
            panic!("highlighting should start on a worker");
        };
        let job = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        highlighting.state = LoadState::Ready {
            dark_mode: true,
            job: Some(job),
        };
        let start = source.find("    let").unwrap();
        let line = highlighting
            .job_for_range(start..start + "    let value = 42;".len())
            .unwrap();
        assert_eq!(line.text, "    let value = 42;");
        assert!(line.sections.len() > 1);
        highlighting.update(&context, Path::new("source.rs"), source, false, true);
        assert!(matches!(highlighting.state, LoadState::Ready { .. }));
    }
}
