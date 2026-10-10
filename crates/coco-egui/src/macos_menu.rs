//! macOS application menu: points winit's default "About" item at the app's
//! own About window instead of AppKit's standard panel.

use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenuItem};
use objc2_foundation::NSString;

use crate::about::{AboutRequest, MENU_LABEL};

/// The application menu's position in the menu bar.
const APP_MENU_INDEX: isize = 0;

struct TargetIvars {
    request: AboutRequest,
    ctx: egui::Context,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and the class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CoCoVMAboutMenuTarget"]
    #[ivars = TargetIvars]
    struct AboutTarget;

    impl AboutTarget {
        #[unsafe(method(openAbout:))]
        fn open_about(&self, _sender: Option<&AnyObject>) {
            let ivars = self.ivars();
            ivars.request.raise();
            ivars.ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }
);

impl AboutTarget {
    fn new(mtm: MainThreadMarker, ivars: TargetIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Retarget the app menu's About item to raise `request`. Call once, on the
/// main thread, after winit has built its default menu.
pub(crate) fn install_about(ctx: &egui::Context, request: AboutRequest) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("About menu item not installed: not on the main thread");
        return;
    };
    let Some(item) = standard_about_item(mtm) else {
        tracing::warn!("About menu item not installed: no standard About item in the app menu");
        return;
    };
    let target = AboutTarget::new(
        mtm,
        TargetIvars {
            request,
            ctx: ctx.clone(),
        },
    );
    // SAFETY: the target implements `openAbout:` with the action signature,
    // and is leaked below so the item's unretained reference stays valid.
    unsafe {
        item.setTarget(Some(&target));
        item.setAction(Some(sel!(openAbout:)));
    }
    item.setTitle(&NSString::from_str(MENU_LABEL));
    let _ = Retained::into_raw(target);
}

/// The app menu item wired to AppKit's standard About panel.
fn standard_about_item(mtm: MainThreadMarker) -> Option<Retained<NSMenuItem>> {
    let app_menu = NSApplication::sharedApplication(mtm)
        .mainMenu()?
        .itemAtIndex(APP_MENU_INDEX)?
        .submenu()?;
    app_menu
        .itemArray()
        .into_iter()
        .find(|item| item.action() == Some(sel!(orderFrontStandardAboutPanel:)))
}
