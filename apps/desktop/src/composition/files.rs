//! One native file dialog per window; late selections cannot cross account/room boundaries.
use super::accounts::{Accounts, files::Kind};
use gpui::{App, Task};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};
pub(super) struct Dialogs {
    task: Option<Task<()>>,
    finished: Rc<Cell<bool>>,
}
impl Dialogs {
    pub fn new() -> Self {
        Self {
            task: None,
            finished: Rc::new(Cell::new(false)),
        }
    }
    pub fn poll(&mut self, accounts: &Rc<RefCell<Accounts>>, cx: &mut App) {
        if self.finished.replace(false) {
            self.task = None;
        }
        if self.task.is_some() {
            return;
        }
        let Some(picker) = accounts.borrow_mut().take_picker() else {
            return;
        };
        let accounts = accounts.clone();
        let finished = self.finished.clone();
        self.task = Some(match &picker.kind {
            Kind::Upload => {
                let response = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: None,
                });
                cx.spawn(async move |_| {
                    let path = response
                        .await
                        .map_err(|_| ())
                        .and_then(|r| r.map_err(|_| ()))
                        .map(|p| p.and_then(|p| p.into_iter().next()));
                    accounts.borrow_mut().finish_picker(picker, path);
                    finished.set(true);
                })
            }
            Kind::Download { name, .. } => {
                let response = cx.prompt_for_new_path(Path::new(""), Some(name.as_str()));
                cx.spawn(async move |_| {
                    let path = response
                        .await
                        .map_err(|_| ())
                        .and_then(|r| r.map_err(|_| ()));
                    accounts.borrow_mut().finish_picker(picker, path);
                    finished.set(true);
                })
            }
        });
    }
}
