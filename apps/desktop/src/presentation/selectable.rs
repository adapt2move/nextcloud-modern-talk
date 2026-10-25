//! Read-only native Markdown and link interaction. One entity owns selection,
//! bounded preparation and gesture state for the desktop; rows own no I/O.
mod document;
mod geometry;
mod interaction;
mod links;
mod styles;

use super::{input, theme::Theme};
use crate::presentation::selectable::document::{Block, DISPLAY_BYTES, Documents, SOURCE_BYTES};
use crate::presentation::selectable::interaction::{Edge, Identity, SelectionState};
use gpui::{
    App, Context, CursorStyle, ElementId, Entity, EventEmitter, FocusHandle, IntoElement,
    KeyBinding, RenderOnce, SharedString, StyledText, Window, div, prelude::*, px,
};
use talk_core::ui::{Text, presentation::ConversationRef};

/// Deliberate activation only. Composition revalidates this source/ordinal in
/// its current scoped message projection before the external-link owner acts.
pub(super) struct LinkActivation {
    pub context: ConversationRef,
    pub message: u128,
    pub source: Text<4096>,
    pub ordinal: u16,
}

pub(super) struct Selection {
    state: SelectionState,
    focus: FocusHandle,
    live: bool,
    documents: Documents,
    context: Option<ConversationRef>,
    hover: Option<(Identity, u16)>,
    hover_live: bool,
}
impl EventEmitter<LinkActivation> for Selection {}

impl Selection {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: SelectionState::default(),
            focus: cx.focus_handle().tab_stop(false),
            live: false,
            documents: Documents::default(),
            context: None,
            hover: None,
            hover_live: false,
        }
    }
    pub fn begin_frame(&mut self) {
        self.live = false;
        self.hover_live = false;
        self.documents.begin_frame();
        self.state.begin_frame();
    }
    pub fn finish_frame(
        &mut self,
        window: &mut Window,
        fallback: &FocusHandle,
        cx: &mut Context<Self>,
    ) {
        self.documents.finish_frame();
        self.state.finish_frame();
        if !self.hover_live {
            self.hover = None;
        }
        if !self.live {
            if self.focus.is_focused(window) {
                fallback.focus(window);
            }
            if self.state.target.is_some() {
                self.clear(cx);
            }
        }
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.state = SelectionState::default();
        self.documents.clear();
        self.hover = None;
        self.context = None;
        cx.notify();
    }
    /// The first frame reserves a finite maximum. Following frames admit only
    /// prepared links; no parsing or interior mutation is needed by Desktop.
    pub fn link_count(&self, source: &str) -> usize {
        self.documents.link_count(source)
    }
    fn prepare(&mut self, source: SharedString, context: Option<ConversationRef>) -> Block {
        if context.is_some() && self.context != context {
            self.state = SelectionState::default();
            self.documents.clear();
            self.hover = None;
            self.context = context;
        }
        self.documents.prepare(source)
    }
}

#[derive(Clone)]
struct MessageLinks {
    context: ConversationRef,
    message: u128,
    source: Text<4096>,
    handles: Vec<Option<FocusHandle>>,
}
impl MessageLinks {
    fn activation(&self, ordinal: u16) -> LinkActivation {
        LinkActivation {
            context: self.context,
            message: self.message,
            source: self.source.clone(),
            ordinal,
        }
    }
}

#[derive(IntoElement)]
pub(super) struct SelectableText {
    id: ElementId,
    source: SharedString,
    selection: Entity<Selection>,
    theme: Theme,
    permitted: bool,
    rich: bool,
    scale: f32,
    links: Option<MessageLinks>,
}
impl SelectableText {
    pub fn new(
        id: impl Into<ElementId>,
        text: &str,
        selection: Entity<Selection>,
        theme: Theme,
        permitted: bool,
    ) -> Self {
        let mut end = text.len().min(SOURCE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            id: id.into(),
            source: text[..end].to_owned().into(),
            selection,
            theme,
            permitted: permitted && text.len() <= SOURCE_BYTES,
            rich: false,
            scale: 1.,
            links: None,
        }
    }
    pub fn rich(mut self, text: &str, scale: f32) -> Self {
        // Admission precedes allocation, including callers outside message rows.
        let mut end = text.len().min(SOURCE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.permitted &= text.len() <= SOURCE_BYTES;
        self.source = text[..end].to_owned().into();
        self.rich = true;
        self.scale = scale;
        self
    }
    pub fn links(
        mut self,
        context: ConversationRef,
        message: u128,
        source: Text<4096>,
        mut handles: Vec<Option<FocusHandle>>,
    ) -> Self {
        handles.truncate(document::MAX_LINKS);
        if self.permitted && source.as_str() == self.source.as_ref() {
            self.links = Some(MessageLinks {
                context,
                message,
                source,
                handles,
            });
        }
        self
    }
}

fn extend<A>(
    owner: Entity<Selection>,
    identity: Identity,
    edge: Edge,
) -> impl Fn(&A, &mut Window, &mut App) {
    move |_, _, cx| {
        owner.update(cx, |owner, cx| {
            if owner.state.matches(&identity) {
                owner.state.extend(edge);
                cx.notify();
            }
        });
        cx.stop_propagation();
    }
}

impl RenderOnce for SelectableText {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            source,
            selection,
            theme,
            permitted,
            rich,
            scale,
            links,
        } = self;
        let block = if rich && permitted {
            selection.update(cx, |owner, _| {
                owner.prepare(source, links.as_ref().map(|links| links.context))
            })
        } else {
            Block::new(source, rich)
        };
        let identity = Identity {
            id: id.clone(),
            source: block.source.clone(),
            text: block.text.clone(),
        };
        let selected = permitted && selection.read(cx).state.matches(&identity);
        if permitted {
            selection.update(cx, |owner, _| {
                owner.state.rendered(&identity);
                owner.hover_live |= owner
                    .hover
                    .as_ref()
                    .is_some_and(|(target, _)| target == &identity);
            });
        }
        let focus = selection.read(cx).focus.clone();
        let range = selected.then(|| selection.read(cx).state.range());
        if selected {
            selection.update(cx, |owner, _| owner.live = true);
        }
        let mut body = div()
            .min_w_0()
            .relative()
            .when(rich, |d| d.flex().flex_col().line_height(px(22. * scale)));
        let mut layouts = Vec::with_capacity(block.lines.len());
        for line in &block.lines {
            let Some(content) = block.text.get(line.range.clone()) else {
                continue;
            };
            let text = StyledText::new(content.to_owned());
            layouts.push((line.range.clone(), text.layout().clone()));
            let spans = block
                .spans
                .iter()
                .filter(|span| {
                    span.range.start < line.range.end && span.range.end > line.range.start
                })
                .cloned()
                .collect();
            body = body.child(
                div()
                    .min_w_0()
                    .when(rich, |d| d.min_h(px(22. * scale)))
                    .when(line.code, |d| {
                        d.font_family("monospace").bg(theme.subtle).px_2()
                    })
                    .when(line.heading > 0, |d| {
                        d.text_size(px((22. - f32::from(line.heading.min(6))) * scale))
                            .font_weight(gpui::FontWeight::BOLD)
                    })
                    .when(line.quote, |d| {
                        d.relative().pl(px(11.)).text_color(theme.muted).child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(3.))
                                .bottom(px(3.))
                                .w(px(3.))
                                .rounded_full()
                                .bg(theme.accent),
                        )
                    })
                    .child(styles::LineText {
                        text,
                        range: line.range.clone(),
                        spans,
                        selected: range.clone(),
                        theme,
                    }),
            );
        }
        if !permitted {
            return body.into_any_element();
        }
        body = body.child(links::surface(
            selection.clone(),
            identity.clone(),
            layouts.clone(),
            block.links,
            links,
            theme,
            scale,
        ));
        let copy_owner = selection.clone();
        let copy_identity = identity.clone();
        let all_owner = selection.clone();
        let all_identity = identity.clone();
        body.id(id)
            .cursor(CursorStyle::IBeam)
            .when(selected, |d| {
                d.track_focus(&focus).key_context("SelectableText")
            })
            .on_action(move |_: &input::Copy, _, cx| {
                if let Some(text) = copy_owner.read(cx).state.copied(&copy_identity, permitted) {
                    input::copy_text_selection(text, 0..text.len(), permitted, DISPLAY_BYTES, cx);
                }
                cx.stop_propagation();
            })
            .on_action(extend::<input::SelectLeft>(
                selection.clone(),
                identity.clone(),
                Edge::Left,
            ))
            .on_action(extend::<input::SelectRight>(
                selection.clone(),
                identity.clone(),
                Edge::Right,
            ))
            .on_action(extend::<input::SelectDocumentStart>(
                selection.clone(),
                identity.clone(),
                Edge::Start,
            ))
            .on_action(extend::<input::SelectDocumentEnd>(
                selection,
                identity,
                Edge::End,
            ))
            .on_action(move |_: &input::SelectAll, _, cx| {
                all_owner.update(cx, |owner, cx| {
                    if owner.state.matches(&all_identity) {
                        owner.state.anchor = 0;
                        owner.state.head = all_identity.text.len();
                        owner.state.cancel_pointer();
                        owner.state.cancel_key();
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            })
            .into_any_element()
    }
}

pub(super) fn bind_keys(cx: &mut App) {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{primary}-c"), input::Copy, Some("SelectableText")),
        KeyBinding::new(
            &format!("{primary}-a"),
            input::SelectAll,
            Some("SelectableText"),
        ),
        KeyBinding::new("shift-left", input::SelectLeft, Some("SelectableText")),
        KeyBinding::new("shift-right", input::SelectRight, Some("SelectableText")),
        KeyBinding::new(
            &format!("{primary}-shift-home"),
            input::SelectDocumentStart,
            Some("SelectableText"),
        ),
        KeyBinding::new(
            &format!("{primary}-shift-end"),
            input::SelectDocumentEnd,
            Some("SelectableText"),
        ),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new(
            "cmd-shift-up",
            input::SelectDocumentStart,
            Some("SelectableText"),
        ),
        KeyBinding::new(
            "cmd-shift-down",
            input::SelectDocumentEnd,
            Some("SelectableText"),
        ),
    ]);
}

#[cfg(test)]
mod tests;
