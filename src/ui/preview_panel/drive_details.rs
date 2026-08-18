use crate::domain::file_entry::FileEntry;
use eframe::egui;
use mtt_search_protocol::{DriveHealthState, DriveRotation};
use rust_i18n::t;

pub(super) fn render_drive_details(
    ui: &mut egui::Ui,
    file: &FileEntry,
    add_detail: &impl Fn(&mut egui::Ui, &str, String),
) {
    let Some(drive) = file.drive_info.as_ref() else {
        return;
    };

    add_optional_detail(ui, add_detail, "file_info.drive_model", &drive.model);
    add_optional_detail(
        ui,
        add_detail,
        "file_info.drive_serial",
        &drive.serial_number,
    );
    add_optional_detail(
        ui,
        add_detail,
        "file_info.drive_firmware",
        &drive.firmware_revision,
    );
    add_optional_detail(ui, add_detail, "file_info.drive_bus_type", &drive.bus_type);

    let Some(snapshot) = drive.health.as_ref() else {
        return;
    };

    if let Some(mode) = transfer_mode(snapshot) {
        add_detail(ui, &t!("file_info.drive_transfer_mode"), mode);
    }
    add_detail(
        ui,
        &t!("file_info.drive_letter"),
        format!("{}:", snapshot.drive_letter.to_ascii_uppercase()),
    );
    add_optional_detail(
        ui,
        add_detail,
        "file_info.drive_standard",
        &snapshot.standard,
    );
    if !snapshot.features.is_empty() {
        add_detail(
            ui,
            &t!("file_info.drive_features"),
            snapshot.features.join(", "),
        );
    }
    if let Some(bytes) = snapshot.total_host_reads_bytes {
        add_detail(
            ui,
            &t!("file_info.drive_total_reads"),
            format_u128_size(bytes),
        );
    }
    if let Some(bytes) = snapshot.total_host_writes_bytes {
        add_detail(
            ui,
            &t!("file_info.drive_total_writes"),
            format_u128_size(bytes),
        );
    }
    match snapshot.rotation {
        DriveRotation::Unknown => {}
        DriveRotation::SolidState => add_detail(
            ui,
            &t!("file_info.drive_rotation"),
            t!("file_info.drive_solid_state").to_string(),
        ),
        DriveRotation::Rpm(rpm) => {
            add_detail(ui, &t!("file_info.drive_rotation"), format!("{rpm} RPM"))
        }
    }
    if let Some(count) = snapshot.power_cycle_count {
        add_detail(ui, &t!("file_info.drive_power_cycles"), count.to_string());
    }
    if let Some(hours) = snapshot.power_on_hours {
        add_detail(
            ui,
            &t!("file_info.drive_power_on_hours"),
            format!("{} {}", hours, t!("file_info.hours_unit")),
        );
    }
    if snapshot.smart_available {
        let health = health_text(snapshot.health_state.clone());
        if matches!(
            snapshot.health_state,
            DriveHealthState::Warning | DriveHealthState::Critical
        ) {
            add_health_alert_detail(
                ui,
                &t!("file_info.drive_health"),
                health,
                &snapshot.health_state,
            );
        } else {
            add_detail(ui, &t!("file_info.drive_health"), health);
        }
    }
    if let Some(temperature) = snapshot.temperature_celsius {
        add_detail(
            ui,
            &t!("file_info.drive_temperature"),
            format!("{temperature} °C"),
        );
    }
}

fn add_health_alert_detail(
    ui: &mut egui::Ui,
    label: &str,
    value: String,
    state: &DriveHealthState,
) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            egui::vec2(110.0, 0.0),
            egui::Label::new(egui::RichText::new(label).color(ui.visuals().weak_text_color())),
        );
        ui.add(egui::Label::new(value).wrap());
        let (badge_rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        crate::ui::drive_health_badge::paint_health_badge_in_rect(ui.painter(), badge_rect, state);
    });
    ui.add_space(4.0);
}

fn add_optional_detail(
    ui: &mut egui::Ui,
    add_detail: &impl Fn(&mut egui::Ui, &str, String),
    label_key: &str,
    value: &Option<String>,
) {
    if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
        add_detail(ui, &t!(label_key), value.clone());
    }
}

fn transfer_mode(snapshot: &mtt_search_protocol::DriveHealthSnapshot) -> Option<String> {
    match (
        snapshot.current_transfer_mode.as_deref(),
        snapshot.max_transfer_mode.as_deref(),
    ) {
        (Some(current), Some(maximum)) => Some(format!("{current} | {maximum}")),
        (Some(value), None) | (None, Some(value)) if !value.is_empty() => Some(value.to_string()),
        _ => None,
    }
}

fn health_text(state: DriveHealthState) -> String {
    match state {
        DriveHealthState::Good => t!("file_info.health_good"),
        DriveHealthState::Warning => t!("file_info.health_warning"),
        DriveHealthState::Critical => t!("file_info.health_critical"),
        DriveHealthState::Unknown => t!("file_info.health_unknown"),
    }
    .to_string()
}

fn format_u128_size(bytes: u128) -> String {
    const UNITS: [&str; 7] = ["bytes", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::format_u128_size;

    #[test]
    fn formats_large_drive_counters_without_u64_truncation() {
        assert_eq!(format_u128_size(1024u128.pow(5)), "1.00 PB");
        assert_eq!(format_u128_size(512), "512 bytes");
    }
}
