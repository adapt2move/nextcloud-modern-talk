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
        let accounts = Rc::downgrade(accounts);
        let finished = self.finished.clone();
        self.task = Some(match &picker.kind {
            Kind::Upload | Kind::Csv { .. } => {
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
                        .and_then(one_file);
                    if let Some(accounts) = accounts.upgrade() {
                        accounts.borrow_mut().finish_picker(picker, path);
                    }
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
                    if let Some(accounts) = accounts.upgrade() {
                        accounts.borrow_mut().finish_picker(picker, path);
                    }
                    finished.set(true);
                })
            }
        });
    }
}
fn one_file(paths: Option<Vec<std::path::PathBuf>>) -> Result<Option<std::path::PathBuf>, ()> {
    match paths {
        None => Ok(None),
        Some(paths) if paths.len() <= 1 => Ok(paths.into_iter().next()),
        Some(_) => Err(()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_selection_never_silently_takes_first_of_multiple_files() {
        assert_eq!(one_file(None), Ok(None));
        assert_eq!(one_file(Some(Vec::new())), Ok(None));
        assert_eq!(
            one_file(Some(vec!["one.csv".into()])),
            Ok(Some("one.csv".into()))
        );
        assert_eq!(
            one_file(Some(vec!["one.csv".into(), "two.csv".into()])),
            Err(())
        );
    }
}
