//! NextCloud Modern Talk patch: one copy-only, asynchronous AppKit file promise.
//! AppKit invokes the delegate on the main queue; file I/O stays with the writer.
use super::ns_string;
use crate::{ForegroundExecutor, SharedString, Task, Timer};
use block::{Block, RcBlock};
use cocoa::{
    base::{id, nil},
    foundation::{NSInteger, NSPoint, NSRect, NSSize, NSUInteger},
};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    rc::StrongPtr,
    runtime::{BOOL, Class, Object, Protocol, Sel, YES},
    sel, sel_impl,
};
use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, OsStr, c_void},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

type NSDragOperation = NSUInteger;
const COPY: NSDragOperation = 1;
type Writer = Box<dyn FnOnce(PathBuf) -> Task<bool>>;
const IVAR: &str = "nmtFilePromiseState";
struct State {
    name: String,
    writer: RefCell<Option<Writer>>,
    ended: RefCell<Option<Box<dyn FnOnce()>>>,
    task: RefCell<Option<Task<()>>>,
    finished: Rc<Cell<bool>>,
    executor: ForegroundExecutor,
    created: Instant,
}
pub(super) struct Session {
    _source: StrongPtr,
    state: Rc<State>,
}
impl Session {
    pub(super) fn replaceable(&self) -> bool {
        self.state.finished.get()
            || self.state.task.borrow().is_none()
                && self.state.created.elapsed() >= Duration::from_secs(120)
    }
    pub(super) fn start(
        view: id,
        executor: ForegroundExecutor,
        name: SharedString,
        writer: Writer,
        ended: Box<dyn FnOnce()>,
    ) -> Option<Self> {
        if name.is_empty()
            || name.len() > 255
            || matches!(name.as_ref(), "." | "..")
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        {
            return None;
        }
        unsafe {
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            let event: id = msg_send![app, currentEvent];
            if event.is_null() {
                return None;
            }
            let source = StrongPtr::new(msg_send![source_class(), new]);
            if source.is_null() {
                return None;
            }
            let state = Rc::new(State {
                name: name.to_string(),
                writer: RefCell::new(Some(writer)),
                ended: RefCell::new(Some(ended)),
                task: RefCell::new(None),
                finished: Rc::new(Cell::new(false)),
                executor,
                created: Instant::now(),
            });
            (**source).set_ivar(IVAR, Box::into_raw(Box::new(state.clone())) as *mut c_void);
            let provider: id = msg_send![class!(NSFilePromiseProvider), alloc];
            let provider = StrongPtr::new(
                msg_send![provider, initWithFileType: ns_string("public.data") delegate: *source],
            );
            if provider.is_null() {
                return None;
            }
            let item: id = msg_send![class!(NSDraggingItem), alloc];
            let item = StrongPtr::new(msg_send![item, initWithPasteboardWriter: *provider]);
            if item.is_null() {
                return None;
            }
            let location: NSPoint = msg_send![event, locationInWindow];
            let point: NSPoint = msg_send![view, convertPoint: location fromView: nil];
            let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let extension = std::path::Path::new(name.as_ref())
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("");
            let icon: id = msg_send![workspace, iconForFileType: ns_string(extension)];
            let frame = NSRect::new(
                NSPoint::new(point.x - 16., point.y - 16.),
                NSSize::new(32., 32.),
            );
            let _: () = msg_send![*item, setDraggingFrame: frame contents: icon];
            let items: id = msg_send![class!(NSArray), arrayWithObject: *item];
            let session: id =
                msg_send![view, beginDraggingSessionWithItems: items event: event source: *source];
            if session.is_null() {
                return None;
            }
            Some(Self {
                _source: source,
                state,
            })
        }
    }
}
fn source_class() -> &'static Class {
    if let Some(class) = Class::get("NMTGPUIFilePromiseSource") {
        return class;
    }
    // This function runs on AppKit's main thread, as do all delegate callbacks.
    let mut class = ClassDecl::new("NMTGPUIFilePromiseSource", class!(NSObject))
        .expect("unique file promise class");
    class.add_ivar::<*mut c_void>(IVAR);
    class.add_protocol(
        Protocol::get("NSFilePromiseProviderDelegate").expect("AppKit file promises"),
    );
    class.add_protocol(Protocol::get("NSDraggingSource").expect("AppKit dragging"));
    unsafe {
        class.add_method(
            sel!(filePromiseProvider:fileNameForType:),
            file_name as extern "C" fn(&Object, Sel, id, id) -> id,
        );
        class.add_method(
            sel!(filePromiseProvider:writePromiseToURL:completionHandler:),
            write_file as extern "C" fn(&Object, Sel, id, id, id),
        );
        class.add_method(
            sel!(operationQueueForFilePromiseProvider:),
            operation_queue as extern "C" fn(&Object, Sel, id) -> id,
        );
        class.add_method(
            sel!(draggingSession:sourceOperationMaskForDraggingContext:),
            operation_mask as extern "C" fn(&Object, Sel, id, NSInteger) -> NSDragOperation,
        );
        class.add_method(
            sel!(ignoreModifierKeysForDraggingSession:),
            ignore_modifiers as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        class.add_method(
            sel!(draggingSession:endedAtPoint:operation:),
            ended as extern "C" fn(&Object, Sel, id, NSPoint, NSDragOperation),
        );
        class.add_method(sel!(dealloc), dealloc as extern "C" fn(&mut Object, Sel));
    }
    class.register()
}
unsafe fn state(object: &Object) -> Rc<State> {
    let pointer: *mut c_void = unsafe { *object.get_ivar(IVAR) };
    unsafe { (*(pointer as *mut Rc<State>)).clone() }
}
extern "C" fn file_name(object: &Object, _: Sel, _: id, _: id) -> id {
    unsafe { ns_string(&state(object).name) }
}
extern "C" fn operation_queue(_: &Object, _: Sel, _: id) -> id {
    unsafe { msg_send![class!(NSOperationQueue), mainQueue] }
}
extern "C" fn operation_mask(_: &Object, _: Sel, _: id, _: NSInteger) -> NSDragOperation {
    COPY
}
extern "C" fn ignore_modifiers(_: &Object, _: Sel, _: id) -> BOOL {
    YES
}
extern "C" fn ended(object: &Object, _: Sel, _: id, _: NSPoint, operation: NSDragOperation) {
    let state = unsafe { state(object) };
    let callback = state.ended.borrow_mut().take();
    if let Some(callback) = callback {
        callback();
    }
    if operation != COPY && state.task.borrow().is_none() {
        state.writer.borrow_mut().take();
        state.finished.set(true);
    }
}
unsafe fn error() -> id {
    unsafe {
        msg_send![class!(NSError), errorWithDomain: ns_string("GPUIFilePromise") code: 1isize userInfo: nil]
    }
}
extern "C" fn write_file(object: &Object, _: Sel, _: id, url: id, completion: id) {
    if completion.is_null() {
        return;
    }
    let completion = unsafe { RcBlock::<(id,), ()>::copy(completion as *mut Block<(id,), ()>) };
    let state = unsafe { state(object) };
    let path = unsafe {
        let is_file: BOOL = msg_send![url, isFileURL];
        let raw: *const std::ffi::c_char = if is_file == YES {
            msg_send![url, fileSystemRepresentation]
        } else {
            std::ptr::null()
        };
        if raw.is_null() {
            None
        } else {
            Some(PathBuf::from(OsStr::from_bytes(
                CStr::from_ptr(raw).to_bytes(),
            )))
        }
    };
    let writer = state.writer.borrow_mut().take();
    let Some((path, writer)) = path
        .zip(writer)
        .filter(|_| state.created.elapsed() < Duration::from_secs(120))
    else {
        if state.task.borrow().is_none() {
            state.finished.set(true);
        }
        unsafe {
            completion.call((error(),));
        }
        return;
    };
    let task = writer(path);
    let finished = state.finished.clone();
    // The callback itself never blocks the main queue. The source owns this
    // finite task and the copied AppKit completion block until publication.
    let task = state.executor.spawn(async move {
        let result = futures::future::select(
            Box::pin(task),
            Box::pin(Timer::after(Duration::from_secs(120))),
        )
        .await;
        let saved = matches!(result, futures::future::Either::Left((true, _)));
        unsafe {
            completion.call((if saved { nil } else { error() },));
        }
        finished.set(true);
    });
    *state.task.borrow_mut() = Some(task);
}
extern "C" fn dealloc(object: &mut Object, _: Sel) {
    unsafe {
        let pointer: *mut c_void = *object.get_ivar(IVAR);
        if !pointer.is_null() {
            drop(Box::from_raw(pointer as *mut Rc<State>));
        }
        let _: () = msg_send![super(object, class!(NSObject)), dealloc];
    }
}
