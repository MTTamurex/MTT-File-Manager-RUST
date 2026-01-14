//! Window subclass for borderless resize handling on Windows
//!
//! This module provides native Windows message handling for borderless windows.
//! It intercepts WM_NCHITTEST to provide resize borders when decorations are disabled.
//! It also tracks WM_ENTERSIZEMOVE/WM_EXITSIZEMOVE for UI optimization during resize.
//!
//! # Layout Freeze System
//! To prevent sidebar width corruption during minimize/restore, this module implements
//! a phase-based layout freeze:
//! - Normal: Layout can be mutated freely
//! - Minimized: Layout mutations frozen, snapshot preserved
//! - Restoring: First frame(s) after restore, still frozen until valid dimensions confirmed
//!
//! # Why This Is Required
//! - egui/winit cannot handle native window chrome
//! - winit returns HTCLIENT for entire borderless window
//! - Windows needs edge codes (HTLEFT, HTRIGHT, etc.) for resize cursors/behavior
//! - During minimize, client area is 0x0 which corrupts egui layout calculations

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsZoomed, WM_NCHITTEST, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_SIZE,
    HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
};

/// SIZE_MINIMIZED constant (wParam for WM_SIZE when window is minimized)
const SIZE_MINIMIZED: usize = 1;
/// SIZE_RESTORED constant (wParam for WM_SIZE when window is restored)
const SIZE_RESTORED: usize = 0;
/// SIZE_MAXIMIZED constant (wParam for WM_SIZE when window is maximized)
const SIZE_MAXIMIZED: usize = 2;

/// Resize border width in pixels (scaled by DPI at runtime if needed)
const RESIZE_BORDER_WIDTH: i32 = 8;

/// Subclass ID for our borderless handler
const BORDERLESS_SUBCLASS_ID: usize = 1;

// ============================================================================
// LAYOUT FREEZE SYSTEM
// ============================================================================

/// Layout phase for sidebar state management.
/// Prevents layout corruption during minimize/restore cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WindowLayoutPhase {
    /// Normal operation - layout can be mutated
    Normal = 0,
    /// Window is minimized - layout frozen, using snapshot
    Minimized = 1,
    /// First frame(s) after restore - still frozen until dimensions valid
    Restoring = 2,
}

impl From<u8> for WindowLayoutPhase {
    fn from(val: u8) -> Self {
        match val {
            1 => WindowLayoutPhase::Minimized,
            2 => WindowLayoutPhase::Restoring,
            _ => WindowLayoutPhase::Normal,
        }
    }
}

/// Snapshot of sidebar widths taken before minimize.
/// Used to restore exact state after restore.
#[derive(Debug, Clone, Copy, Default)]
pub struct SidebarSnapshot {
    pub left_width: f32,
    pub right_width: f32,
    pub valid: bool,
}

// ============================================================================
// STATIC STATE
// ============================================================================

/// Flag to track if subclass is installed (prevents double-install)
static SUBCLASS_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Flag to track if window is currently being resized or dragged
/// Set true on WM_ENTERSIZEMOVE, false on WM_EXITSIZEMOVE
static IS_IN_SIZE_MOVE: AtomicBool = AtomicBool::new(false);

/// Current layout phase (atomic for lock-free read)
static LAYOUT_PHASE: AtomicU8 = AtomicU8::new(0); // 0 = Normal

/// Sidebar snapshot protected by mutex (written only on minimize)
static SIDEBAR_SNAPSHOT: Mutex<SidebarSnapshot> = Mutex::new(SidebarSnapshot {
    left_width: 200.0,
    right_width: 300.0,
    valid: false,
});

// ============================================================================
// PUBLIC API
// ============================================================================

/// Check if the window is currently being resized or dragged.
/// Use this to gate expensive rendering during resize operations.
#[inline]
pub fn is_in_size_move() -> bool {
    IS_IN_SIZE_MOVE.load(Ordering::Relaxed)
}

/// Get the current layout phase.
/// Use this to determine if layout mutations are allowed.
#[inline]
pub fn layout_phase() -> WindowLayoutPhase {
    WindowLayoutPhase::from(LAYOUT_PHASE.load(Ordering::Relaxed))
}

/// Check if layout is currently frozen (Minimized or Restoring).
/// When frozen, sidebar widths should NOT be updated from UI calculations.
#[inline]
pub fn is_layout_frozen() -> bool {
    LAYOUT_PHASE.load(Ordering::Relaxed) != 0
}

/// Check if the window is currently minimized.
/// Legacy API - prefer is_layout_frozen() or layout_phase().
#[inline]
pub fn is_minimized() -> bool {
    LAYOUT_PHASE.load(Ordering::Relaxed) == WindowLayoutPhase::Minimized as u8
}

/// Freeze the layout and save sidebar widths before minimize.
/// Call this with current sidebar widths BEFORE the window is minimized.
pub fn freeze_layout(left_width: f32, right_width: f32) {
    if let Ok(mut snapshot) = SIDEBAR_SNAPSHOT.lock() {
        snapshot.left_width = left_width;
        snapshot.right_width = right_width;
        snapshot.valid = true;
    }
    LAYOUT_PHASE.store(WindowLayoutPhase::Minimized as u8, Ordering::SeqCst);
}

/// Get the frozen sidebar snapshot.
/// Returns the last valid sidebar widths before minimize.
pub fn get_frozen_sidebar_widths() -> (f32, f32) {
    if let Ok(snapshot) = SIDEBAR_SNAPSHOT.lock() {
        if snapshot.valid {
            return (snapshot.left_width, snapshot.right_width);
        }
    }
    // Fallback to reasonable defaults
    (200.0, 300.0)
}

/// Attempt to unfreeze layout (transition from Restoring to Normal).
/// Only succeeds if dimensions are valid (width > 0, height > 0).
/// Returns true if layout is now Normal (unfrozen).
pub fn try_unfreeze_layout(available_width: f32, available_height: f32) -> bool {
    let current_phase = LAYOUT_PHASE.load(Ordering::Relaxed);
    
    // Only transition from Restoring to Normal
    if current_phase == WindowLayoutPhase::Restoring as u8 {
        // Require valid dimensions
        if available_width > 100.0 && available_height > 100.0 {
            LAYOUT_PHASE.store(WindowLayoutPhase::Normal as u8, Ordering::SeqCst);
            return true;
        }
    }
    
    current_phase == WindowLayoutPhase::Normal as u8
}

/// Install the borderless window subclass on the given HWND.
///
/// This intercepts WM_NCHITTEST to provide resize borders.
/// Safe to call multiple times - will only install once.
///
/// # Safety
/// HWND must be a valid window handle from the current process.
pub fn install_borderless_subclass(hwnd: HWND) -> bool {
    if SUBCLASS_INSTALLED.load(Ordering::SeqCst) {
        return true; // Already installed
    }

    // SAFETY: We pass a valid HWND, a valid callback, and our subclass ID.
    // The callback follows Windows calling convention via `extern "system"`.
    let result = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(borderless_subclass_proc),
            BORDERLESS_SUBCLASS_ID,
            0, // No extra data needed
        )
    };

    if result.as_bool() {
        SUBCLASS_INSTALLED.store(true, Ordering::SeqCst);
        true
    } else {
        eprintln!("Failed to install borderless window subclass");
        false
    }
}

/// Remove the borderless window subclass.
///
/// Call this on window close to clean up.
pub fn remove_borderless_subclass(hwnd: HWND) {
    if !SUBCLASS_INSTALLED.load(Ordering::SeqCst) {
        return;
    }

    // SAFETY: HWND is valid, we're removing our own subclass.
    unsafe {
        let _ = RemoveWindowSubclass(hwnd, Some(borderless_subclass_proc), BORDERLESS_SUBCLASS_ID);
    }

    SUBCLASS_INSTALLED.store(false, Ordering::SeqCst);
}

/// Subclass window procedure that handles WM_NCHITTEST for borderless resize.
///
/// # Message Handling
/// - WM_NCHITTEST: Returns edge/corner codes for resize zones, HTCLIENT otherwise
/// - WM_ENTERSIZEMOVE: Sets IS_IN_SIZE_MOVE flag (user started drag/resize)
/// - WM_EXITSIZEMOVE: Clears IS_IN_SIZE_MOVE flag (user finished drag/resize)
/// - All other messages: Passed to DefSubclassProc
extern "system" fn borderless_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid_subclass: usize,
    _dw_ref_data: usize,
) -> LRESULT {
    // Handle resize state tracking for UI optimization
    if msg == WM_ENTERSIZEMOVE {
        IS_IN_SIZE_MOVE.store(true, Ordering::SeqCst);
    } else if msg == WM_EXITSIZEMOVE {
        IS_IN_SIZE_MOVE.store(false, Ordering::SeqCst);
    }

    // Handle layout phase transitions for minimize/restore
    // NOTE: The freeze_layout() now happens in the UI layer (before minimize)
    // to capture current sidebar widths. Here we only handle phase transitions.
    if msg == WM_SIZE {
        let size_type = wparam.0;
        if size_type == SIZE_MINIMIZED {
            // Transition to Minimized phase
            // Note: freeze_layout() should be called by UI layer before this
            // to capture sidebar widths. If not yet frozen, do it now with defaults.
            let current = LAYOUT_PHASE.load(Ordering::Relaxed);
            if current == WindowLayoutPhase::Normal as u8 {
                // UI didn't freeze - shouldn't happen but handle gracefully
                LAYOUT_PHASE.store(WindowLayoutPhase::Minimized as u8, Ordering::SeqCst);
            }
        } else if size_type == SIZE_RESTORED || size_type == SIZE_MAXIMIZED {
            // Transition from Minimized to Restoring (not directly to Normal)
            // The UI layer must call try_unfreeze_layout() to complete transition
            let current = LAYOUT_PHASE.load(Ordering::Relaxed);
            if current == WindowLayoutPhase::Minimized as u8 {
                LAYOUT_PHASE.store(WindowLayoutPhase::Restoring as u8, Ordering::SeqCst);
            }
        }
    }

    if msg == WM_NCHITTEST {
        return handle_nchittest(hwnd, lparam);
    }

    // Pass all other messages to default handler
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Handle WM_NCHITTEST message to provide resize borders.
///
/// Returns appropriate hit-test code based on cursor position:
/// - Edge zones (8px from border): HTLEFT, HTRIGHT, HTTOP, HTBOTTOM
/// - Corner zones (8x8px): HTTOPLEFT, HTTOPRIGHT, HTBOTTOMLEFT, HTBOTTOMRIGHT
/// - Rest of window: HTCLIENT (let egui handle)
fn handle_nchittest(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    // Don't provide resize borders when maximized
    // SAFETY: HWND is valid, IsZoomed just queries window state
    if unsafe { IsZoomed(hwnd).as_bool() } {
        return LRESULT(HTCLIENT as isize);
    }

    // Extract cursor position from lparam (screen coordinates)
    let cursor_x = (lparam.0 as i32) & 0xFFFF;
    let cursor_y = ((lparam.0 as i32) >> 16) & 0xFFFF;

    // Handle signed coordinate conversion for multi-monitor setups
    let cursor_x = if cursor_x > 32767 {
        cursor_x - 65536
    } else {
        cursor_x
    };
    let cursor_y = if cursor_y > 32767 {
        cursor_y - 65536
    } else {
        cursor_y
    };

    // Get window client rect
    let mut client_rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: HWND is valid, client_rect is a valid mutable reference
    if unsafe { GetClientRect(hwnd, &mut client_rect).is_err() } {
        return LRESULT(HTCLIENT as isize);
    }

    // Convert screen coords to window-relative coords
    // We need window rect, not client rect, for proper edge detection
    let mut window_rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: HWND is valid
    if unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut window_rect).is_err()
    } {
        return LRESULT(HTCLIENT as isize);
    }

    // Calculate position relative to window
    let x = cursor_x - window_rect.left;
    let y = cursor_y - window_rect.top;
    let width = window_rect.right - window_rect.left;
    let height = window_rect.bottom - window_rect.top;

    // Check if cursor is within window bounds
    if x < 0 || y < 0 || x >= width || y >= height {
        return LRESULT(HTCLIENT as isize);
    }

    // Determine hit-test zone
    let on_left = x < RESIZE_BORDER_WIDTH;
    let on_right = x >= width - RESIZE_BORDER_WIDTH;
    let on_top = y < RESIZE_BORDER_WIDTH;
    let on_bottom = y >= height - RESIZE_BORDER_WIDTH;

    // Corner detection (corners take priority)
    let hit_test = if on_top && on_left {
        HTTOPLEFT
    } else if on_top && on_right {
        HTTOPRIGHT
    } else if on_bottom && on_left {
        HTBOTTOMLEFT
    } else if on_bottom && on_right {
        HTBOTTOMRIGHT
    } else if on_left {
        HTLEFT
    } else if on_right {
        HTRIGHT
    } else if on_top {
        HTTOP
    } else if on_bottom {
        HTBOTTOM
    } else {
        HTCLIENT
    };

    LRESULT(hit_test as isize)
}

/// RAII guard for borderless subclass.
///
/// Automatically removes subclass when dropped.
pub struct BorderlessSubclass {
    hwnd: HWND,
}

impl BorderlessSubclass {
    /// Create and install borderless subclass.
    ///
    /// Returns None if installation fails.
    pub fn new(hwnd: HWND) -> Option<Self> {
        if install_borderless_subclass(hwnd) {
            Some(Self { hwnd })
        } else {
            None
        }
    }
}

impl Drop for BorderlessSubclass {
    fn drop(&mut self) {
        remove_borderless_subclass(self.hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resize_border_constant() {
        // Ensure border width is reasonable
        assert!(RESIZE_BORDER_WIDTH > 0);
        assert!(RESIZE_BORDER_WIDTH <= 20);
    }
}
