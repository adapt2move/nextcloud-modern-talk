use crate::presentation::theme::Theme;
use gpui::{
    App, FontStyle, FontWeight, IntoElement, RenderOnce, StrikethroughStyle, StyledText, TextRun,
    TextStyle, UnderlineStyle, Window, px,
};
use std::ops::Range;
use talk_core::ui::markup::{Span, Style};

#[derive(gpui::IntoElement)]
pub(super) struct LineText {
    pub text: StyledText,
    pub range: Range<usize>,
    pub spans: Vec<Span>,
    pub selected: Option<Range<usize>>,
    pub theme: Theme,
}

// Selection and markup can overlap. Split them into disjoint UTF-8 runs so
// selection backgrounds never discard emphasis, code fonts or link styling.
pub(super) fn runs(
    range: Range<usize>,
    spans: &[Span],
    selected: Option<&Range<usize>>,
    base: TextStyle,
    theme: Theme,
) -> Vec<TextRun> {
    let mut cuts = vec![range.start, range.end];
    for span in spans {
        if span.range.start < range.end && span.range.end > range.start {
            cuts.push(span.range.start.max(range.start));
            cuts.push(span.range.end.min(range.end));
        }
    }
    if let Some(selected) = selected {
        cuts.push(selected.start.clamp(range.start, range.end));
        cuts.push(selected.end.clamp(range.start, range.end));
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|cut| {
            let [start, end] = *cut else {
                return None;
            };
            if start == end {
                return None;
            }
            let style = spans
                .iter()
                .find(|span| span.range.start <= start && span.range.end >= end)
                .map_or(Style::default(), |span| span.style);
            let mut run = base.to_run(end - start);
            if style.bold {
                run.font.weight = FontWeight::BOLD;
            }
            if style.italic {
                run.font.style = FontStyle::Italic;
            }
            if style.code {
                run.font.family = "monospace".into();
                run.background_color = Some(theme.subtle);
            }
            if style.strike {
                run.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                });
            }
            if style.link {
                run.color = theme.accent;
                run.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                });
            } else if style.marker {
                run.color = theme.muted;
            }
            if selected.is_some_and(|range| range.start <= start && range.end >= end) {
                run.background_color = Some(theme.selected);
            }
            Some(run)
        })
        .collect()
}

impl RenderOnce for LineText {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        self.text.with_runs(runs(
            self.range,
            &self.spans,
            self.selected.as_ref(),
            window.text_style(),
            self.theme,
        ))
    }
}
