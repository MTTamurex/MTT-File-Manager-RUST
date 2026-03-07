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
    local osd_width = mp.get_property_number("osd-width")
    local osd_height = mp.get_property_number("osd-height")
    local codec = mp.get_property("video-codec", "")
    local pixelformat = mp.get_property("video-params/pixelformat", "")
    local hwdec_cur = mp.get_property("hwdec-current", "")

    local raw_w = (osd_width and osd_width > 0) and osd_width or display_width
    local raw_h = (osd_height and osd_height > 0) and osd_height or display_height
    local target_width = raw_w and math.floor(raw_w) or nil
    local target_height = raw_h and math.floor(raw_h) or nil

    if not (video_width and video_height and target_width and target_height and codec and pixelformat) then
        mp.osd_message("RTX: Missing video properties, retrying...", 1)
        return false, "missing-properties"
    end

    local scale = math.max(target_width / video_width, target_height / video_height)
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

    if autovsr_enabled then
        local vsr_scale = math.max(scale, 1.0)
        local filter_str = "@rtx-vsr:d3d11vpp=scaling-mode=nvidia:scale=" .. vsr_scale .. ":nvidia-true-hdr"
        local ok, err = pcall(mp.commandv, "vf", "append", filter_str)
        if not ok then
            mp.msg.warn("RTX VSR filter append failed: " .. tostring(err))
            return false, "append-failed"
        else
            local vf_chain = mp.get_property("vf", "")
            if vf_chain:find("@rtx%-vsr") then
                return true, "applied"
            end
            mp.msg.warn("RTX VSR not in filter chain after append")
            return false, "not-in-chain"
        end
    else
        return false, "disabled"
    end
end

-- Function to toggle VSR
local function toggle_vsr()
    autovsr_enabled = not autovsr_enabled
    -- Update OSC button state immediately
    mp.set_property_bool("user-data/vsr/vsr-enabled", autovsr_enabled)
    local applied, reason = apply_filters()
    if autovsr_enabled then
        mp.osd_message(applied and "RTX: ON" or "RTX: ON (not active)", 2)
    else
        mp.osd_message("RTX: OFF", 2)
    end
end

-- Apply filters automatically on video load.
-- Uses a retry loop because in standalone (borderless) mode the display
-- properties may not be available immediately after file-loaded.
mp.register_event("file-loaded", function()
    if autovsr_enabled then
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
            -- Also wait for hwdec-current to be populated (takes a frame or two)
            if tw and th and vw and vh and tw > 0 and th > 0 and vw > 0 and vh > 0
                and hw ~= nil and hw ~= "" then
                apply_filters()
                mp.observe_property("video-params", "native", apply_filters)
                mp.observe_property("vf", "native", apply_filters)
                mp.observe_property("fullscreen", "bool", apply_filters)
                mp.observe_property("osd-width", "number", apply_filters)
            else
                retries = retries + 1
                if retries <= max_retries then
                    mp.add_timeout(0.2, try_apply)
                else
                    -- Apply anyway even if hwdec-current is empty (codec may not support HW decode)
                    mp.msg.warn("RTX VSR: hwdec-current not populated after retries, applying anyway")
                    apply_filters()
                    mp.observe_property("video-params", "native", apply_filters)
                    mp.observe_property("vf", "native", apply_filters)
                    mp.observe_property("fullscreen", "bool", apply_filters)
                    mp.observe_property("osd-width", "number", apply_filters)
                end
            end
        end
        mp.add_timeout(0.1, try_apply)
    end
end)

-- Keybindings
mp.add_key_binding("ctrl+shift+r", "autovsr", toggle_vsr)

-- Allow OSC to trigger toggles via script-message-to
mp.register_script_message("toggle-vsr", toggle_vsr)
