// Keep these pure worker policies independently testable without starting egui.
#[path = "../src/ui/preview_panel/background.rs"]
mod background;
#[path = "../src/ui/preview_panel/markdown_safety.rs"]
mod markdown_safety;
