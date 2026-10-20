//! Private native control vocabulary; all icons are fixed embedded SVG assets.
use super::{ButtonStyle, Desktop, LocalAction};
use gpui::{
    Context, Div, ElementId, FocusHandle, FontWeight, Hsla, KeyDownEvent, SharedString, Stateful,
    div, prelude::*, px, svg,
};
use std::time::{Duration, Instant};

// One window-owned hover target. The epoch invalidates callbacks from retired
// element trees; it is never reused, even at exhaustion. No executor/task/cache.
pub(super) struct HoverHints {
    epoch: Option<u64>,
    target: Option<(ElementId, Option<Instant>)>,
    frame_requests: u32,
}
impl Default for HoverHints {
    fn default() -> Self {
        Self {
            epoch: Some(0),
            target: None,
            frame_requests: 0,
        }
    }
}
impl HoverHints {
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }
    pub fn enter(&mut self, epoch: Option<u64>, id: ElementId, now: Instant) {
        if epoch.is_some() && epoch == self.epoch {
            self.target = Some((id, Some(now)));
        }
    }
    pub fn exit(&mut self, epoch: Option<u64>, id: &ElementId) {
        if epoch == self.epoch
            && self
                .target
                .as_ref()
                .is_some_and(|(current, _)| current == id)
        {
            self.target = None;
        }
    }
    pub fn cancel(&mut self) {
        self.target = None;
        self.epoch = self.epoch.and_then(|n| n.checked_add(1));
    }
    pub fn retire(&mut self, live: impl FnOnce(&ElementId) -> bool) -> bool {
        if self.target.as_ref().is_some_and(|(id, _)| !live(id)) {
            self.cancel();
            true
        } else {
            false
        }
    }
    pub fn displayed(&self, id: &ElementId) -> bool {
        self.displayed_owner() == Some(id)
    }
    fn displayed_owner(&self) -> Option<&ElementId> {
        self.target
            .as_ref()
            .and_then(|(current, start)| start.is_none().then_some(current))
    }
    pub fn request_frame(&mut self, now: Instant) -> bool {
        let Some((_, start)) = self.target.as_mut() else {
            return false;
        };
        let Some(at) = *start else {
            return false;
        };
        if now.duration_since(at) >= Duration::from_millis(500) {
            *start = None;
            false
        } else {
            self.frame_requests = self.frame_requests.saturating_add(1);
            true
        }
    }
    pub fn diagnostic(&self) -> (bool, bool, u32) {
        (
            self.target
                .as_ref()
                .is_some_and(|(_, start)| start.is_some()),
            self.target
                .as_ref()
                .is_some_and(|(_, start)| start.is_none()),
            self.frame_requests,
        )
    }
}

// A matured pointer hover owns the one visible control hint. While it is
// pending, or after exit/cancellation, the actual keyboard focus may explain
// itself again. This affects hints only; it never transfers keyboard focus.
fn hint_visible(hover_owner: Option<&ElementId>, id: &ElementId, focused: bool) -> bool {
    hover_owner.map_or(focused, |owner| owner == id)
}

pub fn icon(name: &'static str, size: f32, color: Hsla) -> gpui::Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(color)
}
// The private finite focus owner is independent of list position.
pub(super) use super::focus::{FocusStates, scoped_id};

/// A reason is static UI copy, never a server string. An available action has
/// no unavailability explanation even if a caller supplied one.
#[derive(Clone, Copy)]
pub(super) struct Availability {
    enabled: bool,
    reason: Option<&'static str>,
    above_composer: bool,
    below_control: bool,
    beside_control: bool,
    explained_elsewhere: bool,
    visible_label: bool,
    hover_hint: bool,
}
impl Availability {
    pub fn available() -> Self {
        Self::when(true, "")
    }
    pub fn when(enabled: bool, reason: &'static str) -> Self {
        Self {
            enabled,
            reason: (!enabled).then_some(reason),
            above_composer: false,
            below_control: false,
            beside_control: false,
            explained_elsewhere: false,
            visible_label: false,
            hover_hint: false,
        }
    }
    pub fn labelled(mut self) -> Self {
        self.visible_label = true;
        self
    }
    pub fn beside_control(mut self) -> Self {
        self.beside_control = true;
        self
    }
    pub fn explained_elsewhere(mut self) -> Self {
        self.explained_elsewhere = true;
        self
    }
    pub fn above_composer(mut self) -> Self {
        self.above_composer = true;
        self.below_control = false;
        self
    }
    pub fn below_control(mut self) -> Self {
        self.below_control = true;
        self.above_composer = false;
        self
    }
}
impl From<bool> for Availability {
    fn from(enabled: bool) -> Self {
        Self::when(enabled, "Derzeit nicht verfügbar.")
    }
}

// Availability-aware kit surfaces and text resolve here. Always-active custom
// navigation/thread links own explicit roles; conditional composites use the kit.
fn control_colors(t: super::Theme, style: ButtonStyle, enabled: bool) -> (Hsla, Hsla, Hsla) {
    if !enabled {
        return match style {
            ButtonStyle::Quiet => (
                gpui::transparent_black(),
                t.disabled_fg,
                gpui::transparent_black(),
            ),
            ButtonStyle::Primary | ButtonStyle::Danger => (t.subtle, t.disabled_fg, t.border),
            ButtonStyle::Secondary => (t.surface, t.disabled_fg, t.border),
            ButtonStyle::Stage | ButtonStyle::StageDanger => {
                (t.stage_panel, t.stage_disabled, t.stage_panel)
            }
        };
    }
    match style {
        ButtonStyle::Primary => (t.accent, t.on_accent, t.accent),
        ButtonStyle::Danger => (t.danger_bg, t.danger, t.danger),
        ButtonStyle::Stage => (t.stage_panel, t.stage_text, t.stage_panel),
        ButtonStyle::StageDanger => (t.danger, t.danger_bg, t.danger),
        ButtonStyle::Quiet => (
            gpui::transparent_black(),
            t.sidebar_secondary,
            gpui::transparent_black(),
        ),
        ButtonStyle::Secondary => (t.surface, t.text, t.border),
    }
}

// Secondary icons are quieter than their text, on the actual opaque surface.
fn icon_color(t: super::Theme, style: ButtonStyle, enabled: bool) -> Hsla {
    if enabled && matches!(style, ButtonStyle::Secondary) {
        t.muted
    } else {
        control_colors(t, style, enabled).1
    }
}

// Standalone input fallback. Composite fields use Desktop::field_ring so their
// whole container owns focus and scroll revelation.
pub(super) fn focus_ring(
    handle: FocusHandle,
    theme: super::Theme,
    radius: f32,
) -> impl IntoElement {
    native_focus_ring(
        Some(handle),
        theme,
        radius - 1.,
        5.,
        super::navigation::Reveal::default(),
    )
}

/// Border-only native ring. Its prepaint bounds also feed the single focused
/// control into its owning scroll viewport; no item-index geometry is cached.
fn native_focus_ring(
    handle: Option<FocusHandle>,
    theme: super::Theme,
    radius: f32,
    dilation: f32,
    reveal: super::navigation::Reveal,
) -> impl IntoElement {
    let prepaint_handle = handle.clone();
    gpui::canvas(
        move |bounds, window, _| {
            if prepaint_handle
                .as_ref()
                .is_some_and(|h| h.is_focused(window))
            {
                reveal.record(bounds.dilate(px(dilation.max(0.))));
            }
        },
        move |bounds, _, window, _| {
            if handle.as_ref().is_some_and(|h| h.is_focused(window)) {
                window.paint_quad(gpui::quad(
                    bounds.dilate(px(dilation)),
                    px(radius + dilation),
                    gpui::transparent_black(),
                    px(2.),
                    theme.focus,
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
}

#[derive(IntoElement)]
pub(super) struct ReasonTip {
    text_size: gpui::Pixels,
    line_height: gpui::Pixels,
    handle: Option<FocusHandle>,
    hover_owner: Option<ElementId>,
    keyboard_name: bool,
    id: ElementId,
    anchors: super::navigation::HintGeometry,
    below_control: bool,
    beside_control: bool,
    action_name: Option<&'static str>,
    label: SharedString,
    theme: super::Theme,
}
impl gpui::RenderOnce for ReasonTip {
    fn render(self, window: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        let visible = hint_visible(
            self.hover_owner.as_ref(),
            &self.id,
            self.handle.as_ref().is_some_and(|h| h.is_focused(window))
                && (self.keyboard_name || !self.label.is_empty()),
        );
        let limit = (px(310.) * (self.text_size / px(12.)).sqrt())
            .min(window.viewport_size().width - px(16.));
        div().absolute().inset_0().when(visible, |d| {
            d.child(gpui::deferred(MeasuredHint {
                child: div()
                    .max_w(limit)
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(self.theme.text)
                    .text_color(self.theme.surface)
                    .text_size(self.text_size)
                    .line_height(self.line_height)
                    .shadow_sm()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when_some(self.action_name, |d, name| {
                        d.child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    })
                    .when(!self.label.is_empty(), |d| {
                        d.child(div().font_weight(FontWeight::NORMAL).child(self.label))
                    })
                    .into_any_element(),
                limit,
                anchor: self.id,
                anchors: self.anchors,
                below: self.below_control,
                beside: self.beside_control,
            }))
        })
    }
}
/// One ephemeral measured child per visible reason, no retained geometry/cache.
/// The anchor is the entire control (or composer), not its top-left point.
struct MeasuredHint {
    child: gpui::AnyElement,
    limit: gpui::Pixels,
    anchor: ElementId,
    anchors: super::navigation::HintGeometry,
    below: bool,
    beside: bool,
}
impl IntoElement for MeasuredHint {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
fn hint_origin(
    host: gpui::Bounds<gpui::Pixels>,
    control: gpui::Bounds<gpui::Pixels>,
    tip: gpui::Size<gpui::Pixels>,
    viewport: gpui::Size<gpui::Pixels>,
    below: bool,
    beside: bool,
) -> gpui::Point<gpui::Pixels> {
    let margin = px(8.);
    let gap = px(12.);
    let x = (control.right() - tip.width)
        .clamp(margin, (viewport.width - tip.width - margin).max(margin));
    let y = (control.top() + (control.size.height - tip.height) / 2.)
        .clamp(margin, (viewport.height - tip.height - margin).max(margin));
    let top = gpui::point(x, host.top() - gap - tip.height);
    let bottom = gpui::point(x, host.bottom() + gap);
    let right = gpui::point(host.right() + gap, y);
    let left = gpui::point(host.left() - gap - tip.width, y);
    let choices = if beside {
        [right, left, top, bottom]
    } else if below {
        [bottom, top, right, left]
    } else {
        [top, left, right, bottom]
    };
    for origin in choices {
        if origin.x >= margin
            && origin.y >= margin
            && origin.x + tip.width <= viewport.width - margin
            && origin.y + tip.height <= viewport.height - margin
        {
            return origin;
        }
    }
    // Extremely small viewport: favor the largest unobstructed vertical side.
    // This only applies when no side can fit the entire measured text.
    if host.top() >= viewport.height - host.bottom() {
        top
    } else {
        bottom
    }
}
impl gpui::Element for MeasuredHint {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, ()) {
        let style = gpui::Style {
            position: gpui::Position::Absolute,
            inset: gpui::Edges::all(px(0.).into()),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let size = self.child.layout_as_root(
            gpui::size(
                gpui::AvailableSpace::Definite(self.limit),
                gpui::AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        let origin = hint_origin(
            bounds,
            self.anchors.get(&self.anchor).unwrap_or(bounds),
            size,
            window.viewport_size(),
            self.below,
            self.beside,
        );
        self.child.prepaint_at(origin, window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        self.child.paint(window, cx);
    }
}
impl Desktop {
    /// Inline geometry follows the same square-root text scale as avatars.
    /// Decorative miniature previews and the project mark retain their own size.
    pub(super) fn scaled_icon(&self, name: &'static str, size: f32, color: Hsla) -> gpui::Svg {
        icon(name, size * (f32::from(self.scale) / 100.).sqrt(), color)
    }

    pub fn control_focus(
        &self,
        id: impl Into<ElementId>,
        cx: &Context<Self>,
    ) -> Option<FocusHandle> {
        self.control_focus.get(id.into(), cx)
    }
    pub(super) fn field_ring(&self, handle: FocusHandle, radius: f32) -> impl IntoElement {
        self.control_ring(Some(handle), radius - 1., 5.)
    }
    /// Reveal the complete explanatory region when its child action is focused.
    /// Place last in a relative container so its bounds replace the child's ring
    /// bounds. This uses the same single-frame rectangle, not a geometry cache.
    pub(super) fn reveal_region(
        &self,
        id: impl Into<ElementId>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let focus = self.control_focus(id, cx);
        let reveal = self.navigation.reveal.clone();
        gpui::canvas(
            move |bounds, window, _| {
                if focus.as_ref().is_some_and(|f| f.is_focused(window)) {
                    reveal.record(bounds.dilate(px(4.)));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
    pub(super) fn control_ring(
        &self,
        handle: Option<FocusHandle>,
        radius: f32,
        dilation: f32,
    ) -> impl IntoElement {
        native_focus_ring(
            handle,
            self.theme,
            radius,
            dilation,
            self.navigation.reveal.clone(),
        )
    }
    pub(super) fn control(
        &self,
        id: impl Into<ElementId>,
        style: ButtonStyle,
        availability: impl Into<Availability>,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.control_with_geometry(id, style, availability, action, (1., 8.), cx)
    }
    pub(super) fn control_with_geometry(
        &self,
        id: impl Into<ElementId>,
        style: ButtonStyle,
        availability: impl Into<Availability>,
        action: LocalAction,
        geometry: (f32, f32),
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let (border_width, corner_radius) = geometry;
        let availability = availability.into();
        let id = id.into();
        let hover_id = id.clone();
        let anchor_id = id.clone();
        let anchors = self.navigation.hints.clone();
        let hovered = self.hover_hints.displayed(&id);
        let hover_epoch = self.hover_hints.epoch();
        let focus = self.control_focus(id.clone(), cx);
        let anchor_focus = focus.clone();
        let enabled = availability.enabled && focus.is_some();
        let (bg, fg, border) = control_colors(t, style, enabled);
        let room_row = matches!(&action, LocalAction::Navigate(_));
        let selected_segment = match action {
            LocalAction::Destination(destination) => self.destination == destination,
            LocalAction::Scale(n) => self.scale == n,
            LocalAction::Density(compact) => self.compact == compact,
            LocalAction::SearchKind(kind) => self.search_kind == kind,
            LocalAction::Open(overlay) => self.overlay == Some(overlay),
            _ => false,
        };
        div()
            .id(id)
            .relative()
            .when_some(focus.clone(), |d, f| {
                d.track_focus(&f.tab_stop(!self.background_inert))
            })
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .min_h(px(36.))
            .px_3()
            .py(px(6.))
            .rounded(px(corner_radius))
            .text_size(self.sp(13.))
            .line_height(self.sp(20.))
            .font_weight(FontWeight::SEMIBOLD)
            .bg(bg)
            .text_color(fg)
            .border_1()
            .border_color(border)
            // Absolute bounds begin inside the host border: external gap 2 +
            // stroke 2, radius concentric with the actual host, for every width.
            .child(
                gpui::canvas(
                    move |bounds, window, _| {
                        anchors.record(
                            anchor_id.clone(),
                            bounds,
                            anchor_focus.as_ref().is_some_and(|h| h.is_focused(window)),
                            hovered,
                        );
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(native_focus_ring(
                focus,
                if matches!(style, ButtonStyle::Stage | ButtonStyle::StageDanger) {
                    super::Theme {
                        focus: t.stage_text,
                        ..t
                    }
                } else {
                    t
                },
                corner_radius - border_width,
                border_width + 4.,
                self.navigation.reveal.clone(),
            ))
            .when(!self.background_inert && availability.hover_hint, |b| {
                b.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.control_focus.live(&hover_id) {
                        this.hover_hints
                            .enter(hover_epoch, hover_id.clone(), Instant::now());
                    } else {
                        this.hover_hints.exit(hover_epoch, &hover_id);
                    }
                    cx.notify();
                }))
            })
            .when(enabled && !self.background_inert, |b| {
                b.cursor_pointer()
                    .hover(|s| {
                        s.bg(
                            if matches!(style, ButtonStyle::Primary | ButtonStyle::StageDanger) {
                                Hsla {
                                    l: (bg.l - 0.04).max(0.),
                                    ..bg
                                }
                            } else if matches!(style, ButtonStyle::Stage) {
                                Hsla {
                                    l: (bg.l + 0.04).min(1.),
                                    ..bg
                                }
                            } else if selected_segment {
                                t.segment
                            } else if room_row {
                                t.row_hover
                            } else {
                                t.hover
                            },
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.handle(action.clone(), window, cx)
                    }))
                    .on_key_down(|event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            // GPUI invokes on_click once on native key-up. Only
                            // stop the parent form's key-down path here.
                            cx.stop_propagation();
                        }
                    })
            })
            .when(!enabled && !self.background_inert, |b| {
                b.on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                    // Hover explains; a disabled click never steals the draft caret.
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_key_down(|e: &KeyDownEvent, _, cx| {
                    // Focus exposes an explanation, never an action (including a
                    // bubbled Enter that might otherwise submit a parent form).
                    if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                    }
                })
            })
    }
    pub(super) fn reason_tip(
        &self,
        id: ElementId,
        availability: Availability,
        action_name: Option<&'static str>,
        cx: &Context<Self>,
    ) -> ReasonTip {
        ReasonTip {
            text_size: self.sp(12.),
            line_height: self.sp(16.),
            handle: self.control_focus(id.clone(), cx),
            hover_owner: self.hover_hints.displayed_owner().cloned(),
            keyboard_name: self.keyboard_navigation,
            id,
            anchors: self.navigation.hints.clone(),
            // Composer callers attach this element to the actual card top;
            // no single-line height or toolbar offset is assumed.
            below_control: availability.below_control,
            beside_control: availability.beside_control,
            action_name: action_name.map(|name| self.l(name)),
            label: self.l(availability.reason.unwrap_or("")).into(),
            theme: self.theme,
        }
    }
    pub(super) fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        style: ButtonStyle,
        availability: impl Into<Availability>,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let mut availability = availability.into();
        availability.hover_hint = !availability.enabled && !availability.explained_elsewhere;
        let label: SharedString = label.into();
        let label: SharedString = self.l(label.as_ref()).to_owned().into();
        self.control(id.clone(), style, availability, action, cx)
            .child(label)
            .when(
                !availability.enabled
                    && !availability.explained_elsewhere
                    && !self.background_inert,
                |d| d.child(self.reason_tip(id, availability, None, cx)),
            )
    }
    /// A boolean switch uses the same single native click/focus path as buttons.
    /// Unknown remote values keep the thumb centred and cannot masquerade as Off.
    pub(super) fn switch(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        value: Option<bool>,
        availability: Availability,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let mut availability = availability;
        availability.enabled &= value.is_some();
        availability.hover_hint = true;
        let enabled = availability.enabled && self.control_focus(id.clone(), cx).is_some();
        let t = self.theme;
        let (track, thumb) = match (enabled, value) {
            (true, Some(true)) => (t.accent, t.on_accent),
            (true, _) => (t.input_border, t.surface),
            (false, _) => (t.subtle, t.disabled_fg),
        };
        let height = self.sp(26.).min(px(34.));
        let width = height * 1.77;
        let thumb_size = height - px(8.);
        let offset = match value {
            Some(true) => width - thumb_size - px(4.),
            Some(false) => px(4.),
            None => (width - thumb_size) / 2.,
        };
        let mut hint = availability;
        if enabled {
            hint.reason = Some(if value == Some(true) { "An" } else { "Aus" });
        }
        self.control_with_geometry(
            id.clone(),
            if value == Some(true) {
                ButtonStyle::Primary
            } else {
                ButtonStyle::Secondary
            },
            availability,
            action,
            (1., f32::from(height) / 2.),
            cx,
        )
        .flex_none()
        .w(width)
        .h(height)
        .min_h(height)
        .p_0()
        .bg(track)
        .border_color(if enabled { track } else { t.border })
        .child(
            div()
                .absolute()
                .left(offset - px(1.))
                .top(px(3.))
                .size(thumb_size)
                .rounded_full()
                .bg(thumb)
                .flex()
                .items_center()
                .justify_center()
                .when(value == Some(true), |d| {
                    d.child(icon("check", f32::from(thumb_size) - 4., track))
                })
                .when(value.is_none(), |d| {
                    d.child(div().w(px(8.)).h(px(2.)).bg(t.subtle))
                }),
        )
        .when(!self.background_inert, |d| {
            d.child(self.reason_tip(id, hint, Some(label), cx))
        })
    }
    pub(super) fn option_row(
        &self,
        id: impl Into<ElementId>,
        copy: (&'static str, &str, &str),
        availability: impl Into<Availability>,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let availability = availability.into();
        let (_, foreground, _) =
            control_colors(self.theme, ButtonStyle::Secondary, availability.enabled);
        self.button(
            id,
            "",
            ButtonStyle::Secondary,
            availability.explained_elsewhere(),
            action,
            cx,
        )
        .w_full()
        .flex_none()
        .justify_start()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .child(self.scaled_icon(copy.0, 20., foreground))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().child(self.l(copy.1).to_owned()))
                .when(!copy.2.is_empty(), |d| {
                    d.child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(self.sp(12.))
                            .line_height(self.sp(18.))
                            .text_color(self.theme.muted)
                            .child(self.l(copy.2).to_owned()),
                    )
                })
                .when_some(availability.reason, |d, reason| {
                    d.child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(self.sp(12.))
                            .line_height(self.sp(18.))
                            .text_color(self.theme.muted)
                            .child(self.l(reason)),
                    )
                }),
        )
    }
    /// Compound buttons use the same foreground and explanation as the kit.
    pub(super) fn button_with_icon(
        &self,
        id: impl Into<ElementId>,
        content: (&'static str, &'static str),
        style: ButtonStyle,
        availability: impl Into<Availability>,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let availability = availability.into();
        let (_, foreground, _) = control_colors(self.theme, style, availability.enabled);
        self.button(id, content.0, style, availability, action, cx)
            .justify_between()
            .child(self.scaled_icon(content.1, 18., foreground))
    }
    pub(super) fn button_with_leading_icon(
        &self,
        id: impl Into<ElementId>,
        content: (&'static str, &'static str),
        style: ButtonStyle,
        availability: impl Into<Availability>,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let availability = availability.into();
        let (_, foreground, _) = control_colors(self.theme, style, availability.enabled);
        self.button(id, "", style, availability, action, cx)
            .justify_start()
            .child(self.scaled_icon(content.1, 18., foreground))
            .child(self.l(content.0))
    }
    pub(super) fn icon_button(
        &self,
        id: impl Into<ElementId>,
        symbol: (&'static str, &'static str),
        style: ButtonStyle,
        availability: Availability,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.styled_icon_button(id, symbol, (style, 34., 8.), availability, action, cx)
    }
    pub(super) fn round_icon_button(
        &self,
        id: impl Into<ElementId>,
        symbol: (&'static str, &'static str),
        style: ButtonStyle,
        availability: Availability,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.styled_icon_button(id, symbol, (style, 52., 26.), availability, action, cx)
    }
    fn styled_icon_button(
        &self,
        id: impl Into<ElementId>,
        symbol: (&'static str, &'static str),
        geometry: (ButtonStyle, f32, f32),
        availability: Availability,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let (style, extent, radius) = geometry;
        let id = id.into();
        let (name, label) = symbol;
        let t = self.theme;
        let color = icon_color(t, style, availability.enabled);
        let mut availability = availability;
        availability.hover_hint = !availability.visible_label || !availability.enabled;
        self.control_with_geometry(id.clone(), style, availability, action, (1., radius), cx)
            .size(px(extent))
            .min_h(px(extent))
            .p_0()
            .child(self.scaled_icon(name, 18., color))
            .when(
                !self.background_inert
                    && !availability.above_composer
                    && !availability.explained_elsewhere
                    && (!availability.visible_label || !availability.enabled),
                |d| {
                    // Active icons also expose their name to keyboard users. This
                    // same anchored surface handles hover: no overlapping GPUI tip.
                    d.child(self.reason_tip(
                        id,
                        availability,
                        (!availability.visible_label).then_some(label),
                        cx,
                    ))
                },
            )
    }

    pub fn chip(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        group: (bool, usize, usize),
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let (selected, index, _count) = group;
        let t = self.theme;
        let control = self
            .button(id, label, ButtonStyle::Quiet, true, action.clone(), cx)
            .min_h(px(32.))
            .py_0()
            .px_2()
            .text_size(self.sp(12.))
            .bg(if selected {
                t.segment
            } else {
                gpui::transparent_black()
            })
            .border_color(if selected {
                t.input_border
            } else {
                gpui::transparent_black()
            })
            .text_color(if selected {
                t.text
            } else {
                t.sidebar_secondary
            })
            .font_weight(if selected {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            });
        match super::navigation::Group::for_action(&action) {
            Some(group) => self.roving(control, group, index, cx),
            None => control,
        }
    }

    pub fn person_avatar(&self, initials: &str, size: f32) -> Div {
        let seed = initials
            .bytes()
            .fold(0usize, |n, c| n.wrapping_mul(31).wrapping_add(c as usize));
        let (bg, fg) = self.theme.avatar(seed);
        let extent = size * (f32::from(self.scale) / 100.).sqrt();
        div()
            .flex_none()
            .size(px(extent))
            .rounded(px(extent / 3.))
            .flex()
            .items_center()
            .justify_center()
            .bg(bg)
            .text_color(fg)
            .text_size(px(extent * 0.36))
            .line_height(px(extent * 0.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initials.to_owned())
    }
    pub fn avatar(&self, initials: &str, large: bool) -> Div {
        self.person_avatar(initials, if large { 72. } else { 36. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::theme::{Appearance, Palette, Theme};
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |h: Hsla| {
            let c = gpui::Rgba::from(h);
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn focus_and_hover_share_one_visible_hint_and_restore_after_retirement() {
        let mut hints = HoverHints::default();
        let now = Instant::now();
        let call = ElementId::from("call");
        let details = ElementId::from("details");
        let unrelated = ElementId::from("unrelated");
        let visible = |h: &HoverHints, focus: Option<&ElementId>| {
            [&call, &details, &unrelated]
                .into_iter()
                .filter(|id| hint_visible(h.displayed_owner(), id, focus == Some(id)))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(visible(&hints, Some(&call)), vec![call.clone()]);
        let epoch = hints.epoch();
        hints.enter(epoch, details.clone(), now);
        assert!(hints.request_frame(now + Duration::from_millis(499)));
        assert_eq!(visible(&hints, Some(&call)), vec![call.clone()]);
        assert!(!hints.request_frame(now + Duration::from_millis(500)));
        for focus in [None, Some(&call), Some(&details), Some(&unrelated)] {
            assert_eq!(visible(&hints, focus), vec![details.clone()]);
        }
        hints.exit(epoch, &details);
        assert_eq!(visible(&hints, Some(&call)), vec![call.clone()]);
        assert!(visible(&hints, None).is_empty());
        hints.enter(epoch, details.clone(), now);
        hints.request_frame(now + Duration::from_millis(500));
        assert!(hints.retire(|id| id != &details));
        // An old view/account callback cannot take ownership again.
        hints.enter(epoch, details.clone(), now);
        assert_eq!(visible(&hints, Some(&call)), vec![call.clone()]);
        assert!(visible(&hints, None).is_empty());
    }

    #[test]
    fn hover_deadline_has_one_owner_and_stale_callbacks_cannot_rearm_it() {
        let mut h = HoverHints::default();
        let now = Instant::now();
        let a = ElementId::from("old-row");
        let b = ElementId::from("new-row");
        let epoch = h.epoch();
        h.enter(epoch, a.clone(), now);
        assert!(h.request_frame(now + Duration::from_millis(499)));
        h.enter(epoch, b.clone(), now + Duration::from_millis(100));
        h.exit(epoch, &a);
        assert!(!h.displayed(&a));
        assert!(h.request_frame(now + Duration::from_millis(599)));
        assert!(!h.request_frame(now + Duration::from_millis(600)));
        assert!(h.displayed(&b));
        let shown = h.diagnostic();
        assert!(!h.request_frame(now + Duration::from_secs(2)));
        assert_eq!(h.diagnostic(), shown);
        h.exit(epoch, &b);
        assert!(!h.request_frame(now + Duration::from_secs(3)));
        // Input, navigation and accepted projection changes invalidate all old
        // callbacks, including a static control ID reused in another account.
        h.enter(epoch, a.clone(), now);
        h.cancel();
        h.enter(epoch, a.clone(), now);
        assert!(!h.request_frame(now));
        let next_epoch = h.epoch();
        h.enter(next_epoch, b.clone(), now);
        h.exit(epoch, &b);
        assert!(h.request_frame(now));
        assert!(h.retire(|id| id != &b));
        h.enter(next_epoch, b, now);
        assert!(!h.request_frame(now));
        h.epoch = Some(u64::MAX);
        h.cancel();
        h.enter(None, a, now);
        assert!(!h.request_frame(now));
    }

    #[test]
    fn measured_hints_keep_the_host_and_ring_clear_at_edges() {
        let viewport = gpui::size(px(1120.), px(716.));
        for (host, tip, beside) in [
            (
                gpui::Bounds::new(
                    gpui::point(px(1004.), px(34.)),
                    gpui::size(px(32.), px(32.)),
                ),
                gpui::size(px(310.), px(94.)),
                false,
            ),
            (
                gpui::Bounds::new(gpui::point(px(223.), px(72.)), gpui::size(px(32.), px(32.))),
                gpui::size(px(180.), px(40.)),
                true,
            ),
            (
                gpui::Bounds::new(
                    gpui::point(px(304.), px(500.)),
                    gpui::size(px(780.), px(160.)),
                ),
                gpui::size(px(310.), px(130.)),
                false,
            ),
        ] {
            let origin = hint_origin(host, host, tip, viewport, false, beside);
            let hint = gpui::Bounds::new(origin, tip);
            assert!(
                !hint.intersects(&host.dilate(px(6.))),
                "reason must preserve host and exterior ring"
            );
            assert!(
                origin.x >= px(8.)
                    && origin.y >= px(8.)
                    && hint.right() <= viewport.width - px(8.)
                    && hint.bottom() <= viewport.height - px(8.)
            );
        }
    }
    #[test]
    fn large_host_hints_remain_next_to_the_actual_invoker() {
        for (host, control, viewport) in [
            (
                gpui::Bounds::new(
                    gpui::point(px(279.), px(13.)),
                    gpui::size(px(828.), px(140.)),
                ),
                gpui::Bounds::new(
                    gpui::point(px(1004.), px(34.)),
                    gpui::size(px(32.), px(32.)),
                ),
                gpui::size(px(1120.), px(716.)),
            ),
            (
                gpui::Bounds::new(
                    gpui::point(px(13.), px(54.)),
                    gpui::size(px(694.), px(240.)),
                ),
                gpui::Bounds::new(gpui::point(px(604.), px(94.)), gpui::size(px(32.), px(32.))),
                gpui::size(px(720.), px(580.)),
            ),
        ] {
            let tip = gpui::size(px(310.), px(94.));
            let origin = hint_origin(host, control, tip, viewport, true, false);
            let bounds = gpui::Bounds::new(origin, tip);
            assert!(!bounds.intersects(&host.dilate(px(6.))));
            assert!((bounds.right() - control.right()).abs() <= px(8.));
        }
    }
    #[test]
    fn active_icon_roles_keep_semantics_and_header_hierarchy() {
        for p in [Palette::Blue, Palette::Sage] {
            for a in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(p, a, gpui::WindowAppearance::Light);
                let secondary = icon_color(t, ButtonStyle::Secondary, true);
                assert!(
                    contrast(secondary, t.surface) >= 4.5
                        && contrast(secondary, t.surface) < contrast(t.text, t.surface)
                );
                assert_eq!(icon_color(t, ButtonStyle::Primary, true), t.on_accent);
                assert_eq!(icon_color(t, ButtonStyle::Danger, true), t.danger);
                for style in [
                    ButtonStyle::Primary,
                    ButtonStyle::Danger,
                    ButtonStyle::Secondary,
                    ButtonStyle::Quiet,
                ] {
                    let (bg, _, _) = control_colors(t, style, true);
                    for parent in [t.surface, t.subtle, t.canvas, t.chrome_end] {
                        assert!(
                            contrast(
                                icon_color(t, style, true),
                                if bg.a == 0. { parent } else { bg }
                            ) >= 4.5
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn stage_controls_keep_active_and_unavailable_contrast_distinct() {
        for p in [Palette::Blue, Palette::Sage] {
            for a in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(p, a, gpui::WindowAppearance::Light);
                for style in [ButtonStyle::Stage, ButtonStyle::StageDanger] {
                    let (bg, fg, _) = control_colors(t, style, false);
                    let ratio = contrast(bg, fg);
                    assert!(
                        (2.0..=3.6).contains(&ratio),
                        "{p:?}/{a:?}: unavailable stage {ratio}"
                    );
                    let (active_bg, active_fg, _) = control_colors(t, style, true);
                    assert!(contrast(active_bg, active_fg) >= 4.5);
                }
                assert!(
                    contrast(t.stage_text, t.stage) >= 3.
                        && contrast(t.stage_text, t.stage_panel) >= 3.
                );
            }
        }
    }
    #[test]
    fn unavailable_foregrounds_are_readable_on_every_actual_control_surface() {
        for palette in [Palette::Blue, Palette::Sage] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(palette, appearance, gpui::WindowAppearance::Light);
                for style in [
                    ButtonStyle::Primary,
                    ButtonStyle::Danger,
                    ButtonStyle::Secondary,
                    ButtonStyle::Quiet,
                ] {
                    let (bg, fg, _) = control_colors(t, style, false);
                    for parent in [t.surface, t.subtle, t.canvas, t.chrome_end] {
                        let drawn = if bg.a == 0. { parent } else { bg };
                        let ratio = contrast(fg, drawn);
                        assert!(
                            (2.0..=3.6).contains(&ratio),
                            "{palette:?}/{appearance:?}: {ratio}"
                        );
                        assert!(
                            ratio < contrast(t.muted, drawn) * 0.7,
                            "unavailable must remain weaker than active"
                        );
                        let (active_bg, active_fg, _) = control_colors(t, style, true);
                        let active_drawn = if active_bg.a == 0. { parent } else { active_bg };
                        let active_ratio = contrast(active_fg, active_drawn);
                        assert!(
                            active_ratio >= 4.5,
                            "{palette:?}/{appearance:?}: active {active_ratio}"
                        );
                    }
                }
            }
        }
    }
}
