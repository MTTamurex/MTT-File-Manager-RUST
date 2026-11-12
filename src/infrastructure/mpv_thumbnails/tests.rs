//! Fault injection uses a private mock peer, never production helper switches.
mod lifecycle;
use super::*;
use crate::infrastructure::windows::OwnedHandle;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
};

fn peer(script: &str) -> Command {
    let executable =
        PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system directory"))
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
    let mut command = Command::new(executable);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]);
    command
}

fn echo_peer() -> Command {
    peer(
        r#"
        $ErrorActionPreference = 'Stop'
        $source = [Console]::OpenStandardInput()
        $sink = [Console]::OpenStandardOutput()
        function Read-Exact([int]$count) {
            $bytes = New-Object byte[] $count
            $offset = 0
            while ($offset -lt $count) {
                $read = $source.Read($bytes, $offset, $count - $offset)
                if ($read -eq 0) { exit 0 }
                $offset += $read
            }
            return ,$bytes
        }
        while ($true) {
            $header = Read-Exact 28
            $null = Read-Exact ([BitConverter]::ToUInt32($header, 24))
            $reply = New-Object byte[] 36
            [Array]::Copy($header, 0, $reply, 0, 16)
            [BitConverter]::GetBytes([uint32]1).CopyTo($reply, 20)
            [BitConverter]::GetBytes([uint32]1).CopyTo($reply, 24)
            [BitConverter]::GetBytes([uint32]4).CopyTo($reply, 28)
            $reply[32] = 1; $reply[33] = 2; $reply[34] = 3; $reply[35] = 255
            $sink.Write($reply, 0, $reply.Length)
            $sink.Flush()
        }
    "#,
    )
}

fn fixture() -> &'static Path {
    Path::new(r"C:\Videos\clip.mp4")
}

#[test]
fn cancelled_pipeline_request_returns_before_source_io() {
    use crate::{
        infrastructure::io_priority::IOPriority,
        workers::thumbnail::extraction::{
            generate_thumbnail_hybrid_cancellable, ThumbnailExtractionOutcome,
        },
    };
    let pending = dashmap::DashMap::new();
    let outcome = generate_thumbnail_hybrid_cancellable(
        fixture(),
        IOPriority::Background,
        &pending,
        Some(256),
        &|| true,
    );
    assert!(matches!(outcome, ThumbnailExtractionOutcome::Cancelled));
}

fn extractor() -> Extractor {
    // No production spawn is needed: tests inject a contained peer before use.
    Extractor::new(std::env::current_exe().unwrap())
}

fn process_handle(extractor: &Extractor) -> OwnedHandle {
    OwnedHandle::new(
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, extractor.process_id().unwrap()) }
            .unwrap(),
    )
    .unwrap()
}

fn assert_exited(handle: &OwnedHandle) {
    assert_eq!(
        unsafe { WaitForSingleObject(handle.as_raw(), 2000) },
        WAIT_OBJECT_0
    );
}

#[test]
fn healthy_peer_is_reused_and_drop_reaps_it() {
    let mut extractor = extractor();
    extractor.install_test_process(echo_peer());
    let id = extractor.process_id();
    let handle = process_handle(&extractor);
    for _ in 0..5 {
        let frame = extractor.extract(fixture(), 256, &|| false).unwrap();
        assert_eq!(frame.rgba, [1, 2, 3, 255]);
        assert_eq!(extractor.process_id(), id);
    }
    drop(extractor);
    assert_exited(&handle);
}

#[test]
fn timeout_interrupts_blocked_pipe_io_and_reaps_peer() {
    let mut extractor = extractor();
    extractor.install_test_process(peer("Start-Sleep -Seconds 60"));
    let handle = process_handle(&extractor);
    let start = Instant::now();
    assert_eq!(
        extractor
            .extract_until(
                fixture(),
                256,
                &|| false,
                start + Duration::from_millis(200)
            )
            .unwrap_err(),
        Error::Timeout
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(extractor.process_id().is_none());
    assert_exited(&handle);
}

#[test]
fn cancellation_interrupts_inflight_request_without_marking_decode_failure() {
    let mut extractor = extractor();
    extractor.install_test_process(peer("Start-Sleep -Seconds 60"));
    let handle = process_handle(&extractor);
    let start = Instant::now();
    let cancelled = || start.elapsed() >= Duration::from_millis(200);
    assert_eq!(
        extractor.extract(fixture(), 256, &cancelled).unwrap_err(),
        Error::Cancelled
    );
    assert_exited(&handle);
}

#[test]
fn shutdown_closes_job_without_waiting_for_client_lock() {
    let mut extractor = extractor();
    extractor.install_test_process(peer("Start-Sleep -Seconds 60"));
    let handle = process_handle(&extractor);
    let control = extractor.control.clone();
    let shutdown = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        control.shutdown();
    });
    assert_eq!(
        extractor.extract(fixture(), 256, &|| false).unwrap_err(),
        Error::Cancelled
    );
    shutdown.join().unwrap();
    assert_exited(&handle);
    assert_eq!(
        extractor.extract(fixture(), 256, &|| false).unwrap_err(),
        Error::Cancelled
    );
}

#[test]
fn crash_discards_peer_and_next_request_can_use_a_new_process() {
    let mut extractor = extractor();
    extractor.install_test_process(peer("exit 7"));
    let handle = process_handle(&extractor);
    assert_eq!(
        extractor.extract(fixture(), 256, &|| false).unwrap_err(),
        Error::Transport
    );
    assert_exited(&handle);
    assert!(extractor.process_id().is_none());
    extractor.install_test_process(echo_peer());
    assert!(extractor.extract(fixture(), 256, &|| false).is_ok());
}

#[test]
fn malformed_reply_is_rejected_and_peer_is_killed() {
    let mut extractor = extractor();
    extractor.install_test_process(peer(
        r#"
        $outputStream = [Console]::OpenStandardOutput()
        $bytes = New-Object byte[] 32
        $outputStream.Write($bytes, 0, $bytes.Length)
        $outputStream.Flush()
        Start-Sleep -Seconds 60
    "#,
    ));
    let handle = process_handle(&extractor);
    assert_eq!(
        extractor.extract(fixture(), 256, &|| false).unwrap_err(),
        Error::Transport
    );
    assert_exited(&handle);
}

#[test]
fn oversized_request_cannot_block_pipe_writer_past_deadline() {
    let mut extractor = extractor();
    extractor.install_test_process(peer("Start-Sleep -Seconds 60"));
    let handle = process_handle(&extractor);
    // Fits the IPC path bound but not the pipe buffer; peer never reads stdin.
    let path = PathBuf::from(format!("C:\\Videos\\{}.mp4", "a".repeat(60_000)));
    let start = Instant::now();
    assert_eq!(
        extractor
            .extract_until(&path, 256, &|| false, start + Duration::from_millis(200))
            .unwrap_err(),
        Error::Timeout
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_exited(&handle);
}

#[test]
fn queued_timeout_and_cancellation_do_not_discard_an_inflight_helper() {
    let mut client = extractor();
    client.install_test_process(echo_peer());
    client.extract(fixture(), 256, &|| false).unwrap();
    let pid = client.process_id();
    let handle = process_handle(&client);
    let service = Service::new(client);
    // Represent the in-flight owner as well as holding its decoder lock.
    let active = service.register();
    let owner = service.client.lock();
    assert_eq!(
        service
            .extract_until(
                fixture(),
                256,
                &|| false,
                Instant::now() + Duration::from_millis(50)
            )
            .unwrap_err(),
        Error::QueueTimeout
    );
    assert_eq!(
        service
            .extract_until(
                fixture(),
                256,
                &|| true,
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap_err(),
        Error::Cancelled
    );
    drop(owner);
    assert_eq!(service.client.lock().process_id(), pid);
    assert!(service
        .extract_until(fixture(), 256, &|| false, Instant::now() + REQUEST_TIMEOUT)
        .is_ok());
    drop(active);
    assert!(service.client.lock().process_id().is_none());
    assert_exited(&handle);
}
