use objc2_app_kit::{NSApplication, NSEventTrackingRunLoopMode};
use objc2_foundation::{MainThreadMarker, NSRunLoop, NSString};

/// Per-frame macOS menu upkeep; call at the top of `eframe::App::ui`.
pub fn tick(app_name: &str, version: &str) {
    use std::cell::RefCell;

    if MainThreadMarker::new().is_none() {
        return;
    }
    // muda's Menu is main-thread-only (not Send), so the keep-alive handle
    // lives in a thread_local rather than a static.
    thread_local! {
        static MENU: RefCell<Option<muda::Menu>> = const { RefCell::new(None) };
    }
    MENU.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let menu = muda::Menu::new();
            let _ = menu.append(&standard_app_submenu(app_name, version, None));
            menu.init_for_nsapp();
            *slot = Some(menu);
        }
    });
    ensure_app_menu_title(app_name);
    while muda::MenuEvent::receiver().try_recv().is_ok() {}
}

/// Whether this process was launched from inside a real `.app` bundle (its
/// Info.plist declares a bundle identifier). Bundled runs need none of the
/// bare-binary repairs in this module: AppKit derives the bold title, Dock
/// tile, and Dock/Cmd-Tab name from the bundle itself. Bare runs
/// (`cargo run`, a loose release binary) have no identifier and keep them.
pub fn running_in_app_bundle() -> bool {
    use objc2_foundation::NSBundle;
    static BUNDLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // SAFETY: read-only property query on the main bundle; NSBundle is
    // thread-safe.
    *BUNDLED.get_or_init(|| unsafe { NSBundle::mainBundle().bundleIdentifier().is_some() })
}

/// Build the standard macOS application submenu: About, Services, Hide / Hide
/// Others / Show All, and Quit.
pub fn standard_app_submenu(
    app_name: &str,
    version: &str,
    icon: Option<muda::Icon>,
) -> muda::Submenu {
    use muda::{AboutMetadata, PredefinedMenuItem, Submenu};

    let submenu = Submenu::new(app_name, true);
    let _ = submenu.append_items(&[
        &PredefinedMenuItem::about(
            Some(&format!("About {app_name}")),
            Some(AboutMetadata {
                name: Some(app_name.to_string()),
                version: Some(version.to_string()),
                icon,
                ..AboutMetadata::default()
            }),
        ),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some(&format!("Hide {app_name}"))),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(Some(&format!("Quit {app_name}"))),
    ]);
    submenu
}

pub fn ensure_app_menu_title(title: &str) {
    if running_in_app_bundle() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: read-only query of the main run loop's current mode, on the
    // main thread.
    let tracking = unsafe {
        NSRunLoop::mainRunLoop()
            .currentMode()
            .is_some_and(|mode| *mode == *NSEventTrackingRunLoopMode)
    };
    if tracking {
        return;
    }
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: read-only menu traversal plus title sets, on the main thread.
    // `itemAtIndex:` raises (rather than returning nil) when out of range,
    // hence the `numberOfItems` guard.)
    unsafe {
        let Some(main_menu) = app.mainMenu() else {
            return;
        };
        if main_menu.numberOfItems() == 0 {
            return;
        }
        let Some(item) = main_menu.itemAtIndex(0) else {
            return;
        };
        let Some(app_menu) = item.submenu() else {
            return;
        };
        app_menu.setTitle(&NSString::new());
        app_menu.setTitle(&NSString::from_str(title));
    }
}
