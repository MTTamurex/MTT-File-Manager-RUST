local mp = require 'mp'

-- Configuration
local autovsr_enabled = false -- Default to VSR disabled

-- Publish initial state for OSC buttons
mp.set_property_bool("user-data/vsr/vsr-enabled", autovsr_enabled)

-- Function to apply VSR
local function apply_filters()
    local video_width = mp.get_property_number("width")
    local video_height = mp.get_property_number("height")
    local display_width = mp.get_property_number("display-width")
    local display_height = mp.get_property_number("display-height")
    local codec = mp.get_property("video-codec", "")
    local pixelformat = mp.get_property("video-params/pixelformat", "")

    if not (video_width and video_height and display_width and display_height and codec and pixelformat) then
        mp.osd_message("RTX: Missing video properties, retrying...", 1)
        return
    end

    local scale = math.max(display_width / video_width, display_height / video_height)
    scale = math.floor(scale * 10) / 10

    -- Remove existing filters
    mp.commandv("vf", "remove", "@format-nv12")
    mp.commandv("vf", "remove", "@rtx-vsr")

    -- Apply format conversion for Main 10 HEVC if needed
    if codec:lower():match("hevc") or codec:lower():match("h%.265") then
        if pixelformat:match("p10le$") or pixelformat == "p010" then
            mp.commandv("vf", "append", "@format-nv12:format=nv12")
        end
    end

    -- Apply VSR if enabled and upscaling is needed
    if scale > 1 and autovsr_enabled then
        mp.commandv("vf", "append", "@rtx-vsr:d3d11vpp=scaling-mode=nvidia:scale=" .. scale)
        mp.osd_message("RTX VSR (" .. scale .. "x) ON", 2)
    end

    -- Publish current state for OSC buttons
    mp.set_property_bool("user-data/vsr/vsr-enabled", autovsr_enabled)
end

-- Function to toggle VSR
local function toggle_vsr()
    autovsr_enabled = not autovsr_enabled
    apply_filters()
    mp.osd_message("RTX VSR " .. (autovsr_enabled and "ON" or "OFF"), 2)
end

-- Function to show current status
local function show_status()
    local vsr_status = autovsr_enabled and "ON" or "OFF"
    local vf_chain = mp.get_property("vf", "")
    local vo = mp.get_property("vo", "")
    local gpu_api = mp.get_property("gpu-api", "")
    local hwdec = mp.get_property("hwdec-current", "")

    local video_width = mp.get_property_number("width", 0)
    local video_height = mp.get_property_number("height", 0)
    local display_width = mp.get_property_number("display-width", 0)
    local display_height = mp.get_property_number("display-height", 0)
    local scale = 1
    if video_width > 0 and video_height > 0 then
        scale = math.max(display_width / video_width, display_height / video_height)
        scale = math.floor(scale * 10) / 10
    end

    mp.osd_message(
        "RTX VSR Status:\n" ..
        "VSR: " .. vsr_status .. " (scale: " .. scale .. "x)\n" ..
        "VO: " .. vo .. " / API: " .. gpu_api .. " / HWDec: " .. hwdec .. "\n" ..
        "Filter chain: " .. (vf_chain ~= "" and vf_chain or "none"),
        5
    )
end

-- Apply filters automatically on video load
mp.register_event("file-loaded", function()
    if autovsr_enabled then
        mp.add_timeout(0.1, function()
            apply_filters()
            mp.observe_property("video-params", "native", apply_filters)
            mp.observe_property("vf", "native", apply_filters)
        end)
    end
end)

-- Keybindings
mp.add_key_binding("ctrl+shift+r", "autovsr", toggle_vsr)
mp.add_key_binding("ctrl+shift+s", "rtx_status", show_status)

-- Allow OSC to trigger toggles via script-message-to
mp.register_script_message("toggle-vsr", toggle_vsr)
