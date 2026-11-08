use super::protocol::{self, Failure, Frame, Request, Response};
use crate::infrastructure::{
    io_priority::{IOPriority, ThreadPriorityGuard},
    windows::{
        file_flags::{classify_file_read_safety, FileReadSafety},
        file_type::is_video_extension,
    },
};
use mtt_mpv_thumbnail::{Api, Session};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

pub fn run_helper() -> i32 {
    use windows::Win32::System::Diagnostics::Debug::{
        SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
    };
    // Native failures must return to the supervisor, not open a modal error box.
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
    let _priority = ThreadPriorityGuard::new(IOPriority::Background);
    match serve(&mut io::stdin().lock(), &mut io::stdout().lock()) {
        Ok(()) => 0,
        Err(_) => 1, // No paths, titles, native diagnostic strings, or UI logging.
    }
}

fn serve(input: &mut impl io::Read, output: &mut impl io::Write) -> io::Result<()> {
    // Parent assigns the Job Object before sending this first request. If the
    // parent exits during spawn, EOF terminates us without initializing libmpv.
    let Some(mut request) = protocol::read_request(input)? else {
        return Ok(());
    };
    let mut request_start = Instant::now();
    let dll = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("No executable directory"))?
        .join("libmpv-2.dll");
    let Ok(api) = Api::load(&dll) else {
        protocol::write_response(output, request.id, &Err(Failure::Unavailable))?;
        return Ok(());
    };
    let Ok(mut session) = Session::new(&api) else {
        protocol::write_response(output, request.id, &Err(Failure::Unavailable))?;
        return Ok(());
    };
    loop {
        let response = extract(&mut session, &request, request_start);
        let failed = response.is_err();
        protocol::write_response(output, request.id, &response)?;
        // A failed native session is not reused. Parent restarts on next request.
        if failed {
            return Ok(());
        }
        let Some(next) = protocol::read_request(input)? else {
            return Ok(());
        };
        request = next;
        request_start = Instant::now();
    }
}

fn extract(session: &mut Session<'_>, request: &Request, start: Instant) -> Response {
    let path = Path::new(&request.path);
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(is_video_extension)
    {
        return Err(Failure::Decode);
    }
    // Re-check in the child: the source may have become unsafe while queued.
    match classify_file_read_safety(path) {
        FileReadSafety::Safe => {}
        FileReadSafety::IncompleteDownload => return Err(Failure::IncompleteDownload),
        FileReadSafety::WriteLocked => return Err(Failure::WriteLocked),
        FileReadSafety::RecentlyChanging => return Err(Failure::RecentlyChanging),
    }
    // Account for initialization/safety probes as well as the parent's queue
    // wait. Optional quality retries may only spend the remaining request budget.
    let budget = Duration::from_millis(request.budget_ms as u64);
    let remaining = budget
        .checked_sub(start.elapsed())
        .filter(|left| !left.is_zero())
        .ok_or(Failure::Timeout)?;
    let capture = session
        .capture(path, None, request.max_side, remaining)
        .map_err(|_| {
            if start.elapsed() + Duration::from_millis(200) >= budget {
                Failure::Timeout
            } else {
                Failure::Decode
            }
        })?;
    Ok(Frame {
        width: capture.image.width(),
        height: capture.image.height(),
        rgba: capture.image.into_raw(),
    })
}
