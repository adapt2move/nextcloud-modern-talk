use super::geometry::{clipped_rect, link_at};
use super::*;
use talk_core::ui::markup::Style;

fn identity(source: &str, display: &str) -> Identity {
    Identity {
        id: "account-a-message-1".into(),
        source: source.to_owned().into(),
        text: display.to_owned().into(),
    }
}
fn position(x: f32, y: f32) -> gpui::Point<gpui::Pixels> {
    gpui::point(px(x), px(y))
}

#[test]
fn copy_requires_same_scope_source_display_and_current_policy() {
    let mut state = SelectionState::default();
    let original = identity("**Hallo 🌍**", "Hallo 🌍");
    state.begin(original.clone(), 0, 3, false);
    assert_eq!(state.copied(&original, true), Some("Hallo 🌍"));
    let mut other_scope = original.clone();
    other_scope.id = "account-b-message-1".into();
    assert_eq!(state.copied(&other_scope, true), None);
    assert_eq!(
        state.copied(&identity("_Hallo 🌍_", "Hallo 🌍"), true),
        None
    );
    assert_eq!(
        state.copied(&identity("**Hallo 🌍**", "removed"), true),
        None
    );
    assert_eq!(state.copied(&original, false), None);
}

#[test]
fn selection_preserves_unicode_reversed_ranges_and_display_whitespace() {
    let mut state = SelectionState::default();
    let value = identity("source", "Grüße 👨‍👩‍👧‍👦 e\u{301}");
    state.begin(value.clone(), 3, 2, false);
    assert_eq!(state.copied(&value, true), Some("Grüße"));
    state.begin(value.clone(), value.text.len(), 1, false);
    state.begin(value.clone(), 9, 1, true);
    assert_eq!(state.copied(&value, true), Some("👨‍👩‍👧‍👦 e\u{301}"));
    state.extend(Edge::Right);
    assert_eq!(state.copied(&value, true), Some(" e\u{301}"));
    state.extend(Edge::End);
    assert_eq!(state.copied(&value, true), None);
    let large = identity("source", &"x".repeat(DISPLAY_BYTES + 1));
    state.begin(large, 0, 3, false);
    assert!(state.matches(&value));
}

#[test]
fn rich_copy_keeps_code_spaces_tabs_unicode_and_actual_line_endings() {
    let source = "**one**\n> Grüße\n```rust\n  let x = \"🌍\";  \n\tlast\n```";
    let block = Block::new(source.into(), true);
    assert_eq!(
        block.text.as_ref(),
        "one\n\nGrüße\n\n  let x = \"🌍\";  \n\tlast\n"
    );
    assert!(block.text.contains("  let x = \"🌍\";  \n\tlast"));
    assert!(block.lines.iter().any(|line| line.code));
    assert!(block.lines.iter().any(|line| line.quote));
    let value = identity(source, &block.text);
    let mut state = SelectionState::default();
    state.begin(value.clone(), 0, 3, false);
    assert_eq!(state.copied(&value, true), Some(block.text.as_ref()));
    let fallback = Block::new("x\n".repeat(300).into(), true);
    assert!(!fallback.lines.is_empty());
    assert_eq!(fallback.text.as_ref(), "x\n".repeat(300));
}

#[test]
fn only_matching_single_click_release_activates_once() {
    let mut state = SelectionState::default();
    let value = identity("[label](one)", "label");
    state.pointer_down(value.clone(), 1, 1, false, position(10., 10.), Some(0));
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(0), 1, false),
        Some(0)
    );
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(0), 1, false),
        None
    );
    for (clicks, shift) in [(2, false), (3, false), (1, true)] {
        state = SelectionState::default();
        state.pointer_down(value.clone(), 1, clicks, shift, position(10., 10.), Some(0));
        assert_eq!(
            state.pointer_up(&value, position(10., 10.), Some(0), clicks, shift),
            None
        );
    }
    state = SelectionState::default();
    state.pointer_down(value.clone(), 1, 1, false, position(10., 10.), Some(0));
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(1), 1, false),
        None
    );
}

#[test]
fn dragging_links_selects_and_never_activates_even_after_returning() {
    let value = identity("[label](one)", "label");
    let mut state = SelectionState::default();
    state.pointer_down(value.clone(), 0, 1, false, position(10., 10.), Some(0));
    state.pointer_move(4, position(30., 10.), true);
    assert_eq!(state.copied(&value, true), Some("labe"));
    state.pointer_move(0, position(10., 10.), true);
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(0), 1, false),
        None
    );
    state.pointer_down(value.clone(), 0, 1, false, position(10., 10.), Some(0));
    state.pointer_move(0, position(15., 10.), true);
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(0), 1, false),
        None
    );
    state.pointer_down(value.clone(), 0, 1, false, position(10., 10.), Some(0));
    state.pointer_move(0, position(10., 10.), false);
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(0), 1, false),
        None
    );
}

#[test]
fn selection_and_source_replacement_cancel_pointer_and_keyboard_authority() {
    let value = identity("[label](one)", "label");
    let replacement = identity("[label](two)", "label");
    let mut state = SelectionState::default();
    state.pointer_down(value.clone(), 1, 1, false, position(10., 10.), Some(0));
    assert_eq!(
        state.pointer_up(&replacement, position(10., 10.), Some(0), 1, false),
        None
    );
    state.begin(value.clone(), 0, 3, false);
    state.pointer_down(value.clone(), 1, 1, false, position(10., 10.), Some(0));
    assert_eq!(
        state.pointer_up(&value, position(10., 10.), Some(0), 1, false),
        None
    );
    state.key_down(&value, 0, "enter", false, false);
    assert!(!state.key_up(&replacement, 0, "enter", false));
    state.key_down(&value, 0, "space", false, false);
    assert!(state.key_up(&value, 0, "space", false));
    assert!(!state.key_up(&value, 0, "space", false));
    state.key_down(&value, 0, "space", false, true);
    assert!(!state.key_up(&value, 0, "space", false));
    state.key_down(&value, 0, "space", true, false);
    assert!(!state.key_up(&value, 0, "space", false));
}

#[test]
fn prepared_cache_reuses_source_and_enforces_both_budgets() {
    let mut documents = Documents::default();
    let source = concat!("[label](https", "://example.test)");
    assert_eq!(documents.link_count(source), 32);
    let block = documents.prepare(source.into());
    assert_eq!(block.links.len(), 1);
    assert_eq!(documents.link_count(source), 1);
    assert_eq!(documents.prepare(source.into()).text, block.text);
    assert_eq!(documents.retained(), (1, source.len()));
    for index in 0..110 {
        documents.prepare(format!("{index} **sample**").into());
    }
    assert_eq!(documents.retained().0, 96);
    for index in 0..80 {
        documents.prepare(format!("{index:04}{}", "x".repeat(4092)).into());
    }
    let (entries, bytes) = documents.retained();
    assert!(entries <= 96 && bytes <= 256 * 1024);
    documents.clear();
    assert_eq!(documents.retained(), (0, 0));
}

#[test]
fn frame_retirement_drops_edited_deleted_restricted_and_unmounted_sources() {
    let mut documents = Documents::default();
    documents.prepare("**old protected body**".into());
    documents.prepare("still mounted".into());
    documents.begin_frame();
    documents.prepare("_edited body_".into());
    documents.prepare("still mounted".into());
    documents.finish_frame();
    assert_eq!(documents.retained().0, 2);
    assert_eq!(documents.link_count("**old protected body**"), 32);
    assert_eq!(documents.link_count("still mounted"), 0);
    // Restricted rows deliberately don't admit a retained document. Deletion
    // and virtualization use the same missing-preparation retirement seam.
    documents.begin_frame();
    documents.prepare("still mounted".into());
    documents.finish_frame();
    assert_eq!(documents.retained(), (1, "still mounted".len()));
    assert_eq!(documents.link_count("_edited body_"), 32);
    documents.begin_frame();
    documents.finish_frame();
    assert_eq!(documents.retained(), (0, 0));
}

#[test]
fn a_retired_keyboard_source_cannot_activate_after_the_row_changes() {
    let value = identity("[label](one)", "label");
    let changed = identity("[label](two)", "label");
    let mut state = SelectionState::default();
    state.key_down(&value, 0, "enter", false, false);
    state.begin_frame();
    state.rendered(&value);
    state.finish_frame();
    assert!(state.key_up(&value, 0, "enter", false));
    state.key_down(&value, 0, "enter", false, false);
    state.begin_frame();
    state.rendered(&changed);
    state.finish_frame();
    assert!(!state.key_up(&value, 0, "enter", false));
}

#[test]
fn wrapped_link_fragments_are_clipped_separately_and_exclude_blank_space() {
    let bounds = |x1, y1, x2, y2| gpui::Bounds::from_corners(position(x1, y1), position(x2, y2));
    let clip = bounds(0., 12., 100., 36.);
    let first = clipped_rect(0, bounds(60., 0., 100., 22.), clip).unwrap();
    let second = clipped_rect(0, bounds(0., 22., 30., 44.), clip).unwrap();
    assert_eq!(first.bounds, bounds(60., 12., 100., 22.));
    assert_eq!(second.bounds, bounds(0., 22., 30., 36.));
    let rects = vec![first, second];
    assert_eq!(link_at(&rects, position(70., 15.)), Some(0));
    assert_eq!(link_at(&rects, position(10., 30.)), Some(0));
    for point in [
        position(70., 5.),
        position(50., 30.),
        position(30., 30.),
        position(10., 36.),
    ] {
        assert_eq!(link_at(&rects, point), None);
    }
    assert!(clipped_rect(1, bounds(0., 40., 40., 60.), clip).is_none());
}

#[test]
fn destination_hint_can_leave_the_timeline_clip_and_fit_the_window() {
    let timeline = gpui::Bounds::from_corners(position(0., 80.), position(640., 480.));
    let glyph = gpui::Bounds::from_corners(position(580., 460.), position(630., 480.));
    let viewport = gpui::size(px(640.), px(640.));
    let hint = gpui::size(px(220.), px(28.));
    let origin = super::links::hint_origin(glyph, hint, viewport);
    assert_eq!(origin, position(412., 484.));
    assert!(origin.y >= timeline.bottom());
    assert!(origin.x + hint.width <= viewport.width - px(8.));
    assert!(origin.y + hint.height <= viewport.height - px(8.));
    // A pointer and keyboard focus share the same passive overlay placement.
    // Its rectangle is never appended to the clipped glyph interaction set.
    let rects = [clipped_rect(0, glyph, timeline).unwrap()];
    assert_eq!(link_at(&rects, position(600., 470.)), Some(0));
    assert_eq!(link_at(&rects, origin + position(10., 10.)), None);
}

#[test]
fn destination_hint_flips_above_the_window_bottom_and_clamps_to_its_edges() {
    let viewport = gpui::size(px(640.), px(640.));
    let hint = gpui::size(px(220.), px(28.));
    let bottom = gpui::Bounds::from_corners(position(580., 610.), position(630., 632.));
    let origin = super::links::hint_origin(bottom, hint, viewport);
    assert_eq!(origin, position(412., 578.));
    assert!(origin.y + hint.height < bottom.top());
    let top = gpui::Bounds::from_corners(position(0., 8.), position(20., 30.));
    assert_eq!(
        super::links::hint_origin(top, hint, viewport),
        position(8., 34.)
    );
    let narrow = gpui::size(px(120.), px(80.));
    let small_hint = gpui::size(px(104.), px(28.));
    let corner = gpui::Bounds::from_corners(position(110., 62.), position(120., 80.));
    let origin = super::links::hint_origin(corner, small_hint, narrow);
    assert_eq!(origin, position(8., 30.));
    assert!(origin.x + small_hint.width <= narrow.width - px(8.));
    assert!(origin.y + small_hint.height < corner.top());
}

#[test]
fn selection_style_splits_runs_without_losing_code_emphasis_or_link_decorations() {
    let style = Style {
        bold: true,
        italic: true,
        strike: true,
        code: true,
        link: true,
        ..Style::default()
    };
    let spans = [talk_core::ui::markup::Span { range: 0..8, style }];
    let theme = Theme::resolve(
        crate::presentation::theme::Palette::default(),
        crate::presentation::theme::Appearance::default(),
        gpui::WindowAppearance::Light,
    );
    let runs = styles::runs(
        0..8,
        &spans,
        Some(&(2..6)),
        gpui::TextStyle::default(),
        theme,
    );
    assert_eq!(
        runs.iter().map(|run| run.len).collect::<Vec<_>>(),
        [2, 4, 2]
    );
    for run in &runs {
        assert_eq!(run.font.family.as_ref(), "monospace");
        assert_eq!(run.font.weight, gpui::FontWeight::BOLD);
        assert_eq!(run.font.style, gpui::FontStyle::Italic);
        assert!(run.underline.is_some() && run.strikethrough.is_some());
    }
    assert_eq!(runs[1].background_color, Some(theme.selected));
    assert_eq!(runs[0].background_color, Some(theme.subtle));
}
