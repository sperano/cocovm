//! Instructions above the welcome artwork while no machine is selected.

use eframe::egui;

use super::welcome_image::WelcomeImage;
use crate::update::UpdateCheck;

const TOP_GAP: f32 = 24.0;
const HEADING_GAP: f32 = 8.0;
const ARTWORK_GAP: f32 = 16.0;
/// Text of the update notice's link to the release page.
pub(crate) const RELEASE_LINK_TEXT: &str = "Release notes and downloads";
/// Label of the button that hides the update notice for the session.
pub(crate) const DISMISS_LABEL: &str = "Dismiss";

pub(super) fn draw(
    ui: &mut egui::Ui,
    image: &mut WelcomeImage,
    has_machines: bool,
    update: &mut UpdateCheck,
) {
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
        update_notice(ui, update);
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

/// A newer release's version, its release page, and Dismiss; nothing
/// while no newer release is known or after Dismiss.
fn update_notice(ui: &mut egui::Ui, update: &mut UpdateCheck) {
    let Some(release) = update.notice() else {
        return;
    };
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("CoCoVM {} is available.", release.version)).strong(),
        )
        .wrap(),
    );
    ui.hyperlink_to(RELEASE_LINK_TEXT, &release.page_url);
    if ui.small_button(DISMISS_LABEL).clicked() {
        update.dismiss();
    }
    ui.add_space(HEADING_GAP);
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
