//! Native account files surface. The account owner supplies bounded metadata.
use super::*;
use gpui::{FontWeight, SharedString};

pub(super) const PAGE_SIZE: usize = 40;

fn child_path(parent: &str, name: &str) -> Option<Text<16384>> {
    if name.is_empty() || name.contains(['/', '\\']) || matches!(name, "." | "..") {
        return None;
    }
    let path = if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    };
    Text::new(&path).ok()
}
fn parent_path(path: &str) -> Option<Text<16384>> {
    if path == "/" {
        return None;
    }
    let end = path.rfind('/')?;
    Text::new(if end == 0 { "/" } else { &path[..end] }).ok()
}
fn size_label(size: Option<u64>) -> String {
    match size {
        None => String::new(),
        Some(n) if n < 1024 => format!("{n} B"),
        Some(n) if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.),
        Some(n) => format!("{:.1} MB", n as f64 / (1024. * 1024.)),
    }
}
impl Desktop {
    pub(in crate::presentation) fn cloud_files_body(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        let Some(browser) = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| Some(browser.account) == self.snapshot.active_account)
        else {
            return body.child(self.hint(self.l("Dateien werden geladen …")));
        };
        let path = browser.path.as_str();
        let parent = parent_path(path);
        let root = Text::new("/").expect("constant");
        let mut crumbs = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_1()
            .child(self.button(
                "cloud-root",
                "Alle Dateien",
                ButtonStyle::Quiet,
                path != "/",
                LocalAction::Content(Action::BrowseDirectory(root)),
                cx,
            ));
        let mut current = String::new();
        let segments: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
        for (index, part) in segments.iter().enumerate() {
            current.push('/');
            current.push_str(part);
            if index + 12 < segments.len() {
                continue;
            }
            let Ok(target) = Text::new(&current) else {
                break;
            };
            let id: SharedString = format!("cloud-crumb-{current}").into();
            crumbs = crumbs
                .child(div().text_color(self.theme.muted).child("/"))
                .child(
                    self.control(
                        id,
                        ButtonStyle::Quiet,
                        target.as_str() != path,
                        LocalAction::Content(Action::BrowseDirectory(target)),
                        cx,
                    )
                    .child((*part).to_owned()),
                );
        }
        body = body.child(crumbs);
        if browser.offline {
            body = body.child(self.hint(self.l("Offline · gespeicherte Dateien")));
        } else if browser.loading {
            body = body.child(self.hint(self.l("Dateien werden geladen …")));
        }
        if browser.error.is_some() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.hint(self.l("Dateien konnten nicht geladen werden.")))
                    .child(self.button(
                        "cloud-retry",
                        "Erneut versuchen",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Content(Action::BrowseDirectory(browser.path.clone())),
                        cx,
                    )),
            );
        }
        if let Some(parent) = parent {
            body = body.child(self.button(
                "cloud-parent",
                "Eine Ebene nach oben",
                ButtonStyle::Quiet,
                true,
                LocalAction::Content(Action::BrowseDirectory(parent)),
                cx,
            ));
        }
        let entries = browser.entries.as_slice();
        if entries.is_empty() && !browser.loading && browser.error.is_none() {
            body = body.child(self.hint(self.l("Dieser Ordner ist leer.")));
        }
        let start = self
            .content_ui
            .browser_page
            .saturating_mul(PAGE_SIZE)
            .min(entries.len());
        for entry in entries.iter().skip(start).take(PAGE_SIZE) {
            let mut row = div()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(self.theme.border)
                .child(self.scaled_icon(
                    if entry.directory { "inbox" } else { "file" },
                    18.,
                    self.theme.muted,
                ));
            if entry.directory {
                if let Some(path) = child_path(path, entry.name.as_str()) {
                    let id: SharedString = format!("cloud-dir-{}", path.as_str()).into();
                    row = row.child(
                        self.control(
                            id,
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Content(Action::BrowseDirectory(path)),
                            cx,
                        )
                        .flex_1()
                        .min_w_0()
                        .justify_start()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(entry.name.as_str().to_owned()),
                        ),
                    );
                }
            } else {
                row = row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(entry.name.as_str().to_owned()),
                );
            }
            row = row.child(
                div()
                    .text_size(self.sp(12.))
                    .text_color(self.theme.muted)
                    .child(size_label(entry.size)),
            );
            if let Some(modified) = &entry.modified {
                row = row.child(
                    div()
                        .text_size(self.sp(12.))
                        .text_color(self.theme.muted)
                        .child(modified.as_str().to_owned()),
                );
            }
            if !entry.directory
                && let Some(context) = self
                    .snapshot
                    .selected
                    .filter(|context| context.account == browser.account)
            {
                let id: SharedString = format!("cloud-add-{}-{}", path, entry.name.as_str()).into();
                row = row.child(self.button(
                    id,
                    "Zur Auswahl hinzufügen",
                    ButtonStyle::Quiet,
                    self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Files)
                        && !browser.offline,
                    LocalAction::Content(Action::Command(ContentAction::StageCloudFile {
                        context,
                        directory: browser.path.clone(),
                        name: entry.name.clone(),
                    })),
                    cx,
                ));
            }
            body = body.child(row);
        }
        if entries.len() > PAGE_SIZE {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(self.button(
                        "cloud-prev",
                        "Zurück",
                        ButtonStyle::Quiet,
                        start > 0,
                        LocalAction::Content(Action::BrowserPage(-1)),
                        cx,
                    ))
                    .child(div().text_color(self.theme.muted).child(format!(
                        "{}–{} / {}",
                        start + 1,
                        (start + PAGE_SIZE).min(entries.len()),
                        entries.len()
                    )))
                    .child(self.button(
                        "cloud-next",
                        "Weiter",
                        ButtonStyle::Quiet,
                        start + PAGE_SIZE < entries.len(),
                        LocalAction::Content(Action::BrowserPage(1)),
                        cx,
                    )),
            );
        }
        body
    }
}
