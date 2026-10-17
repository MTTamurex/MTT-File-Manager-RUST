use super::*;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::WriteFile;

fn own_process() -> auth::Requester {
    auth::Requester::open(std::process::id()).unwrap()
}

#[test]
fn helper_parser_only_accepts_versioned_local_pipe_coordinates() {
    let request = protocol::HelperRequest::new().unwrap();
    let args = [
        "--ipc-v1".to_owned(),
        request.pipe_name.clone(),
        request.requester_pid.to_string(),
    ];
    let parsed = protocol::HelperRequest::parse(args.map(OsString::from)).unwrap();
    assert_eq!(parsed.pipe_name, request.pipe_name);
    assert_eq!(parsed.requester_pid, std::process::id());
    for args in [
        vec!["--ipc-v1", r"C:\Folder", "1"],
        vec![
            "--ipc-v1",
            r"\\server\pipe\MTT.FolderAccess.00000000000000000000000000000000",
            "1",
        ],
        vec!["--ipc-v1", &request.pipe_name, "0"],
        vec!["--ipc-v1", &request.pipe_name, "S-1-1-0"],
        vec!["--ipc-v1", &request.pipe_name, "1", "S-1-1-0"],
        vec!["--ipc-v2", &request.pipe_name, "1"],
    ] {
        assert!(protocol::HelperRequest::parse(args.into_iter().map(OsString::from)).is_err());
    }
}

#[test]
fn paths_preserve_roots_unc_long_names_and_non_lossy_utf16() {
    for text in [
        r"C:\",
        r"\\server\share\folder",
        r"\\?\C:\Folder Name\Pasta 日本語",
    ] {
        let path = Path::new(text);
        assert_eq!(
            protocol::decode_path(&protocol::encode_path(path).unwrap()).unwrap(),
            path
        );
    }
    let mut units: Vec<_> = OsStr::new(r"C:\Folder\").encode_wide().collect();
    units.push(0xd800);
    let path = std::path::PathBuf::from(OsString::from_wide(&units));
    assert_eq!(
        protocol::decode_path(&protocol::encode_path(&path).unwrap()).unwrap(),
        path
    );
    assert_eq!(protocol::decode_path(&[0xff]), Err(87));
    assert_eq!(protocol::decode_path(&[]), Err(87));
    assert_eq!(protocol::decode_path(&[0; 8]), Err(87));
    assert_eq!(
        protocol::decode_path(&vec![1; protocol::MAX_PATH_BYTES + 2]),
        Err(87)
    );
    assert_eq!(protocol::encode_path(Path::new("relative")), Err(87));
    assert_eq!(
        protocol::encode_path(Path::new("C:\\Folder\0name")),
        Err(87)
    );
}

#[test]
fn authenticated_pipe_uses_client_token_and_does_not_leave_impersonation_active() {
    let endpoint = protocol::HelperRequest::new().unwrap();
    let server = pipe::create_server(&endpoint.pipe_name).unwrap();
    let requester = own_process();
    let peer = own_process();
    let deadline = Instant::now() + Duration::from_secs(5);
    let expected_sid = current_user_sid().unwrap();
    let worker = std::thread::spawn(move || {
        pipe::wait_for_client(server.as_raw(), deadline, requester.process()).unwrap();
        requester.verify_client(server.as_raw()).unwrap();
        let bytes = pipe::receive_path(server.as_raw(), deadline, requester.process()).unwrap();
        let sid = requester.client_sid(server.as_raw()).unwrap();
        let mut token = HANDLE::default();
        let error = unsafe {
            windows::Win32::System::Threading::OpenThreadToken(
                windows::Win32::System::Threading::GetCurrentThread(),
                windows::Win32::Security::TOKEN_QUERY,
                true,
                &mut token,
            )
        }
        .unwrap_err();
        assert_eq!(os_error_code(error), 1008); // ERROR_NO_TOKEN after reverting.
        (protocol::decode_path(&bytes).unwrap(), sid)
    });
    let client = pipe::connect(&endpoint.pipe_name, deadline, peer.process()).unwrap();
    auth::verify_server(client.as_raw(), peer.process()).unwrap();
    let path = Path::new(r"C:\Folder Name\Pasta 日本語");
    pipe::send_path(
        client.as_raw(),
        &protocol::encode_path(path).unwrap(),
        deadline,
        peer.process(),
    )
    .unwrap();
    let (received, sid) = worker.join().unwrap();
    assert_eq!(received, path);
    assert!(sid == expected_sid, "client token identity mismatch");
}

#[test]
fn authenticated_channel_can_be_used_with_acl_grants_on_temporary_folders() {
    let directory = tempfile::tempdir().unwrap();
    let endpoint = protocol::HelperRequest::new().unwrap();
    let server = pipe::create_server(&endpoint.pipe_name).unwrap();
    let requester = own_process();
    let peer = own_process();
    let deadline = Instant::now() + Duration::from_secs(5);
    let worker = std::thread::spawn(move || {
        pipe::wait_for_client(server.as_raw(), deadline, requester.process())?;
        requester.verify_client(server.as_raw())?;
        let bytes = pipe::receive_path(server.as_raw(), deadline, requester.process())?;
        let sid = requester.client_sid(server.as_raw())?;
        acl::grant_access(&protocol::decode_path(&bytes)?, &sid)
    });
    let client = pipe::connect(&endpoint.pipe_name, deadline, peer.process()).unwrap();
    auth::verify_server(client.as_raw(), peer.process()).unwrap();
    pipe::send_path(
        client.as_raw(),
        &protocol::encode_path(directory.path()).unwrap(),
        deadline,
        peer.process(),
    )
    .unwrap();
    assert_eq!(worker.join().unwrap(), Ok(()));
    assert_eq!(verify_access(directory.path()), Ok(()));
}

#[test]
fn malformed_frames_are_rejected_before_a_path_can_be_used() {
    for size in [0, 1, protocol::MAX_PATH_BYTES as u32 + 2, u32::MAX] {
        let endpoint = protocol::HelperRequest::new().unwrap();
        let server = pipe::create_server(&endpoint.pipe_name).unwrap();
        let requester = own_process();
        let peer = own_process();
        let deadline = Instant::now() + Duration::from_secs(5);
        let worker = std::thread::spawn(move || {
            pipe::wait_for_client(server.as_raw(), deadline, requester.process()).unwrap();
            pipe::receive_path(server.as_raw(), deadline, requester.process())
        });
        let client = pipe::connect(&endpoint.pipe_name, deadline, peer.process()).unwrap();
        let mut written = 0;
        unsafe {
            WriteFile(
                client.as_raw(),
                Some(&size.to_le_bytes()),
                Some(&mut written),
                None,
            )
        }
        .unwrap();
        assert_eq!(written, 4);
        assert_eq!(worker.join().unwrap(), Err(87));
    }
}

#[test]
fn pipe_squatting_and_missing_clients_fail_closed() {
    let endpoint = protocol::HelperRequest::new().unwrap();
    let server = pipe::create_server(&endpoint.pipe_name).unwrap();
    assert!(pipe::create_server(&endpoint.pipe_name).is_err());
    let peer = own_process();
    assert_eq!(
        pipe::wait_for_client(server.as_raw(), Instant::now(), peer.process()),
        Err(1460)
    );
    assert_eq!(
        pipe::receive_path(server.as_raw(), Instant::now(), peer.process()),
        Err(1460)
    );
}

#[test]
fn foreign_executable_and_unlaunched_pipe_server_are_rejected() {
    use std::process::{Command, Stdio};
    // This child only waits for EOF on its redirected stdin; it creates no files.
    let system = std::env::var_os("SystemRoot").unwrap();
    let mut child = Command::new(
        std::path::Path::new(&system)
            .join("System32")
            .join("cmd.exe"),
    )
    .args(["/d", "/c", "set /p value="])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    assert!(auth::Requester::open(child.id()).is_err());
    let endpoint = protocol::HelperRequest::new().unwrap();
    let server = pipe::create_server(&endpoint.pipe_name).unwrap();
    let peer = own_process();
    let deadline = Instant::now() + Duration::from_secs(5);
    let client = pipe::connect(&endpoint.pipe_name, deadline, peer.process()).unwrap();
    assert_eq!(
        auth::verify_server(client.as_raw(), HANDLE(child.as_raw_handle())),
        Err(5)
    );
    drop(server);
    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn process_identity_fixture() {
    use std::io::Write;
    let Some(name) = std::env::var_os("MTT_FOLDER_ACCESS_TEST_PIPE") else {
        return;
    };
    let pid = std::env::var("MTT_FOLDER_ACCESS_TEST_PID")
        .unwrap()
        .parse()
        .unwrap();
    let requester = auth::Requester::open(pid).unwrap();
    let server = pipe::create_server(name.to_str().unwrap()).unwrap();
    println!("folder access fixture ready");
    std::io::stdout().flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    pipe::wait_for_client(server.as_raw(), deadline, requester.process()).unwrap();
    requester.verify_client(server.as_raw()).unwrap();
    let bytes = pipe::receive_path(server.as_raw(), deadline, requester.process()).unwrap();
    let sid = requester.client_sid(server.as_raw()).unwrap();
    assert!(
        sid == current_user_sid().unwrap(),
        "cross-process token identity mismatch"
    );
    assert_eq!(protocol::decode_path(&bytes).unwrap(), Path::new(r"C:\"));
}

#[test]
fn cross_process_handshake_rejects_wrong_client_pid_even_for_the_same_executable() {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let endpoint = protocol::HelperRequest::new().unwrap();
    let test_name = format!(
        "{}::process_identity_fixture",
        module_path!().split_once("::").unwrap().1
    );
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test_name, "--nocapture"])
        .env("MTT_FOLDER_ACCESS_TEST_PIPE", &endpoint.pipe_name)
        .env("MTT_FOLDER_ACCESS_TEST_PID", std::process::id().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert!(
            output.read_line(&mut line).unwrap() != 0,
            "fixture exited before listening"
        );
        if line.contains("folder access fixture ready") {
            break;
        }
    }
    let expected_other_client = auth::Requester::open(child.id()).unwrap();
    let fake = protocol::HelperRequest::new().unwrap();
    let fake_server = pipe::create_server(&fake.pipe_name).unwrap();
    let own = own_process();
    let deadline = Instant::now() + Duration::from_secs(5);
    let _fake_client = pipe::connect(&fake.pipe_name, deadline, own.process()).unwrap();
    assert_eq!(
        expected_other_client.verify_client(fake_server.as_raw()),
        Err(5)
    );

    let child_handle = HANDLE(child.as_raw_handle());
    let client = pipe::connect(&endpoint.pipe_name, deadline, child_handle).unwrap();
    auth::verify_server(client.as_raw(), child_handle).unwrap();
    pipe::send_path(
        client.as_raw(),
        &protocol::encode_path(Path::new(r"C:\")).unwrap(),
        deadline,
        child_handle,
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
}
