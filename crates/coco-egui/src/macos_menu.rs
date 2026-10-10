//! macOS application menu: points winit's default "About" item at the app's
//! own About window instead of AppKit's standard panel, and adds "Check for
//! Updates…" right after it.

use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use crate::menu_request::MenuRequest;

/// The application menu's position in the menu bar.
const APP_MENU_INDEX: isize = 0;

/// "Check for Updates…" has no keyboard shortcut.
const NO_KEY_EQUIVALENT: &str = "";

struct TargetIvars {
    request: MenuRequest,
    ctx: egui::Context,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and the class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CoCoVMMenuTarget"]
    #[ivars = TargetIvars]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(raiseRequest:))]
        fn raise_request(&self, _sender: Option<&AnyObject>) {
            let ivars = self.ivars();
            ivars.request.raise();
            ivars.ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, ivars: TargetIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Retarget the app menu's About item to raise `about`, and insert "Check
/// for Updates…" after it, raising `update`. Call once, on the main thread,
/// after winit has built its default menu.
pub(crate) fn install(ctx: &egui::Context, about: MenuRequest, update: MenuRequest) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("app menu items not installed: not on the main thread");
        return;
    };
    let Some(app_menu) = app_menu(mtm) else {
        tracing::warn!("app menu items not installed: no application menu");
        return;
    };
    let Some(about_item) = standard_about_item(&app_menu) else {
        tracing::warn!("app menu items not installed: no standard About item in the app menu");
        return;
    };
    about_item.setTitle(&NSString::from_str(crate::about::MENU_LABEL));
    attach_target(mtm, &about_item, ctx, about);

    let title = NSString::from_str(crate::update::MENU_LABEL);
    // SAFETY: `raiseRequest:` is a valid selector, implemented by the target
    // attached below.
    let update_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &title,
            Some(sel!(raiseRequest:)),
            &NSString::from_str(NO_KEY_EQUIVALENT),
        )
    };
    attach_target(mtm, &update_item, ctx, update);
    app_menu.insertItem_atIndex(&update_item, app_menu.indexOfItem(&about_item) + 1);
}

/// Point `item` at a new target that raises `request`.
fn attach_target(
    mtm: MainThreadMarker,
    item: &NSMenuItem,
    ctx: &egui::Context,
    request: MenuRequest,
) {
    let target = MenuTarget::new(
        mtm,
        TargetIvars {
            request,
            ctx: ctx.clone(),
        },
    );
    // SAFETY: the target implements `raiseRequest:` with the action
    // signature, and is leaked below so the item's unretained reference
    // stays valid.
    unsafe {
        item.setTarget(Some(&target));
        item.setAction(Some(sel!(raiseRequest:)));
    }
    let _ = Retained::into_raw(target);
}

/// The menu under the menu bar's first (application) item.
fn app_menu(mtm: MainThreadMarker) -> Option<Retained<NSMenu>> {
    NSApplication::sharedApplication(mtm)
        .mainMenu()?
        .itemAtIndex(APP_MENU_INDEX)?
        .submenu()
}

/// The app menu item wired to AppKit's standard About panel.
fn standard_about_item(app_menu: &NSMenu) -> Option<Retained<NSMenuItem>> {
    app_menu
        .itemArray()
        .into_iter()
        .find(|item| item.action() == Some(sel!(orderFrontStandardAboutPanel:)))
}
