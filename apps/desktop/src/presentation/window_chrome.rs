//! Application chrome with native macOS traffic lights and OS window actions.
use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{
    AnyElement, Context, CursorStyle, Decorations, Div, MouseButton, ResizeEdge, Stateful, Window,
    WindowControlArea, div, prelude::*, px,
};

pub(super) const HEIGHT: f32 = 40.;

#[derive(Clone, Copy)]
pub enum Action {
    Minimize,
    Zoom,
    Close,
}
impl Action {
    pub(super) fn perform(self, window: &mut Window) {
        match self {
            Self::Minimize => window.minimize_window(),
            Self::Zoom => window.zoom_window(),
            // Same window-closed/service-drain path as the OS close button.
            Self::Close => window.remove_window(),
        }
    }
}

impl Desktop {
    pub(super) fn window_chrome(
        &self,
        window: &Window,
        shell_visible: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let navigation = shell_visible
            && self.overlay.is_none()
            && !self.account_removal.confirming()
            && !self.incoming_visible();
        let traffic_lights = cfg!(target_os = "macos") && !window.is_fullscreen();
        let custom_buttons = cfg!(target_os = "windows")
            || (cfg!(target_os = "linux")
                && matches!(window.window_decorations(), Decorations::Client { .. }));
        let drag = div()
            .id("window-drag-region")
            .window_control_area(WindowControlArea::Drag)
            .h_full()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .px_4()
            .when(traffic_lights && !navigation, |d| d.pl(px(88.)))
            .text_size(px(12.))
            .text_color(self.theme.muted)
            .child(if self.development {
                "Talk · UI development"
            } else {
                "Talk"
            })
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else if cfg!(target_os = "linux") {
                        window.zoom_window();
                    }
                }
            })
            .when(cfg!(target_os = "linux"), |d| {
                d.on_mouse_move(|event, window, _| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        window.start_window_move();
                    }
                })
                .on_mouse_down(MouseButton::Right, |event, window, _| {
                    window.show_window_menu(event.position);
                })
            });
        div()
            .id("application-titlebar")
            .h(px(HEIGHT))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .when(navigation, |d| {
                d.child(
                    div()
                        .flex_none()
                        .pl(px(if traffic_lights { 88. } else { 8. }))
                        .child(
                            self.icon_button(
                                "toggle-navigation",
                                (
                                    "sidebar",
                                    if self.sidebar_visible(window) {
                                        "Seitenleiste ausblenden"
                                    } else {
                                        "Seitenleiste einblenden"
                                    },
                                ),
                                ButtonStyle::Quiet,
                                Availability::available().below_control(),
                                LocalAction::ToggleNavigation,
                                cx,
                            )
                            .size(px(30.))
                            .min_h(px(30.)),
                        ),
                )
            })
            .child(drag)
            .when(navigation && self.narrow_navigation(window), |d| {
                d.child(
                    self.icon_button(
                        "small-settings",
                        ("settings", "Einstellungen"),
                        ButtonStyle::Quiet,
                        Availability::available().below_control(),
                        LocalAction::Open(Overlay::Settings),
                        cx,
                    )
                    .size(px(30.))
                    .min_h(px(30.))
                    .mr_3(),
                )
            })
            .when(self.can_install_update() && !self.projection_blocked, |d| {
                d.child(
                    self.button_with_icon(
                        "install-update",
                        ("Update", "download"),
                        ButtonStyle::Primary,
                        Availability::when(
                            self.overlay.is_none() && !self.account_removal.confirming(),
                            "Schließe den Dialog vor dem Neustart.",
                        ),
                        LocalAction::InstallUpdate(self.updates.generation),
                        cx,
                    )
                    .min_h(px(28.))
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .py(px(4.))
                    .mr_3(),
                )
            })
            .when(custom_buttons && !window.is_fullscreen(), |d| {
                let supported = window.window_controls();
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pr_2()
                        .when(supported.minimize, |d| {
                            d.child(self.window_button(
                                "window-minimize",
                                ("minus", "Fenster minimieren"),
                                Action::Minimize,
                                WindowControlArea::Min,
                                cx,
                            ))
                        })
                        .when(supported.maximize, |d| {
                            d.child(self.window_button(
                                "window-zoom",
                                ("square", "Fenstergröße umschalten"),
                                Action::Zoom,
                                WindowControlArea::Max,
                                cx,
                            ))
                        })
                        .child(self.window_button(
                            "window-close",
                            ("x", "Fenster schließen"),
                            Action::Close,
                            WindowControlArea::Close,
                            cx,
                        )),
                )
            })
    }

    fn window_button(
        &self,
        id: &'static str,
        symbol: (&'static str, &'static str),
        action: Action,
        area: WindowControlArea,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.icon_button(
            id,
            symbol,
            ButtonStyle::Quiet,
            Availability::available().below_control(),
            LocalAction::Window(action),
            cx,
        )
        .window_control_area(area)
        .size(px(30.))
        .min_h(px(30.))
    }

    pub(super) fn window_resize_edges(&self, window: &Window) -> Vec<AnyElement> {
        if !cfg!(target_os = "linux")
            || window.is_fullscreen()
            || window.is_maximized()
            || !matches!(window.window_decorations(), Decorations::Client { tiling } if !tiling.is_tiled())
        {
            return Vec::new();
        }
        // Thin edge hitboxes leave editor drags and scrollbars untouched.
        [
            ResizeEdge::Top,
            ResizeEdge::Bottom,
            ResizeEdge::Left,
            ResizeEdge::Right,
            ResizeEdge::TopLeft,
            ResizeEdge::TopRight,
            ResizeEdge::BottomLeft,
            ResizeEdge::BottomRight,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, edge)| {
            let (element, cursor) = match edge {
                ResizeEdge::Top => (
                    div().top_0().left(px(8.)).right(px(8.)).h(px(4.)),
                    CursorStyle::ResizeUpDown,
                ),
                ResizeEdge::Bottom => (
                    div().bottom_0().left(px(8.)).right(px(8.)).h(px(4.)),
                    CursorStyle::ResizeUpDown,
                ),
                ResizeEdge::Left => (
                    div().left_0().top(px(8.)).bottom(px(8.)).w(px(4.)),
                    CursorStyle::ResizeLeftRight,
                ),
                ResizeEdge::Right => (
                    div().right_0().top(px(8.)).bottom(px(8.)).w(px(4.)),
                    CursorStyle::ResizeLeftRight,
                ),
                ResizeEdge::TopLeft => (
                    div().top_0().left_0().size(px(8.)),
                    CursorStyle::ResizeUpLeftDownRight,
                ),
                ResizeEdge::TopRight => (
                    div().top_0().right_0().size(px(8.)),
                    CursorStyle::ResizeUpRightDownLeft,
                ),
                ResizeEdge::BottomLeft => (
                    div().bottom_0().left_0().size(px(8.)),
                    CursorStyle::ResizeUpRightDownLeft,
                ),
                ResizeEdge::BottomRight => (
                    div().bottom_0().right_0().size(px(8.)),
                    CursorStyle::ResizeUpLeftDownRight,
                ),
            };
            element
                .id(("window-edge", index))
                .absolute()
                .cursor(cursor)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.start_window_resize(edge);
                    cx.stop_propagation();
                })
                .into_any_element()
        })
        .collect()
    }
}
