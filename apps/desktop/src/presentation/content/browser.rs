//! Native account files surface. The account owner supplies bounded metadata.
use super::*;
use gpui::{FontWeight, KeyDownEvent, SharedString};

pub(super) const PAGE_SIZE: usize = 40;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Location {
    Folder(Text<16384>),
    Recent,
    Favorites,
    Search(Text<256>),
}

fn parent_path(path: &str) -> Option<Text<16384>> {
    if path == "/" {
        return None;
    }
    let end = path.rfind('/')?;
    Text::new(if end == 0 { "/" } else { &path[..end] }).ok()
}
fn file_icon(name: &str, mime: Option<&str>, directory: bool) -> &'static str {
    if directory {
        return "folder";
    }
    let extension = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    let has_extension = |options: &[&str]| {
        options
            .iter()
            .any(|value| extension.eq_ignore_ascii_case(value))
    };
    if mime.is_some_and(|mime| mime.starts_with("image/"))
        || has_extension(&["png", "jpg", "jpeg", "gif", "webp", "svg"])
    {
        "file-image"
    } else if mime.is_some_and(|mime| mime.starts_with("video/"))
        || has_extension(&["mp4", "webm", "mov", "mkv"])
    {
        "file-video"
    } else if mime.is_some_and(|mime| mime.starts_with("audio/"))
        || has_extension(&["mp3", "ogg", "wav", "m4a"])
    {
        "file-audio"
    } else if mime.is_some_and(|mime| {
        matches!(
            mime,
            "application/zip" | "application/x-tar" | "application/gzip"
        )
    }) || has_extension(&["zip", "tar", "gz", "7z"])
    {
        "file-archive"
    } else {
        "file"
    }
}
impl Desktop {
    pub(super) fn remember_cloud_location(&mut self) {
        use talk_core::ui::content::FileSection;
        let Some(browser) = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| Some(browser.account) == self.snapshot.active_account)
        else {
            return;
        };
        let location = match browser.section {
            FileSection::All => Location::Folder(browser.path.clone()),
            FileSection::Recent => Location::Recent,
            FileSection::Favorites => Location::Favorites,
            FileSection::Search => {
                let Ok(term) = Text::new(self.content_ui.browser_search_value.trim()) else {
                    return;
                };
                if term.as_str().is_empty() {
                    return;
                }
                Location::Search(term)
            }
        };
        if self.content_ui.browser_history.last() == Some(&location) {
            return;
        }
        if self.content_ui.browser_history.len() == 12 {
            self.content_ui.browser_history.remove(0);
        }
        self.content_ui.browser_history.push(location);
    }
    fn select_cloud_file(
        &mut self,
        path: Text<16384>,
        shift: bool,
        toggle: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(browser) = self.snapshot.content_panel.file_browser.as_ref() else {
            return;
        };
        if !self.cloud_can_stage(browser) {
            return;
        }
        let entries = browser.entries.as_slice();
        let Some(index) = entries
            .iter()
            .position(|entry| entry.path == path && !entry.directory)
        else {
            return;
        };
        let mut selection = if shift && toggle {
            self.content_ui.browser_selection.clone()
        } else {
            Vec::new()
        };
        if shift {
            let anchor = self
                .content_ui
                .browser_anchor
                .as_ref()
                .and_then(|anchor| entries.iter().position(|entry| entry.path == *anchor))
                .unwrap_or(index);
            let (start, end) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            for entry in &entries[start..=end] {
                if !entry.directory && !selection.contains(&entry.path) {
                    selection.push(entry.path.clone());
                }
            }
        } else if toggle {
            selection = self.content_ui.browser_selection.clone();
            if let Some(position) = selection.iter().position(|selected| *selected == path) {
                selection.remove(position);
            } else {
                selection.push(path.clone());
            }
            self.content_ui.browser_anchor = Some(path.clone());
        } else {
            selection.push(path.clone());
            self.content_ui.browser_anchor = Some(path.clone());
        }
        if selection.len() > 8 {
            if let Some(context) = self.snapshot.selected {
                self.feedback = Some(
                    crate::presentation::feedback::Feedback::error(
                        "Du kannst höchstens 8 Dateien auf einmal anhängen.",
                    )
                    .in_conversation(context),
                );
            }
        } else {
            self.content_ui.browser_selection = selection;
            self.content_ui.browser_cursor = Some(path);
        }
        cx.notify();
    }
    pub(super) fn cloud_can_stage(
        &self,
        browser: &talk_core::ui::content::FileBrowserView,
    ) -> bool {
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
        if key == "space"
            && (modifiers.control || modifiers.platform)
            && !modifiers.alt
            && let Some(path) = self.content_ui.browser_cursor.clone()
        {
            self.select_cloud_file(path, modifiers.shift, true, cx);
            return true;
        }
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
                    if self.content_ui.browser_selection.is_empty() {
                        self.content_action(
                            Action::Command(ContentAction::StageCloudFile {
                                context,
                                path: entry.path,
                            }),
                            window,
                            cx,
                        );
                    } else {
                        self.content_action(Action::BrowserStageSelection, window, cx);
                    }
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
                _ if key.chars().count() == 1 => {
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
            let path = entries[index].path.clone();
            self.content_ui.browser_cursor = Some(path.clone());
            if modifiers.shift {
                self.select_cloud_file(path, true, false, cx);
            }
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
        if browser.search_available {
            body = body.child(
                self.input_surface(&self.content_ui.browser_search, cx)
                    .child(self.scaled_icon("search", 16., self.theme.muted))
                    .child(self.content_ui.browser_search.clone())
                    .when(!self.content_ui.browser_search.read(cx).content.is_empty(), |d| d.child(self.icon_button(
                        "cloud-clear-search", ("x", "Suche löschen"), ButtonStyle::Quiet,
                        Availability::available(), LocalAction::Content(Action::BrowserClearSearch), cx,
                    ))),
            );
        }
        body = body.child(
            self.segmented()
                .when(!self.content_ui.browser_history.is_empty(), |row| {
                    row.child(self.icon_button(
                        "cloud-history-back",
                        ("arrow-left", "Zurück"),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::Content(Action::BrowserBack),
                        cx,
                    ))
                })
                .child(self.chip(
                    "cloud-all",
                    "Alle Dateien",
                    (browser.section == FileSection::All, 0, 3),
                    LocalAction::Content(Action::BrowseDirectory(
                        Text::new("/").expect("constant"),
                    )),
                    cx,
                ))
                .when(browser.search_available, |row| {
                    row.child(self.chip(
                        "cloud-recent",
                        "Zuletzt",
                        (browser.section == FileSection::Recent, 1, 3),
                        LocalAction::Content(Action::BrowserRecent),
                        cx,
                    ))
                })
                .when(browser.favorites_available, |row| {
                    row.child(self.chip(
                        "cloud-favorites",
                        "Favoriten",
                        (browser.section == FileSection::Favorites, 2, 3),
                        LocalAction::Content(Action::BrowserFavorites),
                        cx,
                    ))
                }),
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
        if browser.section == FileSection::All && path != "/" {
            body = body.child(crumbs);
        }
        if browser.offline {
            body = body.child(self.banner(crate::presentation::controls::BannerRole::Warning, self.l("Offline · gespeicherte Dateien")));
        } else if browser.loading {
            body = body.child(self.banner(crate::presentation::controls::BannerRole::Info, self.l("Dateien werden geladen …")));
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
        if browser.error == Some(talk_core::ui::UnavailableReason::Unsupported) {
            body = body.child(self.hint(self.l("Diese Dateioption ist derzeit nicht verfügbar.")));
            if browser.section != FileSection::All {
                body = body.child(self.button(
                    "cloud-unsupported-root",
                    "Alle Dateien",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Content(Action::BrowseDirectory(
                        Text::new("/").expect("constant"),
                    )),
                    cx,
                ));
            }
        } else if browser.error.is_some() {
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
        let sort_label = match browser.sort {
            talk_core::ui::content::FileSort::Name => "Name",
            talk_core::ui::content::FileSort::Modified => "Änderungsdatum",
            talk_core::ui::content::FileSort::Size => "Größe",
        };
        body = body.child(div().flex().flex_col().items_start()
            .child(self.button_with_leading_icon("cloud-sort-toggle", (sort_label, "chevron-down"), ButtonStyle::Secondary,
                true, LocalAction::Content(Action::ToggleBrowserSort), cx))
            .when(self.content_ui.browser_sort_open, |d| d.child(self.segmented().children([
                (talk_core::ui::content::FileSort::Name, "Name"),
                (talk_core::ui::content::FileSort::Modified, "Änderungsdatum"),
                (talk_core::ui::content::FileSort::Size, "Größe"),
            ].into_iter().enumerate().map(|(i, (sort, label))| self.chip(("cloud-sort", i), label,
                (browser.sort == sort, i, 3), LocalAction::Content(Action::BrowserSort(sort)), cx))))));
        if browser
            .pagination
            .is_some_and(|(_, total)| total > PAGE_SIZE as u64)
        {
            body = body.child(self.hint(self.l("Sortierung gilt für diese Seite.")));
        }
        if entries.is_empty() && !browser.loading && browser.error.is_none() {
            let label = if browser.offline && !browser.has_listing {
                "Für diesen Bereich sind offline keine Dateien gespeichert."
            } else {
                "Dieser Ordner ist leer."
            };
            body = body.child(self.hint(self.l(label)));
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
                        generation: browser.generation,
                    })
                });
            let mut row = div()
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .h(px(48.))
                .flex_none()
                .group(SharedString::from(format!("cloud-row-{}", entry.path.as_str())))
                .border_b_1()
                .border_color(self.theme.border)
                .when(
                    self.content_ui.browser_cursor.as_ref() == Some(&entry.path)
                        || self.content_ui.browser_selection.contains(&entry.path),
                    |row| row.bg(self.theme.selected),
                )
                .child(if !entry.directory && !self.content_ui.browser_folder_picker {
                    let path = entry.path.clone();
                    div().id(SharedString::from(format!("cloud-checkbox-{}", path.as_str())))
                        .child(self.choice_marker(self.content_ui.browser_selection.contains(&entry.path), false))
                        .when(self.cloud_can_stage(browser), |d| d.on_mouse_down(gpui::MouseButton::Left,
                            cx.listener(move |this, _: &gpui::MouseDownEvent, window, cx| {
                                this.modal_focus.focus(window);
                                this.select_cloud_file(path.clone(), false, true, cx);
                                cx.stop_propagation();
                            }))).into_any_element()
                } else { div().size(px(20.)).flex_none().into_any_element() })
                .child(if let Some(image) = thumbnail {
                    div()
                        .size(px(32.))
                        .rounded(px(4.))
                        .overflow_hidden()
                        .child(img(image).size_full().object_fit(ObjectFit::Cover))
                } else {
                    div()
                        .size(px(32.))
                        .rounded(px(6.))
                        .bg(self.theme.subtle)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.scaled_icon(
                            file_icon(
                                entry.name.as_str(),
                                entry.mime.as_ref().map(Text::as_str),
                                entry.directory,
                            ),
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
                    );
            }
            row = row.child(
                div()
                    .text_size(self.sp(12.))
                    .text_color(self.theme.muted)
                    .child(crate::presentation::localization::file_size(self.locale, entry.size)),
            );
            if let Some(modified) = entry.modified.as_ref().and_then(|value| crate::presentation::localization::file_date(self.locale, value.as_str())) {
                row = row.child(
                    div()
                        .text_size(self.sp(12.))
                        .text_color(self.theme.muted)
                        .child(modified),
                );
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
                ).opacity(0.).group_hover(SharedString::from(format!("cloud-row-{}", entry.path.as_str())), |style| style.opacity(1.)).focus(|style| style.opacity(1.)));
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
                ).opacity(0.).group_hover(SharedString::from(format!("cloud-row-{}", entry.path.as_str())), |style| style.opacity(1.)).focus(|style| style.opacity(1.)));
            }
            if !entry.directory
                && !self.content_ui.browser_folder_picker
                && self.cloud_can_stage(browser)
            {
                let path = entry.path.clone();
                let id: SharedString = format!("cloud-select-{}", path.as_str()).into();
                body = body.child(
                    div()
                        .id(id)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                                // Leave the search editor so subsequent arrows and
                                // type-ahead operate on the selected file list.
                                this.modal_focus.focus(window);
                                this.select_cloud_file(
                                    path.clone(),
                                    event.modifiers.shift,
                                    event.modifiers.control || event.modifiers.platform,
                                    cx,
                                );
                            }),
                        )
                        .child(row),
                );
            } else {
                body = body.child(row);
            }
        }
        if let Some((offset, total)) = browser.pagination {
            if total > PAGE_SIZE as u64 {
                let start = if offset == 0 { 1 } else { offset };
                let end = start.saturating_add(entries.len() as u64).saturating_sub(1);
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
                            offset > 0 && !browser.loading,
                            LocalAction::Content(Action::BrowserPage(-1)),
                            cx,
                        ))
                        .child(div().text_color(self.theme.muted).child(format!(
                            "{}–{} / {}",
                            start,
                            end,
                            total.saturating_sub(1)
                        )))
                        .child(self.button(
                            "cloud-next",
                            "Weiter",
                            ButtonStyle::Quiet,
                            offset.saturating_add(PAGE_SIZE as u64) < total && !browser.loading,
                            LocalAction::Content(Action::BrowserPage(1)),
                            cx,
                        )),
                );
            }
        } else if entries.len() > PAGE_SIZE {
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
    pub(in crate::presentation) fn cloud_files_footer(&self, cx: &Context<Self>) -> Div {
        let count = self.content_ui.browser_selection.len();
        let can_stage = self.snapshot.content_panel.file_browser.as_ref().is_some_and(|browser| self.cloud_can_stage(browser));
        div().flex_none().flex().items_center().justify_end().gap_2().px_4().py_3()
            .border_t_1().border_color(self.theme.border)
            .when(count > 0, |d| d.child(self.hint(&self.lf("{n} ausgewählt", &[("n", &count.to_string())]))))
            .child(self.button("cloud-cancel", "Abbrechen", ButtonStyle::Quiet, true, LocalAction::Close, cx))
            .when(!self.content_ui.browser_folder_picker, |d| d.child(self.button(
                "cloud-stage-selection", format!("{} ({count})", self.l("Auswahl hinzufügen")), ButtonStyle::Primary,
                Availability::when(count > 0 && can_stage, if !can_stage { "Die Verbindung ist derzeit nicht bereit." } else { "Wähle mindestens eine Datei aus." }),
                LocalAction::Content(Action::BrowserStageSelection), cx,
            )))
    }

}

#[cfg(test)]
mod tests {
    use super::file_icon;
    #[test]
    fn file_icons_prefer_directory_and_recognize_mime_or_extension() {
        assert_eq!(file_icon("pictures", Some("image/png"), true), "folder");
        assert_eq!(file_icon("photo.PNG", None, false), "file-image");
        assert_eq!(
            file_icon("recording", Some("audio/ogg"), false),
            "file-audio"
        );
        assert_eq!(file_icon("movie.webm", None, false), "file-video");
        assert_eq!(file_icon("bundle.zip", None, false), "file-archive");
        assert_eq!(file_icon("notes.txt", None, false), "file");
    }
}
