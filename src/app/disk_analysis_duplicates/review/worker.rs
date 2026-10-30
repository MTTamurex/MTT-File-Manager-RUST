use super::*;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

pub(super) struct Request {
    pub generation: u64,
    pub slot: usize,
    pub member: DuplicateMember,
    pub path: PathBuf,
}
pub(super) struct Response {
    pub generation: u64,
    pub slot: usize,
    pub idx: u32,
    pub info: CopyInfo,
}
pub(super) struct Worker {
    pub requests: SyncSender<Request>,
    pub responses: Receiver<Response>,
}

impl Worker {
    pub fn spawn(generation: Arc<AtomicU64>) -> std::io::Result<Self> {
        let (requests, jobs) = sync_channel::<Request>(2);
        let (results, responses) = sync_channel(2);
        std::thread::Builder::new()
            .name("duplicate-comparison".into())
            .spawn(move || {
                for request in jobs {
                    let stale = || generation.load(Ordering::Acquire) != request.generation;
                    if stale() {
                        continue;
                    }
                    let info = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        loader::load(&request.path, &request.member, stale)
                    }))
                    .unwrap_or_else(|_| CopyInfo::unavailable(CopyStatus::Inaccessible));
                    if stale() {
                        continue;
                    }
                    let response = Response {
                        generation: request.generation,
                        slot: request.slot,
                        idx: request.member.idx,
                        info,
                    };
                    if results.send(response).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            responses,
        })
    }
}
