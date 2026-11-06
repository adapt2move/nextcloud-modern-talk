//! Native account files surface. The account owner supplies bounded metadata.
use super::*;
use gpui::{FontWeight, KeyDownEvent, SharedString};

pub(super) const PAGE_SIZE: usize = 40;

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
    fn cloud_can_stage(&self, browser: &talk_core::ui::content::FileBrowserView) -> bool {
        !browser.offline
            && !self.update_restart_pending
            && !self.projection_blocked
            && self.snapshot.content == ContentState::Ready
            && self
                .snapshot
                .selected
                .is_some_and(|context| context.account == browser.account)
            && self
                .selected()
                .is_some_and(|room| room.send == talk_core::ui::Decision::Allowed)
            && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Files)
    }
    pub(in crate::presentation) fn cloud_browser_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.overlay != Some(Overlay::CloudFiles)
            || self
                .content_ui
                .browser_search
                .focus_handle(cx)
                .is_focused(window)
        {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return false;
        }
        let Some(browser) = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| Some(browser.account) == self.snapshot.active_account)
        else {
            return false;
        };
        if key == "backspace" {
            if browser.section == talk_core::ui::content::FileSection::All
                && let Some(parent) = parent_path(browser.path.as_str())
            {
                self.content_action(Action::BrowseDirectory(parent), window, cx);
            }
            return true;
        }
        if key == "enter" {
            let entry = self
                .content_ui
                .browser_cursor
                .as_ref()
                .and_then(|path| {
                    browser
                        .entries
                        .as_slice()
                        .iter()
                        .find(|entry| entry.path == *path)
                })
                .cloned();
            if let Some(entry) = entry {
                if entry.directory {
                    self.content_action(Action::BrowseDirectory(entry.path), window, cx);
                } else if !self.content_ui.browser_folder_picker
                    && self.cloud_can_stage(browser)
                    && let Some(context) = self
                        .snapshot
                        .selected
                        .filter(|context| context.account == browser.account)
                {
                    self.submit_content(ContentAction::StageCloudFile {
                        context,
                        path: entry.path,
                    });
                }
                return true;
            }
            return false;
        }
        let entries = browser.entries.as_slice();
        if entries.is_empty() {
            return false;
        }
        let current = self
            .content_ui
            .browser_cursor
            .as_ref()
            .and_then(|path| entries.iter().position(|entry| entry.path == *path));
        let next =
            match key {
                "down" => Some(current.map_or(0, |index| (index + 1).min(entries.len() - 1))),
                "up" => Some(current.map_or(entries.len() - 1, |index| index.saturating_sub(1))),
                "home" => Some(0),
                "end" => Some(entries.len() - 1),
                _ if key.chars().count() == 1 && !modifiers.shift => {
                    let now = std::time::Instant::now();
                    if self.content_ui.browser_typeahead_at.is_none_or(|at| {
                        now.duration_since(at) > std::time::Duration::from_millis(750)
                    }) {
                        self.content_ui.browser_typeahead.clear();
                    }
                    if self.content_ui.browser_typeahead.len() + key.len() <= 64 {
                        self.content_ui.browser_typeahead.push_str(key);
                    }
                    self.content_ui.browser_typeahead_at = Some(now);
                    let prefix = self.content_ui.browser_typeahead.to_lowercase();
                    entries
                        .iter()
                        .position(|entry| entry.name.as_str().to_lowercase().starts_with(&prefix))
                }
                _ => return false,
            };
        if let Some(index) = next {
            self.content_ui.browser_page = index / PAGE_SIZE;
            self.content_ui.browser_cursor = Some(entries[index].path.clone());
            self.keyboard_navigation = true;
            cx.notify();
        }
        true
    }
    pub(super) fn clear_cloud_search(&mut self, cx: &mut Context<Self>) {
        self.content_ui.browser_search_since = None;
        self.content_ui.browser_search_value.clear();
        self.content_ui
            .browser_search
            .update(cx, |input, _| input.reset());
    }
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
        use talk_core::ui::content::FileSection;
        body = body.child(
            self.input_surface(&self.content_ui.browser_search, cx)
                .child(self.content_ui.browser_search.clone()),
        );
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(self.chip(
                    "cloud-all",
                    "Alle Dateien",
                    (browser.section == FileSection::All, 0, 3),
                    LocalAction::Content(Action::BrowseDirectory(
                        Text::new("/").expect("constant"),
                    )),
                    cx,
                ))
                .child(self.chip(
                    "cloud-recent",
                    "Zuletzt",
                    (browser.section == FileSection::Recent, 1, 3),
                    LocalAction::Content(Action::BrowserRecent),
                    cx,
                ))
                .child(self.chip(
                    "cloud-favorites",
                    "Favoriten",
                    (browser.section == FileSection::Favorites, 2, 3),
                    LocalAction::Content(Action::BrowserFavorites),
                    cx,
                )),
        );
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
        if browser.section == FileSection::All {
            body = body.child(crumbs);
        }
        if browser.offline {
            body = body.child(self.hint(self.l("Offline · gespeicherte Dateien")));
        } else if browser.loading {
            body = body.child(self.hint(self.l("Dateien werden geladen …")));
        }
        if self.content_ui.browser_folder_picker && browser.section == FileSection::All {
            body = body.child(self.button(
                "cloud-choose-folder",
                "Diesen Ordner auswählen",
                ButtonStyle::Primary,
                !browser.loading && browser.error.is_none() && !browser.offline,
                LocalAction::Content(Action::ChooseFolder),
                cx,
            ));
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
                        LocalAction::Content(Action::BrowserRetry),
                        cx,
                    )),
            );
        }
        if let Some(download) = &self.snapshot.content_panel.browser_download {
            let active = matches!(
                download.state,
                talk_core::ui::content::DownloadState::Transferring
                    | talk_core::ui::content::DownloadState::Committing
            );
            let label = match download.state {
                talk_core::ui::content::DownloadState::Transferring
                | talk_core::ui::content::DownloadState::Committing => {
                    "Datei wird heruntergeladen …"
                }
                talk_core::ui::content::DownloadState::Saved => "Datei gespeichert.",
                talk_core::ui::content::DownloadState::Cancelled => "Download abgebrochen.",
                talk_core::ui::content::DownloadState::Failed(_) => {
                    "Download fehlgeschlagen. Erneut versuchen."
                }
            };
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().truncate().child(format!(
                        "{} · {}",
                        download.name.as_str(),
                        self.l(label)
                    )))
                    .child(self.button(
                        "cloud-download-dismiss",
                        if active { "Abbrechen" } else { "Schließen" },
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Content(Action::Command(if active {
                            ContentAction::CancelBrowserDownload
                        } else {
                            ContentAction::DismissBrowserDownload
                        })),
                        cx,
                    )),
            );
        }
        if let Some(parent) = parent.filter(|_| browser.section == FileSection::All) {
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
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.hint(self.l("Sortieren nach")))
                .child(self.chip(
                    "cloud-sort-name",
                    "Name",
                    (browser.sort == talk_core::ui::content::FileSort::Name, 0, 3),
                    LocalAction::Content(Action::BrowserSort(
                        talk_core::ui::content::FileSort::Name,
                    )),
                    cx,
                ))
                .child(self.chip(
                    "cloud-sort-modified",
                    "Änderungsdatum",
                    (
                        browser.sort == talk_core::ui::content::FileSort::Modified,
                        1,
                        3,
                    ),
                    LocalAction::Content(Action::BrowserSort(
                        talk_core::ui::content::FileSort::Modified,
                    )),
                    cx,
                ))
                .child(self.chip(
                    "cloud-sort-size",
                    "Größe",
                    (browser.sort == talk_core::ui::content::FileSort::Size, 2, 3),
                    LocalAction::Content(Action::BrowserSort(
                        talk_core::ui::content::FileSort::Size,
                    )),
                    cx,
                )),
        );
        if entries.is_empty() && !browser.loading && browser.error.is_none() {
            body = body.child(self.hint(self.l("Dieser Ordner ist leer.")));
        }
        let start = self
            .content_ui
            .browser_page
            .saturating_mul(PAGE_SIZE)
            .min(entries.len());
        for entry in entries.iter().skip(start).take(PAGE_SIZE) {
            let thumbnail = entry
                .file_id
                .as_ref()
                .and_then(|id| id.as_str().parse::<u64>().ok())
                .and_then(|file| {
                    self.asset_image(&AssetTarget::CloudFile {
                        account: browser.account,
                        path: entry.path.clone(),
                        file,
                    })
                });
            let mut row = div()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(self.theme.border)
                .when(
                    self.content_ui.browser_cursor.as_ref() == Some(&entry.path),
                    |row| row.bg(self.theme.selected),
                )
                .child(if let Some(image) = thumbnail {
                    div()
                        .size(px(32.))
                        .rounded(px(4.))
                        .overflow_hidden()
                        .child(img(image).size_full().object_fit(ObjectFit::Cover))
                } else {
                    div()
                        .size(px(32.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.scaled_icon(
                            if entry.directory { "inbox" } else { "file" },
                            18.,
                            self.theme.muted,
                        ))
                });
            if entry.directory {
                {
                    let path = entry.path.clone();
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
                row =
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(entry.name.as_str().to_owned()))
                            .when(browser.section != FileSection::All, |column| {
                                column.child(
                                    div()
                                        .text_size(self.sp(11.))
                                        .text_color(self.theme.muted)
                                        .truncate()
                                        .child(parent_path(entry.path.as_str()).map_or_else(
                                            || "/".to_owned(),
                                            |p| p.as_str().to_owned(),
                                        )),
                                )
                            }),
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
            if !self.content_ui.browser_folder_picker
                && !entry.directory
                && let Some(context) = self
                    .snapshot
                    .selected
                    .filter(|context| context.account == browser.account)
            {
                let id: SharedString = format!("cloud-add-{}", entry.path.as_str()).into();
                row = row.child(self.button(
                    id,
                    "Zur Auswahl hinzufügen",
                    ButtonStyle::Quiet,
                    self.cloud_can_stage(browser),
                    LocalAction::Content(Action::Command(ContentAction::StageCloudFile {
                        context,
                        path: entry.path.clone(),
                    })),
                    cx,
                ));
            }
            if entry
                .file_id
                .as_ref()
                .and_then(|id| id.as_str().parse::<u64>().ok())
                .is_some_and(|id| id > 0)
            {
                let id: SharedString = format!("cloud-open-{}", entry.path.as_str()).into();
                row = row.child(self.icon_button(
                    id,
                    ("external", "In NextCloud öffnen"),
                    ButtonStyle::Quiet,
                    Availability::when(
                        !browser.offline,
                        "Die Verbindung ist derzeit nicht bereit.",
                    ),
                    LocalAction::Content(Action::Command(ContentAction::OpenBrowserFile(
                        entry.path.clone(),
                    ))),
                    cx,
                ));
            }
            if !entry.directory && self.snapshot.selected.is_some() {
                let id: SharedString = format!("cloud-download-{}", entry.path.as_str()).into();
                let available = !browser.offline
                    && self
                        .snapshot
                        .content_panel
                        .browser_download
                        .as_ref()
                        .is_none_or(|download| {
                            !matches!(
                                download.state,
                                talk_core::ui::content::DownloadState::Transferring
                                    | talk_core::ui::content::DownloadState::Committing
                            )
                        });
                row = row.child(self.icon_button(
                    id,
                    ("download", "Datei herunterladen"),
                    ButtonStyle::Quiet,
                    Availability::when(available, "Die Verbindung ist derzeit nicht bereit."),
                    LocalAction::Content(Action::Command(ContentAction::DownloadBrowserFile(
                        entry.path.clone(),
                    ))),
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
