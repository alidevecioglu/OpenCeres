//! The macOS menu bar (OpenCeres, File, View, Window, Help). Every app item
//! sends a [`Cmd`] that the app runs like the matching toolbar button or key,
//! so the menu adds no logic of its own; the system items (Hide, Quit,
//! Minimise, Full Screen…) use AppKit's standard actions. Other systems have
//! no menu bar, so there [`install`] does nothing and [`take`] is always empty.
//!
//! Only ⌘ shortcuts are given to AppKit as key equivalents. A menu item takes
//! its key before the window sees it, so a plain-letter equivalent such as K
//! would stop that letter reaching a text field; the single-key shortcuts stay
//! with the app's own key handling and are listed in its help instead.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// What a menu item asks the app to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    Settings,
    CheckUpdates,
    Open,
    OpenFolder,
    History,
    Play,
    PrevFrame,
    NextFrame,
    Slower,
    Faster,
    Stars,
    Invert,
    Align,
    Grid,
    Known,
    Mark,
    ZoomIn,
    ZoomOut,
    Fit,
    SidePanel,
    Shortcuts,
    Welcome,
    Website,
    ReportProblem,
    ExportJson,
    ImportJson,
    ReviewReport,
}

impl Cmd {
    const ALL: [Cmd; 27] = [
        Cmd::Settings,
        Cmd::CheckUpdates,
        Cmd::Open,
        Cmd::OpenFolder,
        Cmd::History,
        Cmd::Play,
        Cmd::PrevFrame,
        Cmd::NextFrame,
        Cmd::Slower,
        Cmd::Faster,
        Cmd::Stars,
        Cmd::Invert,
        Cmd::Align,
        Cmd::Grid,
        Cmd::Known,
        Cmd::Mark,
        Cmd::ZoomIn,
        Cmd::ZoomOut,
        Cmd::Fit,
        Cmd::SidePanel,
        Cmd::Shortcuts,
        Cmd::Welcome,
        Cmd::Website,
        Cmd::ReportProblem,
        Cmd::ExportJson,
        Cmd::ImportJson,
        Cmd::ReviewReport,
    ];

    fn bit(self) -> u64 {
        1 << self as u32
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn from_tag(tag: isize) -> Option<Cmd> {
        Cmd::ALL.iter().copied().find(|c| *c as isize == tag)
    }
}

/// Commands chosen in the menu since the app last asked.
static PENDING: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());
/// Which items are enabled and which are ticked, one bit per [`Cmd`]; AppKit
/// reads these when it opens a menu.
static ENABLED: AtomicU64 = AtomicU64::new(u64::MAX);
static CHECKED: AtomicU64 = AtomicU64::new(0);

/// Takes the commands chosen in the menu since the last call.
pub fn take() -> Vec<Cmd> {
    PENDING
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default()
}

/// Records each item's (enabled, ticked) state for the next time a menu opens.
pub fn sync(state: impl Fn(Cmd) -> (bool, bool)) {
    let (mut enabled, mut checked) = (0, 0);
    for cmd in Cmd::ALL {
        let (e, c) = state(cmd);
        if e {
            enabled |= cmd.bit();
        }
        if c {
            checked |= cmd.bit();
        }
    }
    ENABLED.store(enabled, Ordering::Relaxed);
    CHECKED.store(checked, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn install(_ctx: &eframe::egui::Context) {}

#[cfg(target_os = "macos")]
pub use mac::install;

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use eframe::egui;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, Sel};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
        NSMenuItem,
    };
    use objc2_foundation::NSString;

    use super::{CHECKED, Cmd, ENABLED, PENDING};

    /// Wakes the app when an item is chosen (the app only repaints on input).
    static CTX: OnceLock<egui::Context> = OnceLock::new();

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Target
        // implements no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenCeresMenuTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(menuAction:))]
            fn menu_action(&self, sender: &NSMenuItem) {
                if let Some(cmd) = Cmd::from_tag(sender.tag())
                    && let Ok(mut pending) = PENDING.lock()
                {
                    pending.push(cmd);
                }
                if let Some(ctx) = CTX.get() {
                    ctx.request_repaint();
                }
            }

            #[unsafe(method(validateMenuItem:))]
            fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
                validate(item)
            }
        }
    );

    /// Ticks `item` and says whether it is enabled, from the app's last [`super::sync`].
    fn validate(item: &NSMenuItem) -> bool {
        let Some(cmd) = Cmd::from_tag(item.tag()) else {
            return true;
        };
        let checked = CHECKED.load(Ordering::Relaxed) & cmd.bit() != 0;
        item.setState(if checked {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        ENABLED.load(Ordering::Relaxed) & cmd.bit() != 0
    }

    /// Builds the menu bar and makes it the app's, replacing winit's default.
    pub fn install(ctx: &egui::Context) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let _ = CTX.set(ctx.clone());
        // SAFETY: plain NSObject init of a class without instance variables.
        let target: Retained<Target> =
            unsafe { msg_send![super(Target::alloc(mtm).set_ivars(())), init] };
        let m = Menus {
            mtm,
            target: &target,
        };
        let app = NSApplication::sharedApplication(mtm);
        let bar = NSMenu::new(mtm);

        let services = NSMenu::new(mtm);
        let app_menu = m.menu(
            &bar,
            "OpenCeres",
            &[
                Item::System("About OpenCeres", sel!(orderFrontStandardAboutPanel:), ""),
                Item::Separator,
                Item::App("Settings…", Cmd::Settings, ","),
                Item::App("Check for Updates…", Cmd::CheckUpdates, ""),
                Item::Separator,
            ],
        );
        let services_item = m.item("Services", None, "");
        services_item.setSubmenu(Some(&services));
        app_menu.addItem(&services_item);
        m.add(
            &app_menu,
            &[
                Item::Separator,
                Item::System("Hide OpenCeres", sel!(hide:), "h"),
                Item::SystemAlt("Hide Others", sel!(hideOtherApplications:), "h"),
                Item::System("Show All", sel!(unhideAllApplications:), ""),
                Item::Separator,
                Item::System("Quit OpenCeres", sel!(terminate:), "q"),
            ],
        );
        m.menu(
            &bar,
            "File",
            &[
                Item::App("Open Image Set…", Cmd::Open, "o"),
                Item::AppShift("Open Folder…", Cmd::OpenFolder, "o"),
                Item::Separator,
                Item::App("Export as JSON…", Cmd::ExportJson, ""),
                Item::App("Import JSON…", Cmd::ImportJson, ""),
                Item::App("Review & Copy Report…", Cmd::ReviewReport, ""),
                Item::Separator,
                Item::App("Search History", Cmd::History, ""),
                Item::Separator,
                Item::System("Close Window", sel!(performClose:), "w"),
            ],
        );
        m.menu(
            &bar,
            "View",
            &[
                Item::App("Blink", Cmd::Play, ""),
                Item::App("Previous Frame", Cmd::PrevFrame, ""),
                Item::App("Next Frame", Cmd::NextFrame, ""),
                Item::App("Blink Slower", Cmd::Slower, ""),
                Item::App("Blink Faster", Cmd::Faster, ""),
                Item::Separator,
                Item::App("Detected Stars", Cmd::Stars, ""),
                Item::App("Invert", Cmd::Invert, ""),
                Item::App("Align Frames", Cmd::Align, ""),
                Item::App("Report Grid", Cmd::Grid, ""),
                Item::App("Known Objects", Cmd::Known, ""),
                Item::App("Mark Moving Object", Cmd::Mark, ""),
                Item::Separator,
                Item::App("Zoom In", Cmd::ZoomIn, ""),
                Item::App("Zoom Out", Cmd::ZoomOut, ""),
                Item::App("Fit to Window", Cmd::Fit, ""),
                Item::Separator,
                Item::App("Side Panel", Cmd::SidePanel, ""),
                Item::Separator,
                Item::SystemCtrl("Enter Full Screen", sel!(toggleFullScreen:), "f"),
            ],
        );
        let window = m.menu(
            &bar,
            "Window",
            &[
                Item::System("Minimize", sel!(performMiniaturize:), "m"),
                Item::System("Zoom", sel!(performZoom:), ""),
            ],
        );
        let help = m.menu(
            &bar,
            "Help",
            &[
                Item::App("Keyboard Shortcuts", Cmd::Shortcuts, ""),
                Item::App("Welcome…", Cmd::Welcome, ""),
                Item::Separator,
                Item::App("OpenCeres Website", Cmd::Website, ""),
                Item::App("Report a Problem…", Cmd::ReportProblem, ""),
            ],
        );
        app.setMainMenu(Some(&bar));
        app.setServicesMenu(Some(&services));
        app.setWindowsMenu(Some(&window));
        app.setHelpMenu(Some(&help));
        // Menu items hold their target weakly; the menu lives as long as the
        // app, so its target does too.
        std::mem::forget(target);
    }

    enum Item {
        /// An app command with an optional ⌘ key.
        App(&'static str, Cmd, &'static str),
        /// An app command with a ⇧⌘ key.
        AppShift(&'static str, Cmd, &'static str),
        /// A standard AppKit action, sent along the responder chain, ⌘ key.
        System(&'static str, Sel, &'static str),
        /// The same with ⌥⌘.
        SystemAlt(&'static str, Sel, &'static str),
        /// The same with ⌃⌘.
        SystemCtrl(&'static str, Sel, &'static str),
        Separator,
    }

    struct Menus<'a> {
        mtm: MainThreadMarker,
        target: &'a Target,
    }

    impl Menus<'_> {
        /// A top-level menu called `title` with `items`, added to `bar`.
        fn menu(&self, bar: &NSMenu, title: &str, items: &[Item]) -> Retained<NSMenu> {
            let menu = NSMenu::initWithTitle(NSMenu::alloc(self.mtm), &NSString::from_str(title));
            self.add(&menu, items);
            let top = self.item(title, None, "");
            top.setSubmenu(Some(&menu));
            bar.addItem(&top);
            menu
        }

        fn add(&self, menu: &NSMenu, items: &[Item]) {
            let cmd = NSEventModifierFlags::Command;
            for item in items {
                let made = match *item {
                    Item::Separator => NSMenuItem::separatorItem(self.mtm),
                    Item::App(title, c, key) | Item::AppShift(title, c, key) => {
                        let i = self.item(title, Some(sel!(menuAction:)), key);
                        i.setTag(c as isize);
                        // SAFETY: the target answers menuAction: and outlives the menu.
                        unsafe { i.setTarget(Some(self.target as &AnyObject)) };
                        if matches!(item, Item::AppShift(..)) {
                            i.setKeyEquivalentModifierMask(cmd | NSEventModifierFlags::Shift);
                        }
                        i
                    }
                    Item::System(title, action, key) => self.item(title, Some(action), key),
                    Item::SystemAlt(title, action, key) => {
                        let i = self.item(title, Some(action), key);
                        i.setKeyEquivalentModifierMask(cmd | NSEventModifierFlags::Option);
                        i
                    }
                    Item::SystemCtrl(title, action, key) => {
                        let i = self.item(title, Some(action), key);
                        i.setKeyEquivalentModifierMask(cmd | NSEventModifierFlags::Control);
                        i
                    }
                };
                menu.addItem(&made);
            }
        }

        fn item(&self, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
            // SAFETY: the actions are AppKit's standard ones or menuAction:,
            // which the target implements.
            unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.mtm),
                    &NSString::from_str(title),
                    action,
                    &NSString::from_str(key),
                )
            }
        }
    }
}
