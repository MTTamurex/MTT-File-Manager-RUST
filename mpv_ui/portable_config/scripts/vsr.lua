local mp = require 'mp'

-- Configuration
local autovsr_enabled = false
local vsr_filter_active = false

-- Publish initial state for OSC buttons
mp.set_property_bool("user-data/vsr/vsr-enabled", autovsr_enabled)

local function needs_format_conversion(codec, pixelformat)
    if codec:lower():match("hevc") or codec:lower():match("h%.265") then
        if pixelformat:match("p10le$") or pixelformat == "p010" then
            return true
        end
    end
    return false
end

-- Compute the upscale factor from display vs video dimensions.
local function compute_vsr_scale()
    local vw = mp.get_property_number("width")
    local vh = mp.get_property_number("height")
    local dw = mp.get_property_number("display-width")
    local dh = mp.get_property_number("display-height")
    if not (vw and vh and dw and dh) then return nil end
    if vw <= 0 or vh <= 0 or dw <= 0 or dh <= 0 then return nil end
    local s = math.max(dw / vw, dh / vh)
    s = math.floor(s * 10) / 10
    return math.max(s, 2.0)
end

local function remove_vsr()
    pcall(mp.commandv, "vf", "remove", "@rtx-vsr")
    pcall(mp.commandv, "vf", "remove", "@format-nv12")
    vsr_filter_active = false
end

-- Add the VSR filter chain. Returns true if successful.
local function add_vsr()
    local codec = mp.get_property("video-codec", "")
    local pixelformat = mp.get_property("video-params/pixelformat", "")
    if codec == "" then return false end

    if needs_format_conversion(codec, pixelformat) then
        pcall(mp.commandv, "vf", "append", "@format-nv12:format=nv12")
    end

    local scale = compute_vsr_scale()
    if not scale then return false end

    local filter_str = "@rtx-vsr:d3d11vpp=scaling-mode=nvidia:scale=" .. scale .. ":nvidia-true-hdr"
    local ok = pcall(mp.commandv, "vf", "append", filter_str)
    if ok then
        vsr_filter_active = true
        return true
    end
    return false
end

local function toggle_vsr()
    autovsr_enabled = not autovsr_enabled
    mp.set_property_bool("user-data/vsr/vsr-enabled", autovsr_enabled)

    remove_vsr()
    if autovsr_enabled then
        local ok = add_vsr()
        mp.osd_message(ok and "RTX: ON" or "RTX: ON (not active)", 2)
    else
        mp.osd_message("RTX: OFF", 2)
    end
end

-- FULLSCREEN TRANSITION FIX
-- The d3d11vpp filter changes video-out dimensions (upscales to display res).
-- When mpv exits fullscreen it recalculates the window size based on those
-- inflated dimensions, resulting in a near-zero or wrong-sized window.
-- Fix: temporarily remove the filter before the window is resized, then
-- re-add it once the window has settled at the correct geometry.
mp.observe_property("fullscreen", "bool", function(name, is_fs)
    if not is_fs and autovsr_enabled and vsr_filter_active then
        remove_vsr()
        mp.add_timeout(0.3, function()
            if autovsr_enabled and not mp.get_property_bool("fullscreen") then
                add_vsr()
            end
        end)
    end
end)

-- Apply filters on file load if VSR was already enabled.
mp.register_event("file-loaded", function()
    if autovsr_enabled then
        local retries = 0
        local max_retries = 15
        local function try_apply()
            local dw = mp.get_property_number("display-width")
            local dh = mp.get_property_number("display-height")
            local vw = mp.get_property_number("width")
            local vh = mp.get_property_number("height")
            local hw = mp.get_property("hwdec-current", "")
            if dw and dh and vw and vh and dw > 0 and dh > 0 and vw > 0 and vh > 0
                and hw ~= nil and hw ~= "" then
                add_vsr()
            else
                retries = retries + 1
                if retries <= max_retries then
                    mp.add_timeout(0.2, try_apply)
                else
                    mp.msg.warn("RTX VSR: hwdec-current not populated after retries, applying anyway")
                    add_vsr()
                end
            end
        end
        mp.add_timeout(0.1, try_apply)
    end
end)

mp.add_key_binding("ctrl+shift+r", "autovsr", toggle_vsr)
mp.register_script_message("toggle-vsr", toggle_vsr)
