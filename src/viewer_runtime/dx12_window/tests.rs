use super::*;
use std::cell::RefCell;
use std::rc::Rc;

struct ProbeApp {
    events: Rc<RefCell<Vec<&'static str>>>,
}

impl App for ProbeApp {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.events.borrow_mut().push("logic");
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }

    fn ui(&mut self, _: &mut egui::Ui, _: &mut eframe::Frame) {
        self.events.borrow_mut().push("ui");
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.events.borrow_mut().push("inner_exit");
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(7)
    }

    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.1, 0.2, 0.3, 1.0]
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn raw_input_hook(&mut self, _: &egui::Context, _: &mut egui::RawInput) {
        self.events.borrow_mut().push("input");
    }

    fn save(&mut self, _: &mut dyn eframe::Storage) {
        self.events.borrow_mut().push("save");
    }
}

#[test]
fn accepted_exit_hides_before_inner_cleanup() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = ProbeApp {
        events: Rc::clone(&events),
    };
    exit_app(&mut app, None, || events.borrow_mut().push("hide"));
    assert_eq!(*events.borrow(), ["hide", "inner_exit"]);
}

#[test]
fn wrapper_preserves_inner_app_settings_and_input() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = Dx12WindowApp {
        app: Box::new(ProbeApp {
            events: Rc::clone(&events),
        }),
        hwnd: None,
    };
    assert_eq!(app.auto_save_interval(), std::time::Duration::from_secs(7));
    assert_eq!(
        app.clear_color(&egui::Visuals::dark()),
        [0.1, 0.2, 0.3, 1.0]
    );
    assert!(!app.persist_egui_memory());
    app.raw_input_hook(&egui::Context::default(), &mut egui::RawInput::default());
    let _ = egui::Context::default().run_ui(egui::RawInput::default(), |ui| {
        app.ui(ui, &mut eframe::Frame::_new_kittest());
    });
    struct TestStorage;
    impl eframe::Storage for TestStorage {
        fn get_string(&self, _: &str) -> Option<String> {
            None
        }
        fn set_string(&mut self, _: &str, _: String) {}
        fn remove_string(&mut self, _: &str) {}
        fn flush(&mut self) {}
    }
    app.save(&mut TestStorage);
    assert_eq!(*events.borrow(), ["input", "ui", "save"]);
}

#[test]
fn close_cancellation_is_forwarded_without_running_exit_cleanup() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = Dx12WindowApp {
        app: Box::new(ProbeApp {
            events: Rc::clone(&events),
        }),
        hwnd: None,
    };
    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut eframe::Frame::_new_kittest());
    });
    let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    assert!(!commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::Visible(false))));
    assert_eq!(*events.borrow(), ["logic"]);
}

#[test]
fn accepted_exit_does_not_resize_or_destroy_native_window() {
    use windows::core::w;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetClientRect, IsWindow, IsWindowVisible, SetWindowPos,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };

    struct TestWindow(HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    // Never visible; all native operations run on the owning test thread.
    let window = TestWindow(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            320,
            240,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    unsafe {
        SetWindowPos(
            window.0,
            None,
            0,
            0,
            640,
            480,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
        )
        .unwrap();
        let mut before = RECT::default();
        GetClientRect(window.0, &mut before).unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut app = Dx12WindowApp {
            app: Box::new(ProbeApp {
                events: Rc::clone(&events),
            }),
            hwnd: Some(window.0),
        };
        app.on_exit(None);
        let mut after = RECT::default();
        GetClientRect(window.0, &mut after).unwrap();
        assert_eq!(before, after);
        assert!(IsWindow(Some(window.0)).as_bool());
        assert!(!IsWindowVisible(window.0).as_bool());
        assert_eq!(*events.borrow(), ["inner_exit"]);
    }
}
