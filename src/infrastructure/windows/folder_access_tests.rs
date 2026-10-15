use super::*;

#[test]
fn arguments_preserve_trailing_slashes_spaces_and_unicode() {
    for (input, expected) in [
        ("C:\\", "\"C:\\\\\""),
        ("C:\\Folder Name\\", "\"C:\\Folder Name\\\\\""),
        ("C:\\Pasta 日本語", "\"C:\\Pasta 日本語\""),
    ] {
        assert_eq!(
            quoted_argument(std::ffi::OsStr::new(input)),
            expected.encode_utf16().collect::<Vec<_>>()
        );
    }
}

#[test]
fn helper_rejects_relative_paths_without_changing_permissions() {
    assert_eq!(run_helper(Path::new("folder"), "S-1-5-18"), 87);
}

#[test]
fn grant_preserves_other_trustees_owner_and_protected_inheritance() {
    use windows::Win32::Security::{
        GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, ACE_HEADER,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
    };

    // Only a newly created temporary folder is modified; no elevation needed.
    let directory = tempfile::tempdir().unwrap();
    let name: Vec<u16> = directory
        .path()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let sid_text = current_user_sid().unwrap();
    let user: Vec<u16> = sid_text.encode_utf16().chain(Some(0)).collect();
    let system: Vec<u16> = "S-1-5-18\0".encode_utf16().collect();
    unsafe {
        let mut user_sid = PSID::default();
        let mut system_sid = PSID::default();
        ConvertStringSidToSidW(PCWSTR(user.as_ptr()), &mut user_sid).unwrap();
        let _user = LocalAllocation(user_sid.0);
        ConvertStringSidToSidW(PCWSTR(system.as_ptr()), &mut system_sid).unwrap();
        let _system = LocalAllocation(system_sid.0);
        let entries = [user_sid, system_sid].map(|sid| EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS.0,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: windows::Win32::Security::ACE_FLAGS(0),
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: PWSTR(sid.0.cast()),
                ..Default::default()
            },
        });
        let mut initial_acl = std::ptr::null_mut();
        assert_eq!(
            SetEntriesInAclW(Some(&entries), None, &mut initial_acl).0,
            0
        );
        let _initial_acl = LocalAllocation(initial_acl.cast());
        assert_eq!(
            SetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(initial_acl),
                None,
            )
            .0,
            0
        );

        // Capture the owner and the unrelated trustee's ACE before the grant.
        let snapshot = || {
            let mut acl = std::ptr::null_mut();
            let mut owner = PSID::default();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            assert_eq!(
                GetNamedSecurityInfoW(
                    PCWSTR(name.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                    Some(&mut owner),
                    None,
                    Some(&mut acl),
                    None,
                    &mut descriptor,
                )
                .0,
                0
            );
            let _descriptor = LocalAllocation(descriptor.0);
            let mut owner_text = PWSTR::null();
            ConvertSidToStringSidW(owner, &mut owner_text).unwrap();
            let _owner = LocalAllocation(owner_text.0.cast());
            let owner = owner_text.to_string().unwrap();
            let mut control = 0;
            let mut revision = 0;
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).unwrap();
            let mut system_ace = Vec::new();
            let mut inherited_user_grant = false;
            for index in 0..(*acl).AceCount {
                let mut ace = std::ptr::null_mut();
                GetAce(acl, index as u32, &mut ace).unwrap();
                let header = &*ace.cast::<ACE_HEADER>();
                let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                let sid = PSID(std::ptr::addr_of!(allowed.SidStart).cast_mut().cast());
                if windows::Win32::Security::EqualSid(sid, system_sid).is_ok() {
                    system_ace =
                        std::slice::from_raw_parts(ace.cast::<u8>(), header.AceSize as usize)
                            .to_vec();
                }
                if windows::Win32::Security::EqualSid(sid, user_sid).is_ok()
                    && allowed.Mask == FILE_ALL_ACCESS.0
                    && header.AceFlags & 3 == 3
                {
                    inherited_user_grant = true;
                }
            }
            (
                owner,
                control & SE_DACL_PROTECTED.0,
                system_ace,
                inherited_user_grant,
                (*acl).AceCount,
            )
        };
        let before = snapshot();
        assert!(!before.2.is_empty());
        grant_access(directory.path(), &sid_text).unwrap();
        let after = snapshot();
        assert_eq!(before.0, after.0);
        assert_eq!(after.1, SE_DACL_PROTECTED.0);
        assert_eq!(before.2, after.2);
        assert!(after.3);
        // Repeated attempts must not accumulate duplicate entries.
        grant_access(directory.path(), &sid_text).unwrap();
        assert_eq!(after, snapshot());
    }
}
