local mp = require 'mp'

-- Configuration
local autovsr_enabled = false -- Default to VSR disabled
local autohdr_enabled = false -- Default to HDR disabled (controlled by shared RTX toggle)
local VSR_MAX_LONG_SIDE = 2560
local VSR_MAX_SHORT_SIDE = 1440
local VSR_MIN_SCALE_FHD_OR_LESS = 1.5
local VSR_MIN_SCALE_ABOVE_FHD = 1.2
local VSR_MAX_SCALE_FHD_OR_LESS = 2.0
local VSR_MAX_SCALE_ABOVE_FHD = 1.5

local state = {
    applying_filters = false,
    observers_registered = false,
}

local function publish_state(active, hdr_active, vsr_active, hdr_supported, vsr_supported)
    mp.set_property_bool("user-data/vsr/vsr-enabled", autovsr_enabled)
    mp.set_property_bool("user-data/rtx/enabled", autovsr_enabled or autohdr_enabled)
    mp.set_property_bool("user-data/rtx/active", active)
    mp.set_property_bool("user-data/rtx/hdr-enabled", autohdr_enabled)
    mp.set_property_bool("user-data/rtx/hdr-active", hdr_active)
    mp.set_property_bool("user-data/rtx/hdr-supported", hdr_supported)
    mp.set_property_bool("user-data/rtx/vsr-enabled", autovsr_enabled)
    mp.set_property_bool("user-data/rtx/vsr-active", vsr_active)
    mp.set_property_bool("user-data/rtx/vsr-supported", vsr_supported)
end

local function is_hdr_source()
    local gamma = (mp.get_property("video-params/gamma", "") or ""):lower()
    return gamma == "pq" or gamma == "hlg"
end

local function is_vsr_resolution_supported(video_width, video_height)
    if not (video_width and video_height) then
        return false
    end

    local long_side = math.max(video_width, video_height)
    local short_side = math.min(video_width, video_height)
    return long_side <= VSR_MAX_LONG_SIDE and short_side <= VSR_MAX_SHORT_SIDE
end

local function compute_vsr_scale(video_width, video_height, target_width, target_height)
    if not (video_width and video_height and target_width and target_height) then
        return nil
    end

    local long_side = math.max(video_width, video_height)
    local upscale = math.max(target_width / video_width, target_height / video_height)
    local min_scale = (long_side > 1920) and VSR_MIN_SCALE_ABOVE_FHD or VSR_MIN_SCALE_FHD_OR_LESS
    local base_scale = math.max(upscale, min_scale)
    local perf_cap = (long_side > 1920) and VSR_MAX_SCALE_ABOVE_FHD or VSR_MAX_SCALE_FHD_OR_LESS
    local clamped = math.min(base_scale, perf_cap)

    return math.floor(clamped * 10 + 0.5) / 10
end

local function remove_managed_filters()
    pcall(mp.commandv, "vf", "remove", "@format-nv12")
    pcall(mp.commandv, "vf", "remove", "@rtx-video")
end

local function are_video_properties_ready()
    local display_width = mp.get_property_number("display-width")
    local display_height = mp.get_property_number("display-height")
    local osd_width = mp.get_property_number("osd-width")
    local osd_height = mp.get_property_number("osd-height")
    local video_width = mp.get_property_number("width")
    local video_height = mp.get_property_number("height")
    local codec = mp.get_property("video-codec", "")
    local pixelformat = mp.get_property("video-params/pixelformat", "")
    local hwdec = mp.get_property("hwdec-current", "")

    local target_width = (osd_width and osd_width > 0) and osd_width or display_width
    local target_height = (osd_height and osd_height > 0) and osd_height or display_height

    return video_width and video_width > 0
        and video_height and video_height > 0
        and target_width and target_width > 0
        and target_height and target_height > 0
        and codec ~= nil and codec ~= ""
        and pixelformat ~= nil and pixelformat ~= ""
        and hwdec ~= nil and hwdec ~= ""
end

local function apply_filters()
    if state.applying_filters then
        return false, "busy"
    end

    local video_width = mp.get_property_number("width")
    local video_height = mp.get_property_number("height")
    local display_width = mp.get_property_number("display-width")
    local display_height = mp.get_property_number("display-height")
    local osd_width = mp.get_property_number("osd-width")
    local osd_height = mp.get_property_number("osd-height")
    -- Account for Windows DPI scaling: osd-*/display-* are logical pixels,
    -- multiply by hidpi scale to get physical pixels (e.g. 4K @ 225% = 1707 -> 3840)
    local hidpi = mp.get_property_number("display-hidpi-scale", 1.0)
    local codec = mp.get_property("video-codec", "")
    local pixelformat = mp.get_property("video-params/pixelformat", "")

    local want_hdr = autohdr_enabled and not is_hdr_source()

    local raw_w = (osd_width and osd_width > 0) and osd_width or display_width
    local raw_h = (osd_height and osd_height > 0) and osd_height or display_height
    local target_width = raw_w and math.floor(raw_w * hidpi) or nil
    local target_height = raw_h and math.floor(raw_h * hidpi) or nil

    if not (video_width and video_height and target_width and target_height and codec and pixelformat) then
        mp.msg.debug("RTX: Missing video properties, retrying...")
        publish_state(false, false, false, false, false)
        return false, "missing-properties"
    end

    local hdr_supported = not is_hdr_source()
    local vsr_scale = nil
    local vsr_supported = is_vsr_resolution_supported(video_width, video_height)
    if autovsr_enabled and vsr_supported then
        vsr_scale = compute_vsr_scale(video_width, video_height, target_width, target_height)
    end
    local want_vsr = vsr_scale ~= nil

    if not want_hdr and not want_vsr then
        state.applying_filters = true
        remove_managed_filters()
        state.applying_filters = false
        publish_state(false, false, false, hdr_supported, vsr_supported)
        return false, "disabled"
    end

    local filter_parts = {}
    local need_nv12 = false

    if want_vsr then
        table.insert(filter_parts, "scaling-mode=nvidia")
        table.insert(filter_parts, "scale=" .. vsr_scale)

        if codec:lower():match("hevc") or codec:lower():match("h%.265") then
            if pixelformat:match("p10le$") or pixelformat == "p010" then
                need_nv12 = true
            end
        end
    end

    if want_hdr then
        table.insert(filter_parts, "nvidia-true-hdr")
    end

    state.applying_filters = true
    remove_managed_filters()

    if need_nv12 then
        local nv12_ok, nv12_err = pcall(mp.commandv, "vf", "append", "@format-nv12:format=nv12")
        if not nv12_ok then
            mp.msg.warn("RTX: nv12 format filter failed: " .. tostring(nv12_err))
        end
    end

    local filter_str = "@rtx-video:d3d11vpp=" .. table.concat(filter_parts, ":")
    local ok, err = pcall(mp.commandv, "vf", "append", filter_str)
    state.applying_filters = false

    if not ok then
        mp.msg.warn("RTX filter append failed: " .. tostring(err))
        publish_state(false, false, false, hdr_supported, vsr_supported)
        return false, "append-failed"
    end

    local vf_chain = mp.get_property("vf", "")
    if vf_chain:find("@rtx%-video") then
        publish_state(true, want_hdr, want_vsr, hdr_supported, vsr_supported)
        return true, "applied"
    end

    mp.msg.warn("RTX filter not in filter chain after append")
    publish_state(false, false, false, hdr_supported, vsr_supported)
    return false, "not-in-chain"
end

local function show_vsr_status()
    local enabled = mp.get_property_bool("user-data/rtx/vsr-enabled", false)
    local active = mp.get_property_bool("user-data/rtx/vsr-active", false)
    local supported = mp.get_property_bool("user-data/rtx/vsr-supported", false)
    local suffix = ""

    if enabled and not supported then
        suffix = " (max 1440p)"
    end

    mp.osd_message("RTX VSR: " .. (active and "ON" or "OFF") .. suffix, 2)
end

local function show_hdr_status()
    local active = mp.get_property_bool("user-data/rtx/hdr-active", false)
    local enabled = mp.get_property_bool("user-data/rtx/hdr-enabled", false)
    local supported = mp.get_property_bool("user-data/rtx/hdr-supported", false)
    local suffix = ""

    if enabled and not supported then
        suffix = " (source HDR)"
    end

    mp.osd_message("RTX HDR: " .. (active and "ON" or "OFF") .. suffix, 2)
end

local function toggle_vsr()
    autovsr_enabled = not autovsr_enabled
    publish_state(false, false, false, false, false)

    local result, reason = apply_filters()

    -- On first play the D3D11 pipeline may not be fully ready; retry once
    if autovsr_enabled and not result
        and reason ~= "disabled" and reason ~= "missing-properties" and reason ~= "busy" then
        mp.add_timeout(0.5, function()
            apply_filters()
            show_vsr_status()
        end)
    end

    show_vsr_status()
end

local function toggle_hdr()
    autohdr_enabled = not autohdr_enabled
    publish_state(false, false, false, false, false)

    local result, reason = apply_filters()

    if autohdr_enabled and not result
        and reason ~= "disabled" and reason ~= "missing-properties" and reason ~= "busy" then
        mp.add_timeout(0.5, function()
            apply_filters()
            show_hdr_status()
        end)
    end

    show_hdr_status()
end

local function toggle_rtx()
    local next_enabled = not (autovsr_enabled or autohdr_enabled)
    autovsr_enabled = next_enabled
    autohdr_enabled = next_enabled
    publish_state(false, false, false, false, false)
    apply_filters()

    if next_enabled then
        show_hdr_status()
        show_vsr_status()
    else
        mp.osd_message("RTX HDR: OFF | RTX VSR: OFF", 2)
    end
end

local function on_relevant_change()
    if not state.applying_filters and are_video_properties_ready() then
        apply_filters()
    end
end

local function ensure_observers_registered()
    if state.observers_registered then
        return
    end

    state.observers_registered = true
    mp.observe_property("video-params", "native", on_relevant_change)
    mp.observe_property("fullscreen", "bool", on_relevant_change)
    mp.observe_property("osd-width", "number", on_relevant_change)
    mp.observe_property("osd-height", "number", on_relevant_change)
    mp.observe_property("display-hidpi-scale", "number", on_relevant_change)
end

local function schedule_apply_with_retry()
    local retries = 0
    local max_retries = 15

    local function try_apply()
        local dw = mp.get_property_number("display-width")
        local dh = mp.get_property_number("display-height")
        local ow = mp.get_property_number("osd-width")
        local oh = mp.get_property_number("osd-height")
        local vw = mp.get_property_number("width")
        local vh = mp.get_property_number("height")
        local hw = mp.get_property("hwdec-current", "")
        local tw = (ow and ow > 0) and ow or dw
        local th = (oh and oh > 0) and oh or dh

        if tw and th and vw and vh and tw > 0 and th > 0 and vw > 0 and vh > 0
            and hw ~= nil and hw ~= "" then
            apply_filters()
        else
            retries = retries + 1
            if retries <= max_retries then
                mp.add_timeout(0.2, try_apply)
            else
                mp.msg.warn("RTX: hwdec-current not populated after retries, applying anyway")
                apply_filters()
            end
        end
    end

    mp.add_timeout(0.1, try_apply)
end

ensure_observers_registered()
publish_state(false, false, false, false, false)

mp.register_event("file-loaded", function()
    schedule_apply_with_retry()
end)

-- Keybindings
mp.add_key_binding("ctrl+shift+r", "autortx", toggle_rtx)
mp.add_key_binding("ctrl+shift+v", "autovsr", toggle_vsr)
mp.add_key_binding("ctrl+shift+h", "autohdr", toggle_hdr)

-- Allow OSC to trigger toggles via script-message-to
mp.register_script_message("toggle-vsr", toggle_vsr)
mp.register_script_message("toggle-hdr", toggle_hdr)
mp.register_script_message("toggle-rtx", toggle_rtx)
