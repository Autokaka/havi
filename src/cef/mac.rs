//! NSApplication subclass conforming to CefAppProtocol — required before
//! `cef_initialize` or it deadlocks.

use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::runtime::{Bool, NSObjectProtocol};
use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadMarker};
use objc2_app_kit::{NSApp, NSApplication, NSEvent};
use std::cell::Cell;

#[derive(Default)]
pub struct CapApplicationIvars {
    handling_send_event: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSApplication))]
    #[ivars = CapApplicationIvars]
    pub struct CapApplication;

    impl CapApplication {
        #[unsafe(method(sendEvent:))]
        unsafe fn send_event(&self, event: &NSEvent) {
            let was = self.ivars().handling_send_event.get();
            if !was {
                self.ivars().handling_send_event.set(true);
            }
            let _: () = unsafe { msg_send![super(self), sendEvent: event] };
            if !was {
                self.ivars().handling_send_event.set(false);
            }
        }
    }

    unsafe impl NSObjectProtocol for CapApplication {}

    unsafe impl CrAppControlProtocol for CapApplication {
        #[unsafe(method(setHandlingSendEvent:))]
        unsafe fn set_handling_send_event(&self, handling: Bool) {
            self.ivars().handling_send_event.set(handling.as_bool());
        }
    }

    unsafe impl CrAppProtocol for CapApplication {
        #[unsafe(method(isHandlingSendEvent))]
        unsafe fn is_handling_send_event(&self) -> Bool {
            Bool::new(self.ivars().handling_send_event.get())
        }
    }

    unsafe impl CefAppProtocol for CapApplication {}
);

pub fn setup_application() {
    let mtm = MainThreadMarker::new().expect("setup_application must run on the main thread");
    let _: objc2::rc::Retained<CapApplication> =
        unsafe { msg_send![CapApplication::class(), sharedApplication] };
    // If anything touched NSApp first, it'd be a plain NSApplication.
    assert!(
        NSApp(mtm).isKindOfClass(CapApplication::class()),
        "NSApp is not a CapApplication — something created NSApp too early"
    );
}
