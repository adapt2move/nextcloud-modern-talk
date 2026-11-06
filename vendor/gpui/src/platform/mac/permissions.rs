//! NextCloud Modern Talk patch: permission APIs belong to the running application.
//! A sibling executable has a different code identity. Never request TCC or UN
//! authorization there and then treat its answer as authority for this process.
use super::dispatcher::{dispatch_get_main_queue, dispatch_sys::dispatch_async_f};
use block::ConcreteBlock;
use cocoa::base::{id, nil};
use objc::{
    class, msg_send,
    runtime::{BOOL, YES},
    sel, sel_impl,
};
use std::{ffi::c_void, sync::Arc};

#[derive(Clone, Copy)]
pub enum MacPermissionKind {
    Microphone,
    Camera,
    Screen,
    Notifications,
}
#[path = "permission_probe.rs"]
mod probe;
pub use probe::{MacPermissionError, MacPermissionOperation, MacPermissionStatus};

type MainJob = Box<dyn FnOnce() + Send>;
extern "C" fn run_main(raw: *mut c_void) {
    // Owned once by the main dispatch queue; the job contains no borrowed ObjC
    // object or caller stack pointer. Native objects never cross this boundary.
    let job = unsafe { Box::from_raw(raw.cast::<MainJob>()) };
    job();
}
pub fn macos_permission(
    kind: MacPermissionKind,
    request: bool,
) -> Result<MacPermissionOperation, MacPermissionError> {
    let index = match kind {
        MacPermissionKind::Microphone => 0,
        MacPermissionKind::Camera => 1,
        MacPermissionKind::Screen => 2,
        MacPermissionKind::Notifications => 3,
    };
    let (operation, callback) = probe::admit(index)?;
    let job: MainJob = Box::new(move || {
        if !callback.live() {
            return;
        }
        // UNUserNotificationCenter can raise an ObjC exception for an unbundled
        // executable. AV requests also require the app's usage descriptions.
        let bundled = unsafe {
            let bundle: id = msg_send![class!(NSBundle), mainBundle];
            let identifier: id = msg_send![bundle, bundleIdentifier];
            if identifier == nil {
                false
            } else {
                let equal: BOOL = msg_send![identifier, isEqualToString: super::ns_string("org.adapt2move.nextcloud-modern-talk")];
                equal == YES
            }
        };
        if !bundled {
            callback.finish(Err(MacPermissionError::Unavailable));
            return;
        }
        // All requests use the actual NSApplication's main thread. Merely
        // checking never calls an authorization API and never opens Settings.
        match kind {
            MacPermissionKind::Notifications => unsafe { notifications(callback, request) },
            MacPermissionKind::Screen => {
                let requested = request && unsafe { CGRequestScreenCaptureAccess() };
                let granted = unsafe { CGPreflightScreenCaptureAccess() };
                callback.finish(Ok(if granted {
                    MacPermissionStatus::Granted
                } else if requested {
                    MacPermissionStatus::RestartRequired
                } else {
                    MacPermissionStatus::Unknown
                }));
            }
            kind => unsafe { media(callback, kind, request) },
        }
    });
    unsafe {
        dispatch_async_f(
            dispatch_get_main_queue(),
            Box::into_raw(Box::new(job)).cast(),
            Some(run_main),
        );
    }
    Ok(operation)
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

fn media_status(value: i64) -> MacPermissionStatus {
    match value {
        0 => MacPermissionStatus::NotDetermined,
        1 => MacPermissionStatus::Restricted,
        2 => MacPermissionStatus::Denied,
        3 => MacPermissionStatus::Granted,
        _ => MacPermissionStatus::Unknown,
    }
}
unsafe fn media(probe: Arc<Probe>, kind: MacPermissionKind, request: bool) {
    unsafe {
        let device = class!(AVCaptureDevice);
        let media = super::ns_string(match kind {
            MacPermissionKind::Camera => "vide",
            _ => "soun",
        });
        let status: i64 = msg_send![device, authorizationStatusForMediaType: media];
        if request && status == 0 {
            let block = ConcreteBlock::new(move |_: BOOL| {
                let media = super::ns_string(match kind {
                    MacPermissionKind::Camera => "vide",
                    _ => "soun",
                });
                let status: i64 =
                    msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: media];
                probe.finish(Ok(media_status(status)));
            })
            .copy();
            let _: () =
                msg_send![device, requestAccessForMediaType: media completionHandler: &*block];
        } else {
            probe.finish(Ok(media_status(status)));
        }
    }
}
fn notification_status(value: i64) -> MacPermissionStatus {
    match value {
        0 => MacPermissionStatus::NotDetermined,
        1 => MacPermissionStatus::Denied,
        2 | 3 => MacPermissionStatus::Granted,
        _ => MacPermissionStatus::Unknown,
    }
}
unsafe fn notifications(probe: Arc<Probe>, request: bool) {
    unsafe {
        let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let block = ConcreteBlock::new(move |settings: id| {
            let status: i64 = msg_send![settings, authorizationStatus];
            if request && status == 0 && probe.live() {
                let probe = probe.clone();
                let done = ConcreteBlock::new(move |_: BOOL, error: id| {
                    if error != nil { probe.finish(Err(MacPermissionError::Failed)); }
                    else { notifications(probe.clone(), false); }
                }).copy();
                let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
                let _: () = msg_send![center, requestAuthorizationWithOptions: 6usize completionHandler: &*done];
            } else { probe.finish(Ok(notification_status(status))); }
        }).copy();
        let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*block];
    }
}
