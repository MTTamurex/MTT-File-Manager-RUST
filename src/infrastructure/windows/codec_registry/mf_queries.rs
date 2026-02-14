use windows::core::GUID;

/// Query Media Foundation Transform Registry for codec friendly name
///
/// Uses MFTEnumEx to enumerate transforms and extract friendly names from IMFAttributes.
/// This is the preferred method as it works with both installed and system codecs.
pub(super) fn query_mf_codec_name(guid: &GUID) -> Option<String> {
    use windows::Win32::Media::MediaFoundation::{
        IMFActivate, MFMediaType_Audio, MFMediaType_Video, MFTEnumEx, MFT_CATEGORY_AUDIO_DECODER,
        MFT_CATEGORY_AUDIO_ENCODER, MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER,
        MFT_ENUM_FLAG, MFT_REGISTER_TYPE_INFO, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    };
    use windows::Win32::System::Com::CoTaskMemFree;

    // Convert GUID to tag format used by Media Foundation
    log::trace!(
        "[CODEC DEBUG] Querying MF codec name for GUID: {{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1, guid.data2, guid.data3,
        guid.data4[0], guid.data4[1], guid.data4[2], guid.data4[3],
        guid.data4[4], guid.data4[5], guid.data4[6], guid.data4[7]
    );

    unsafe {
        // Try both audio and video categories, both decoders and encoders
        for media_type in [MFMediaType_Audio, MFMediaType_Video] {
            for category in [
                MFT_CATEGORY_AUDIO_DECODER,
                MFT_CATEGORY_AUDIO_ENCODER,
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_CATEGORY_VIDEO_ENCODER,
            ] {
                // Skip mismatched category/media type combinations
                if media_type == MFMediaType_Audio
                    && (category == MFT_CATEGORY_VIDEO_DECODER
                        || category == MFT_CATEGORY_VIDEO_ENCODER)
                {
                    continue;
                }
                if media_type == MFMediaType_Video
                    && (category == MFT_CATEGORY_AUDIO_DECODER
                        || category == MFT_CATEGORY_AUDIO_ENCODER)
                {
                    continue;
                }

                for use_input in [false, true] {
                    let type_info = MFT_REGISTER_TYPE_INFO {
                        guidMajorType: media_type,
                        guidSubtype: *guid,
                    };

                    let (input_type, output_type) = if use_input {
                        (Some(&type_info as *const _), None)
                    } else {
                        (None, Some(&type_info as *const _))
                    };

                    let mut activate_array: *mut Option<IMFActivate> = std::ptr::null_mut();
                    let mut count: u32 = 0;

                    let result = MFTEnumEx(
                        category,
                        MFT_ENUM_FLAG(0),
                        input_type,
                        output_type,
                        &mut activate_array,
                        &mut count,
                    );

                    if result.is_ok() && count > 0 {
                        log::trace!(
                            "[CODEC DEBUG] Found {} MFTs for codec (cat={:?}, media_type={:?})",
                            count,
                            category,
                            media_type
                        );

                        // Get friendly name from first transform
                        if let Some(Some(act)) = activate_array.as_ref() {
                            use windows::core::PWSTR;
                            let mut friendly_name_ptr = PWSTR::null();
                            let mut length: u32 = 0;

                            if act
                                .GetAllocatedString(
                                    &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                                    &mut friendly_name_ptr,
                                    &mut length,
                                )
                                .is_ok()
                                && !friendly_name_ptr.is_null()
                            {
                                let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                                    friendly_name_ptr.as_ptr(),
                                    length as usize,
                                ));
                                CoTaskMemFree(Some(friendly_name_ptr.as_ptr() as *const _));

                                // Cleanup activate array
                                for i in 0..count {
                                    if let Some(Some(act)) = activate_array.add(i as usize).as_ref()
                                    {
                                        let _ = act.ShutdownObject();
                                    }
                                }
                                CoTaskMemFree(Some(activate_array as *const _));

                                return Some(name);
                            }
                        }

                        // Cleanup if name extraction failed
                        for i in 0..count {
                            if let Some(Some(act)) = activate_array.add(i as usize).as_ref() {
                                let _ = act.ShutdownObject();
                            }
                        }
                        CoTaskMemFree(Some(activate_array as *const _));
                    }
                }
            }
        }
    }

    None
}

/// Query Media Foundation Transform by subtype using MFTEnumEx
pub(super) fn query_mft_by_subtype(tag: u32) -> Option<String> {
    use windows::Win32::Media::MediaFoundation::{
        IMFActivate, MFMediaType_Audio, MFMediaType_Video, MFTEnumEx, MFT_CATEGORY_AUDIO_DECODER,
        MFT_CATEGORY_AUDIO_ENCODER, MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER,
        MFT_ENUM_FLAG, MFT_REGISTER_TYPE_INFO, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    };
    use windows::Win32::System::Com::CoTaskMemFree;

    // Convert tag to GUID (partial GUID format used by Media Foundation)
    let guid = GUID {
        data1: tag,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
    };

    log::trace!(
        "[CODEC DEBUG] Searching MFT with GUID: {{{:08X}-0000-0010-8000-00AA00389B71}}",
        tag
    );

    unsafe {
        // Try both audio and video categories, both decoders and encoders
        for media_type in [MFMediaType_Audio, MFMediaType_Video] {
            for category in [
                MFT_CATEGORY_AUDIO_DECODER,
                MFT_CATEGORY_AUDIO_ENCODER,
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_CATEGORY_VIDEO_ENCODER,
            ] {
                // Skip mismatched category/media type combinations
                if media_type == MFMediaType_Audio
                    && (category == MFT_CATEGORY_VIDEO_DECODER
                        || category == MFT_CATEGORY_VIDEO_ENCODER)
                {
                    continue;
                }
                if media_type == MFMediaType_Video
                    && (category == MFT_CATEGORY_AUDIO_DECODER
                        || category == MFT_CATEGORY_AUDIO_ENCODER)
                {
                    continue;
                }

                for use_input in [false, true] {
                    let type_info = MFT_REGISTER_TYPE_INFO {
                        guidMajorType: media_type,
                        guidSubtype: guid,
                    };

                    let (input_type, output_type) = if use_input {
                        (Some(&type_info as *const _), None)
                    } else {
                        (None, Some(&type_info as *const _))
                    };

                    let mut activate_array: *mut Option<IMFActivate> = std::ptr::null_mut();
                    let mut count: u32 = 0;

                    let result = MFTEnumEx(
                        category,
                        MFT_ENUM_FLAG(0),
                        input_type,
                        output_type,
                        &mut activate_array,
                        &mut count,
                    );

                    if result.is_ok() && count > 0 {
                        log::trace!(
                            "[CODEC DEBUG] Found {} MFTs (input={}, cat={:?}, media_type={:?})",
                            count,
                            use_input,
                            category,
                            media_type
                        );

                        // Get friendly name from first transform
                        if let Some(Some(act)) = activate_array.as_ref() {
                            use windows::core::PWSTR;
                            let mut friendly_name_ptr = PWSTR::null();
                            let mut length: u32 = 0;

                            if act
                                .GetAllocatedString(
                                    &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                                    &mut friendly_name_ptr,
                                    &mut length,
                                )
                                .is_ok()
                                && !friendly_name_ptr.is_null()
                            {
                                let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                                    friendly_name_ptr.as_ptr(),
                                    length as usize,
                                ));
                                CoTaskMemFree(Some(friendly_name_ptr.as_ptr() as *const _));

                                // Cleanup activate array
                                for i in 0..count {
                                    if let Some(Some(act)) = activate_array.add(i as usize).as_ref()
                                    {
                                        let _ = act.ShutdownObject();
                                    }
                                }
                                CoTaskMemFree(Some(activate_array as *const _));

                                return Some(name);
                            }
                        }

                        // Cleanup if name extraction failed
                        for i in 0..count {
                            if let Some(Some(act)) = activate_array.add(i as usize).as_ref() {
                                let _ = act.ShutdownObject();
                            }
                        }
                        CoTaskMemFree(Some(activate_array as *const _));
                    }
                }
            }
        }
    }

    None
}
