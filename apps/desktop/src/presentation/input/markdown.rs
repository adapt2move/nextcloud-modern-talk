//! Styling over original source byte ranges. The editable text, native preedit
//! and selection model stay unchanged; IME underline wins over link decoration.
use gpui::{FontStyle, FontWeight, StrikethroughStyle, TextRun, UnderlineStyle, px};
use std::ops::Range;
use talk_core::ui::markup::Span;

pub(super) fn runs(
    text: &str,
    base: TextRun,
    spans: &[Span],
    marked: Option<Range<usize>>,
    theme: crate::presentation::theme::Theme,
) -> Vec<TextRun> {
    let valid = |range: &Range<usize>| {
        range.start <= range.end
            && range.end <= text.len()
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end)
    };
    let marked = marked.filter(valid);
    // The parser supplies at most512 disjoint spans. Never allocate according
    // to an unchecked external span count, even if a caller violates that API.
    let spans = spans.get(..spans.len().min(512)).unwrap_or(&[]);
    let mut edges = Vec::with_capacity(spans.len() * 2 + 4);
    edges.extend([0, text.len()]);
    for span in spans.iter().filter(|span| valid(&span.range)) {
        edges.extend([span.range.start, span.range.end]);
    }
    if let Some(marked) = &marked {
        edges.extend([marked.start, marked.end]);
    }
    edges.sort_unstable();
    edges.dedup();
    let mut result = Vec::with_capacity(edges.len().saturating_sub(1));
    for pair in edges.windows(2) {
        let [start, end] = *pair else { continue };
        if start == end {
            continue;
        }
        let index = spans.partition_point(|span| span.range.end <= start);
        let style = spans
            .get(index)
            .filter(|span| valid(&span.range) && span.range.start <= start && end <= span.range.end)
            .map(|span| span.style)
            .unwrap_or_default();
        let mut run = TextRun {
            len: end - start,
            ..base.clone()
        };
        if style.bold || style.heading > 0 {
            run.font.weight = FontWeight::BOLD;
        }
        if style.italic {
            run.font.style = FontStyle::Italic;
        }
        if style.code {
            run.font.family = "Google Sans Code".into();
            run.background_color = Some(theme.subtle);
        }
        if style.quote || style.marker {
            run.color = theme.muted;
        }
        if style.link {
            run.color = theme.accent;
            run.underline = Some(UnderlineStyle {
                color: Some(theme.accent),
                thickness: px(1.),
                wavy: false,
            });
        }
        if style.strike {
            run.strikethrough = Some(StrikethroughStyle {
                color: Some(run.color),
                thickness: px(1.),
            });
        }
        if marked
            .as_ref()
            .is_some_and(|range| range.start < end && start < range.end)
        {
            run.underline = Some(UnderlineStyle {
                color: Some(base.color),
                thickness: px(1.),
                wavy: false,
            });
        }
        result.push(run);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::markup::{Style, parse};
    fn base() -> (TextRun, crate::presentation::theme::Theme) {
        let theme = crate::presentation::theme::Theme::resolve(
            talk_core::preferences::Palette::default(),
            talk_core::preferences::Appearance::Light,
            gpui::WindowAppearance::Light,
        );
        (
            TextRun {
                len: 0,
                font: gpui::font("Google Sans"),
                color: theme.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            },
            theme,
        )
    }
    #[test]
    fn source_runs_preserve_unicode_bytes_and_split_formatting_at_ime_boundaries() {
        let source = "**Ä👩🏽‍💻中** und `code`";
        let document = parse(source);
        let (base, theme) = base();
        let marked = source.find('中').unwrap()..source.find('中').unwrap() + '中'.len_utf8();
        let result = runs(
            source,
            base.clone(),
            document.source_spans.as_slice(),
            Some(marked.clone()),
            theme,
        );
        assert_eq!(result.iter().map(|r| r.len).sum::<usize>(), source.len());
        let mut offset = 0;
        let mut marked_runs = 0;
        for run in result {
            assert!(source.is_char_boundary(offset));
            if offset == marked.start {
                assert_eq!(run.len, marked.len());
                assert_eq!(run.font.weight, FontWeight::BOLD);
                assert_eq!(run.underline.unwrap().color, Some(base.color));
                marked_runs += 1;
            }
            offset += run.len;
        }
        assert_eq!(marked_runs, 1);
    }

    #[test]
    fn invalid_or_partial_ranges_never_remove_source_or_panic_and_preedit_is_visible() {
        let text = "é汉字";
        let (base, theme) = base();
        let spans = [Span {
            range: 1..3,
            style: Style {
                bold: true,
                ..Default::default()
            },
        }];
        let result = runs(text, base, &spans, Some(2..text.len()), theme);
        assert_eq!(result.iter().map(|r| r.len).sum::<usize>(), text.len());
        assert_eq!(result.len(), 2);
        assert!(result[1].underline.is_some());
    }
}
