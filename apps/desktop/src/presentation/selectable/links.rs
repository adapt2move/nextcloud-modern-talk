//! Frame-local hit/focus surfaces. All geometry derives from painted text;
//! activation emits a source identity and ordinal, never an external request.
use super::{
    MessageLinks, Selection,
    geometry::{Layouts, link_at, link_rects, mouse_index},
    interaction::Identity,
};
use crate::presentation::theme::Theme;
use gpui::{
    AnyElement, AvailableSpace, Bounds, CursorStyle, DispatchPhase, Entity, HitboxBehavior,
    IntoElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Size, canvas, div, point, prelude::*, px, size,
};
use talk_core::ui::markup::Link;

/// Hints use window coordinates, independently of a message/list's clip. A
/// lower timeline link may place its hint over the composer; deferred paint
/// keeps that passive hint visible above the ordinary window content.
pub(super) fn hint_origin(
    anchor: Bounds<Pixels>,
    hint: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let margin = px(8.);
    let gap = px(4.);
    let left = margin.min((viewport.width - hint.width).max(px(0.)));
    let right = (viewport.width - hint.width - margin).max(left);
    let top = margin.min((viewport.height - hint.height).max(px(0.)));
    let bottom = (viewport.height - hint.height - margin).max(top);
    let below = anchor.bottom() + gap;
    let above = anchor.top() - hint.height - gap;
    let y = if below <= bottom { below } else { above };
    point(anchor.left().clamp(left, right), y.clamp(top, bottom))
}

pub(super) fn surface(
    owner: Entity<Selection>,
    identity: Identity,
    layouts: Layouts,
    links: Vec<Link>,
    message: Option<MessageLinks>,
    theme: Theme,
    scale: f32,
) -> impl IntoElement {
    let paint_owner = owner.clone();
    let paint_identity = identity.clone();
    let paint_layouts = layouts.clone();
    let paint_message = message.clone();
    canvas(
        move |bounds, window, cx| {
            let clip = bounds.intersect(&window.content_mask().bounds);
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let rects = link_rects(&layouts, if message.is_some() { &links } else { &[] }, clip);
            let link_hitboxes = rects
                .iter()
                .map(|rect| window.insert_hitbox(rect.bounds, HitboxBehavior::Normal))
                .collect::<Vec<_>>();
            let mut controls = Vec::<AnyElement>::new();
            if let Some(message) = &message {
                for (ordinal, link) in links.iter().enumerate() {
                    let handle = message.handles.get(ordinal).and_then(Option::as_ref);
                    let Some(rect) = rects
                        .iter()
                        .find(|rect| usize::from(rect.ordinal) == ordinal)
                    else {
                        if handle.is_some_and(|handle| handle.is_focused(window)) {
                            owner.read(cx).focus.focus(window);
                        }
                        continue;
                    };
                    let ordinal = ordinal as u16;
                    if let Some(handle) = handle {
                        let down_owner = owner.clone();
                        let down_identity = identity.clone();
                        let up_owner = owner.clone();
                        let up_identity = identity.clone();
                        let up_handle = handle.clone();
                        let message = message.clone();
                        let mut control = div()
                            .id(("link-key", usize::from(ordinal)))
                            .w(rect.bounds.size.width)
                            .h(rect.bounds.size.height)
                            .track_focus(handle)
                            .key_context("MessageLink")
                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                let key = event.keystroke.key.as_str();
                                down_owner.update(cx, |owner, _| {
                                    if matches!(key, "enter" | "space") {
                                        owner.state.key_down(
                                            &down_identity,
                                            ordinal,
                                            key,
                                            event.is_held,
                                            event.keystroke.modifiers.modified(),
                                        );
                                    } else {
                                        owner.state.cancel_key();
                                    }
                                });
                                if matches!(key, "enter" | "space") {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                            })
                            .on_key_up(move |event: &KeyUpEvent, window, cx| {
                                let key = event.keystroke.key.as_str();
                                if matches!(key, "enter" | "space") {
                                    up_owner.update(cx, |owner, cx| {
                                        if up_handle.is_focused(window)
                                            && owner.state.key_up(
                                                &up_identity,
                                                ordinal,
                                                key,
                                                event.keystroke.modifiers.modified(),
                                            )
                                        {
                                            cx.emit(message.activation(ordinal));
                                        }
                                    });
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                            })
                            .into_any_element();
                        control.prepaint_as_root(
                            rect.bounds.origin,
                            size(
                                AvailableSpace::Definite(rect.bounds.size.width),
                                AvailableSpace::Definite(rect.bounds.size.height),
                            ),
                            window,
                            cx,
                        );
                        controls.push(control);
                    }
                    // Keep hint measurement and the glyph hit/focus surfaces
                    // separate. The hint has no cursor, focus, mouse handlers or
                    // occlusion, so it adds no popup hitbox over selectable text.
                    let hovered = link_at(&rects, window.mouse_position()) == Some(ordinal);
                    if hovered || handle.is_some_and(|handle| handle.is_focused(window)) {
                        let max_width = px(480.)
                            .min(window.viewport_size().width - px(16.))
                            .max(px(0.));
                        let mut hint = div()
                            .max_w(max_width)
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme.surface)
                            .border_1()
                            .border_color(theme.border)
                            .text_color(theme.text)
                            .text_size(px(12. * scale))
                            .line_height(px(18. * scale))
                            .truncate()
                            .child(link.destination.as_str().to_owned())
                            .into_any_element();
                        let hint_size = hint.layout_as_root(
                            size(
                                AvailableSpace::Definite(max_width),
                                AvailableSpace::MinContent,
                            ),
                            window,
                            cx,
                        );
                        let origin = hint_origin(rect.bounds, hint_size, window.viewport_size());
                        // Like GPUI's Deferred element, hand an already laid-out
                        // child to defer_draw during prepaint. GPUI prepaints and
                        // paints it after the root tree, outside the list mask.
                        // Both pointer and keyboard hints take this same path.
                        window.defer_draw(hint, origin, 0);
                    }
                }
            }
            (clip, hitbox, rects, link_hitboxes, controls)
        },
        move |_, (clip, hitbox, rects, link_hitboxes, mut controls), window, cx| {
            for control in &mut controls {
                control.paint(window, cx);
            }
            if let Some(message) = &paint_message {
                for (ordinal, handle) in message.handles.iter().enumerate() {
                    if handle
                        .as_ref()
                        .is_some_and(|handle| handle.is_focused(window))
                    {
                        for rect in rects
                            .iter()
                            .filter(|rect| usize::from(rect.ordinal) == ordinal)
                        {
                            let mut ring = gpui::outline(
                                rect.bounds,
                                theme.focus,
                                gpui::BorderStyle::default(),
                            );
                            ring.corner_radii = px(2.).into();
                            window.paint_quad(ring);
                        }
                    }
                }
            }
            for hitbox in &link_hitboxes {
                window.set_cursor_style(CursorStyle::PointingHand, hitbox);
            }
            let down_owner = paint_owner.clone();
            let down_identity = paint_identity.clone();
            let down_layouts = paint_layouts.clone();
            let down_rects = rects.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                    || !clip.contains(&event.position)
                {
                    return;
                }
                down_owner.update(cx, |owner, cx| {
                    owner.state.pointer_down(
                        down_identity.clone(),
                        mouse_index(&down_layouts, event.position, &down_identity.text),
                        event.click_count,
                        event.modifiers.shift,
                        event.position,
                        link_at(&down_rects, event.position),
                    );
                    owner.focus.focus(window);
                    cx.notify();
                });
                window.prevent_default();
                cx.stop_propagation();
            });
            let move_owner = paint_owner.clone();
            let move_identity = paint_identity.clone();
            let move_rects = rects.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                move_owner.update(cx, |owner, cx| {
                    let hover = link_at(&move_rects, event.position)
                        .map(|ordinal| (move_identity.clone(), ordinal));
                    let owned_hover = owner
                        .hover
                        .as_ref()
                        .is_some_and(|(target, _)| target == &move_identity);
                    let hover_changed = (hover.is_some() || owned_hover) && owner.hover != hover;
                    if hover_changed {
                        owner.hover = hover;
                    }
                    let dragging = owner.state.matches(&move_identity) && owner.state.dragging;
                    if dragging {
                        owner.state.pointer_move(
                            mouse_index(&paint_layouts, event.position, &move_identity.text),
                            event.position,
                            event.pressed_button == Some(MouseButton::Left),
                        );
                    }
                    if hover_changed || dragging {
                        cx.notify();
                    }
                });
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                paint_owner.update(cx, |owner, cx| {
                    if owner.state.matches(&paint_identity) && owner.state.dragging {
                        let ordinal = owner.state.pointer_up(
                            &paint_identity,
                            event.position,
                            link_at(&rects, event.position),
                            event.click_count,
                            event.modifiers.shift,
                        );
                        if let (Some(message), Some(ordinal)) = (&paint_message, ordinal) {
                            cx.emit(message.activation(ordinal));
                        }
                        cx.notify();
                    }
                });
            });
        },
    )
    .absolute()
    .inset_0()
}
