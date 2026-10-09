//! Instructions above the welcome artwork while no machine is selected.

use eframe::egui;

use super::welcome_image::WelcomeImage;

const TOP_GAP: f32 = 24.0;
const HEADING_GAP: f32 = 8.0;
const ARTWORK_GAP: f32 = 16.0;

pub(super) fn draw(ui: &mut egui::Ui, image: &mut WelcomeImage, has_machines: bool) {
    ui.vertical_centered(|ui| {
        ui.add_space(TOP_GAP);
        ui.add(
            egui::Label::new(
                egui::RichText::new(concat!(
                    "Welcome to CoCoVM ",
                    env!("CARGO_PKG_VERSION"),
                    " !"
                ))
                .heading(),
            )
            .wrap(),
        );
        ui.add_space(HEADING_GAP);
        ui.add(egui::Label::new(creation_instruction(ui)).wrap());
        if has_machines {
            ui.add(egui::Label::new("Or click a VM in the list to edit its settings.").wrap());
        }
        ui.add_space(ARTWORK_GAP);
        if ui.available_height() > 0.0 && ui.available_width() > 0.0 {
            image.draw(ui);
        }
    });
}

fn creation_instruction(ui: &egui::Ui) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for text in [
        egui::RichText::new("Click "),
        egui::RichText::new("New").strong(),
        egui::RichText::new(" to create a virtual machine."),
    ] {
        text.append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Center,
        );
    }
    job
}
