local mp = require("mp")

local FILTER_TAG = "@mtt-rtx"
local FORMAT_TAG = "@mtt-rtx-format"

local state = {
    vsr_enabled = false,
    hdr_enabled = false,
    rtx_checked = false,
    rtx_supported = false,
    rtx_adapter_name = "",
    chain_active = false,
    physical_display = { width = nil, height = nil },
    restore_timer = nil,
}

local function sync_ui_flags()
    mp.set_property_bool("user-data/vsr/vsr-enabled", state.vsr_enabled)
    mp.set_property_bool("user-data/vsr/hdr-enabled", state.hdr_enabled)
    mp.set_property_bool("user-data/vsr/rtx-checked", state.rtx_checked)
    mp.set_property_bool("user-data/vsr/rtx-supported", state.rtx_supported)
    mp.set_property("user-data/vsr/rtx-adapter-name", state.rtx_adapter_name)
end

local function floor_to_tenth(value)
    return math.floor(value * 10) / 10
end

local function probe_physical_display_async()
    mp.command_native_async({
        name = "subprocess",
        args = {
            "powershell.exe", "-NoProfile", "-NonInteractive",
            "-ExecutionPolicy", "Bypass", "-Command",
            "$adapter = Get-CimInstance Win32_VideoController | Where-Object {$_.CurrentHorizontalResolution -gt 0} | Sort-Object CurrentHorizontalResolution -Descending | Select-Object -First 1; if ($adapter) { Write-Output ($adapter.CurrentHorizontalResolution.ToString() + 'x' + $adapter.CurrentVerticalResolution.ToString()) }"
        },
        capture_stdout = true,
        playback_only = false,
    }, function(success, result)
        if not success or not result or result.status ~= 0 or not result.stdout then
            mp.msg.warn("RTX filters: unable to query physical monitor size, using mpv display metrics")
            return
        end

        local width, height = result.stdout:match("(%d+)%s*x%s*(%d+)")
        if not width or not height then
            mp.msg.warn("RTX filters: unexpected monitor probe output: " .. tostring(result.stdout))
            return
        end

        state.physical_display.width = tonumber(width)
        state.physical_display.height = tonumber(height)
        mp.msg.info("RTX filters: physical display " .. state.physical_display.width .. "x" .. state.physical_display.height)
    end)
end

local function detect_rtx_compatibility_async()
    mp.command_native_async({
        name = "subprocess",
        args = {
            "powershell.exe", "-NoProfile", "-NonInteractive",
            "-ExecutionPolicy", "Bypass", "-Command",
            "$gpu = Get-CimInstance Win32_VideoController | Where-Object { (($_.Name -match 'NVIDIA') -or ($_.AdapterCompatibility -match 'NVIDIA')) -and ($_.Name -match 'RTX') } | Select-Object -First 1; if ($gpu) { Write-Output $gpu.Name; exit 0 } else { exit 2 }"
        },
        capture_stdout = true,
        playback_only = false,
    }, function(success, result)
        state.rtx_checked = true
        state.rtx_supported = false
        state.rtx_adapter_name = ""

        if success and result and result.status == 0 and result.stdout then
            local adapter = result.stdout:gsub("^%s+", ""):gsub("%s+$", "")
            if adapter ~= "" then
                state.rtx_supported = true
                state.rtx_adapter_name = adapter
                mp.msg.info("RTX filters: compatible adapter detected: " .. adapter)
            end
        end

        if not state.rtx_supported then
            mp.msg.info("RTX filters: NVIDIA RTX adapter not detected; RTX controls disabled")
        end

        sync_ui_flags()
    end)
end

local function current_display_size()
    if state.physical_display.width and state.physical_display.height then
        return state.physical_display.width, state.physical_display.height
    end

    local width = mp.get_property_number("display-width")
    local height = mp.get_property_number("display-height")
    if not width or not height or width <= 0 or height <= 0 then
        return nil, nil
    end

    local hidpi_scale = mp.get_property_number("display-hidpi-scale")
    if hidpi_scale and hidpi_scale > 1.0 then
        width = math.floor(width * hidpi_scale + 0.5)
        height = math.floor(height * hidpi_scale + 0.5)
    end

    return math.floor(width), math.floor(height)
end

local function source_dimensions()
    local width = mp.get_property_number("width")
    local height = mp.get_property_number("height")
    if not width or not height or width <= 0 or height <= 0 then
        return nil, nil
    end
    return width, height
end

local function source_is_hdr()
    local primaries = mp.get_property("video-params/primaries", "")
    local transfer = mp.get_property("video-params/gamma", "")
    return primaries == "bt.2020" or transfer == "pq" or transfer == "hlg"
end

local function hevc_main10_requires_bridge(codec, pixel_format)
    if codec == nil or pixel_format == nil then
        return false
    end

    local lowered = codec:lower()
    if not lowered:match("hevc") and not lowered:match("h%.265") then
        return false
    end

    return pixel_format:match("p10le$") ~= nil or pixel_format == "p010"
end

local function requested_scale()
    local video_width, video_height = source_dimensions()
    local display_width, display_height = current_display_size()
    if not video_width or not video_height or not display_width or not display_height then
        return nil
    end

    local fit_ratio = math.min(display_width / video_width, display_height / video_height)
    if fit_ratio <= 1.0 then
        return nil
    end

    return floor_to_tenth(fit_ratio)
end

local function remove_filter_chain()
    pcall(mp.commandv, "vf", "remove", FILTER_TAG)
    pcall(mp.commandv, "vf", "remove", FORMAT_TAG)
    state.chain_active = false
end

local function build_filter_chain()
    local segments = {}

    if state.vsr_enabled then
        local scale = requested_scale()
        if scale then
            segments[#segments + 1] = "scaling-mode=nvidia"
            segments[#segments + 1] = "scale=" .. scale
        end
    end

    if state.hdr_enabled and not source_is_hdr() then
        segments[#segments + 1] = "nvidia-true-hdr"
    end

    if #segments == 0 then
        return nil
    end

    return FILTER_TAG .. ":d3d11vpp=" .. table.concat(segments, ":")
end

local function video_context_ready()
    local video_width, video_height = source_dimensions()
    local display_width, display_height = current_display_size()
    local codec = mp.get_property("video-codec", "")

    return video_width ~= nil and video_height ~= nil
        and display_width ~= nil and display_height ~= nil
        and codec ~= nil and codec ~= ""
end

local function apply_filter_chain()
    remove_filter_chain()

    if not state.vsr_enabled and not state.hdr_enabled then
        return false
    end

    local codec = mp.get_property("video-codec", "")
    local pixel_format = mp.get_property("video-params/pixelformat", "")
    if codec == "" then
        return false
    end

    local chain = build_filter_chain()
    if not chain then
        return false
    end

    if hevc_main10_requires_bridge(codec, pixel_format) then
        pcall(mp.commandv, "vf", "append", FORMAT_TAG .. ":format=nv12")
    end

    local ok = pcall(mp.commandv, "vf", "append", chain)
    if not ok then
        pcall(mp.commandv, "vf", "remove", FORMAT_TAG)
        return false
    end

    state.chain_active = true
    return true
end

local function show_toggle_result(label, enabled, applied)
    if not enabled then
        mp.osd_message(label .. ": OFF", 2)
        return
    end

    if applied then
        mp.osd_message(label .. ": ON", 2)
    else
        mp.osd_message(label .. ": ON (not active)", 2)
    end
end

local function rtx_available_for_toggle()
    if not state.rtx_checked then
        mp.osd_message("RTX features: checking GPU compatibility", 2)
        return false
    end

    if not state.rtx_supported then
        mp.osd_message("RTX features unavailable: NVIDIA RTX GPU not detected", 3)
        return false
    end

    return true
end

local function toggle_vsr()
    if not rtx_available_for_toggle() then
        return
    end

    state.vsr_enabled = not state.vsr_enabled
    sync_ui_flags()
    local applied = apply_filter_chain()
    show_toggle_result("RTX VSR", state.vsr_enabled, applied)
end

local function toggle_hdr()
    if not rtx_available_for_toggle() then
        return
    end

    state.hdr_enabled = not state.hdr_enabled
    sync_ui_flags()
    local applied = apply_filter_chain()
    show_toggle_result("RTX HDR", state.hdr_enabled, applied)
end

local function schedule_restore(delay_seconds)
    if state.restore_timer then
        state.restore_timer:kill()
        state.restore_timer = nil
    end

    state.restore_timer = mp.add_timeout(delay_seconds, function()
        state.restore_timer = nil
        if (state.vsr_enabled or state.hdr_enabled) and not mp.get_property_bool("fullscreen", false) then
            apply_filter_chain()
        end
    end)
end

mp.observe_property("fullscreen", "bool", function(_, is_fullscreen)
    if is_fullscreen or not state.chain_active then
        return
    end

    remove_filter_chain()
    schedule_restore(0.30)
end)

mp.register_event("file-loaded", function()
    if not state.vsr_enabled and not state.hdr_enabled then
        return
    end

    local attempts = 0
    local max_attempts = 15

    local function retry_apply()
        if video_context_ready() then
            apply_filter_chain()
            return
        end

        attempts = attempts + 1
        if attempts >= max_attempts then
            mp.msg.warn("RTX filters: applying with incomplete video context after retries")
            apply_filter_chain()
            return
        end

        mp.add_timeout(0.20, retry_apply)
    end

    mp.add_timeout(0.10, retry_apply)
end)

mp.register_event("end-file", function()
    remove_filter_chain()
end)

sync_ui_flags()
probe_physical_display_async()
detect_rtx_compatibility_async()

mp.add_key_binding("ctrl+shift+r", "toggle-mtt-rtx-vsr", toggle_vsr)
mp.register_script_message("toggle-vsr", toggle_vsr)
mp.register_script_message("toggle-hdr", toggle_hdr)
