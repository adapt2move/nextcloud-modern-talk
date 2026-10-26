use super::*;
use crate::presentation::input::edit::{
    EditHistory, EditSnapshot, Insertion, replacement, to_utf16,
};

fn load(source: &str) -> Document {
    Document::import(source, MAX_BYTES).expect("admitted fixture")
}
fn state(source: &str, selection: Range<usize>) -> EditSnapshot {
    let rich = load(source);
    EditSnapshot {
        text: rich.text.clone(),
        rich: Some(rich),
        selection,
        reversed: false,
    }
}
fn type_text(doc: Document, text: &str) -> Document {
    let end = doc.text.len();
    doc.replace(end..end, text, true, MAX_BYTES)
        .unwrap_or_else(|| panic!("typing {text:?} into {doc:?}"))
        .0
}

#[test]
fn imports_emphasis_blocks_links_and_keeps_unsupported_content_inert() {
    let doc = load(concat!(
        "# Heading\n- **Ä🧭** and *文*\n> ~~old~~ [site](https:",
        "//example.test/a)\n<img src=\"x\"> ![alt](image.png)"
    ));
    assert_eq!(
        doc.text.as_ref(),
        "Heading\nÄ🧭 and 文\nold site\n<img src=\"x\"> ![alt](image.png)"
    );
    assert_eq!(doc.blocks[0].heading, 1);
    assert!(doc.blocks[1].list.is_some());
    assert_eq!(doc.blocks[2].quote, 1);
    assert_eq!(doc.links[0].as_ref(), concat!("https:", "//example.test/a"));
    assert!(doc.spans.iter().any(|span| span.style.bold));
    assert!(doc.spans.iter().any(|span| span.style.italic));
    assert!(doc.spans.iter().any(|span| span.style.strike));
    assert!(doc.spans.iter().any(|span| span.style.link));
    let restored = load(&doc.wire);
    assert_eq!(restored.text, doc.text);
    assert_eq!(restored.blocks, doc.blocks);
    assert_eq!(restored.links, doc.links);
}

#[test]
fn balanced_typing_consumes_syntax_and_native_caret_uses_visible_unicode() {
    for (source, expected, flag) in [
        ("**Ä🧭**", "Ä🧭", BOLD),
        ("*文*", "文", ITALIC),
        ("~~old~~", "old", STRIKE),
        ("`a * b`", "a * b", CODE),
    ] {
        let mut doc = load("");
        for ch in source.chars() {
            doc = type_text(doc, &ch.to_string());
        }
        assert_eq!(doc.text.as_ref(), expected, "{source}");
        assert!(
            doc.marks.iter().all(|mark| mark.flags & flag != 0),
            "{source}"
        );
        assert_eq!(load(&doc.wire).text, doc.text);
        doc = type_text(doc, "!");
        assert_eq!(doc.marks.last().unwrap().flags & flag, 0);
    }
}

#[test]
fn block_shortcuts_continue_and_empty_items_exit_without_a_newline() {
    for prefix in ["# ", "- ", "1. ", "> ", "``` "] {
        let doc = type_text(load(""), prefix);
        assert_eq!(doc.text.as_ref(), "", "{prefix}");
        assert_ne!(doc.blocks[0], Block::default());
        let doc = type_text(doc, "one");
        let (doc, selected) = doc.newline(3..3, MAX_BYTES).unwrap();
        assert_eq!(doc.text.as_ref(), "one\n");
        assert_eq!(selected, 4..4);
        if prefix == "# " {
            assert_eq!(doc.blocks[1], Block::default());
        }
        if prefix == "1. " {
            assert_eq!(doc.blocks[1].list.unwrap().number, Some(2));
        }
        if matches!(prefix, "- " | "1. " | "> ") {
            let (doc, selected) = doc.newline(4..4, MAX_BYTES).unwrap();
            assert_eq!(doc.text.as_ref(), "one\n");
            assert_eq!(selected, 4..4);
            assert_eq!(doc.blocks[1], Block::default());
        }
    }
}

#[test]
fn code_keeps_whitespace_and_literal_syntax_during_edit_and_restore() {
    let doc = load("```rust\n  **x**\n\n\t y  \n```\nafter");
    assert_eq!(doc.text.as_ref(), "  **x**\n\n\t y  \nafter");
    assert!(doc.wire.contains("  **x**\n\n\t y  \n```"));
    assert_eq!(load(&doc.wire).text, doc.text);
    let (doc, _) = doc.replace(2..2, "**new**", true, MAX_BYTES).unwrap();
    assert!(doc.text.starts_with("  **new****x**"));
    assert!(doc.wire.contains("  **new****x**"));
    let inline = load("` **x** `");
    assert_eq!(inline.text.as_ref(), "**x**");
    let inline = type_text(inline, "**literal**");
    assert_eq!(inline.text.as_ref(), "**x****literal**");
    assert!(inline.marks.iter().all(|m| m.flags & CODE != 0));
}

#[test]
fn escaped_punctuation_and_code_backticks_round_trip_without_injecting_markup() {
    for source in [
        r"\*literal\*",
        r"\# title",
        r"1\. item",
        "`` `code` ``",
        "`  a  `",
        "`   `",
        "**bold _mixed_ end**",
    ] {
        let doc = load(source);
        let restored = load(&doc.wire);
        assert_eq!(restored.text, doc.text, "{source}: {}", doc.wire);
        let display = talk_core::ui::markup::parse(&doc.wire);
        assert_eq!(
            display.text.as_str(),
            doc.text.as_ref(),
            "{source}: {}",
            doc.wire
        );
    }
    let doc = type_text(load(r"\*literal"), "*");
    assert_eq!(doc.text.as_ref(), "*literal*");
    assert!(doc.marks.iter().all(|m| m.flags & ITALIC == 0));
}

#[test]
fn formatting_and_visible_selection_are_restored_atomically_by_history() {
    let before = state("Ä🧭 text", 0..6);
    let mut after = before.clone();
    after.rich = Some(
        before
            .rich
            .as_ref()
            .unwrap()
            .toggle(0..6, Format::Bold, MAX_BYTES)
            .unwrap(),
    );
    let mut history = EditHistory::default();
    history.record(before.clone());
    assert_eq!(after.value(), "**Ä🧭** text");
    assert_eq!(after.text, before.text);
    let restored = history.travel(after.clone(), false).unwrap();
    assert_eq!(restored, before);
    assert_eq!(history.travel(restored, true), Some(after));
}

#[test]
fn collapsed_shortcuts_set_typing_marks_and_undo_restores_them() {
    let mut doc = load("a");
    doc = doc.toggle(1..1, Format::Italic, MAX_BYTES).unwrap();
    doc = type_text(doc, "文");
    assert_eq!(doc.wire.as_ref(), "a*文*");
    doc.clear_typing();
    let plain = load("x").toggle(0..1, Format::Code, MAX_BYTES).unwrap();
    assert_eq!(plain.wire.as_ref(), "`x`");
}

#[test]
fn repeated_utf16_preedit_stays_visible_untransformed_and_commits_once() {
    let before = state("**A🧭** Z", 1..5);
    let mut current = before.clone();
    let mut marked = None;
    let mut history = EditHistory::default();
    for text in ["n", "ni", "**你🧭**"] {
        let next = replacement(
            &current,
            marked.as_ref(),
            None,
            Insertion::Mark {
                text,
                selection_utf16: Some(2..5),
            },
            MAX_BYTES,
            true,
        )
        .unwrap_or_else(|| panic!("preedit {text:?} current {current:?}"));
        history.begin_composition(current);
        current = next.state;
        marked = next.marked;
        assert_eq!(history.committed_value(current.value()), before.value());
        assert!(current.text.contains(text));
    }
    let next = replacement(
        &current,
        marked.as_ref(),
        None,
        Insertion::Commit("**你🧭**"),
        MAX_BYTES,
        true,
    )
    .unwrap();
    current = next.state;
    assert_eq!(current.text.as_ref(), "A你🧭 Z");
    assert_eq!(current.selection, 8..8);
    assert_eq!(to_utf16(&current.text, current.selection.start), 4);
    assert!(next.marked.is_none());
    history.finish_composition(current.value());
    let restored = history.travel(current.clone(), false).unwrap();
    assert_eq!(restored, before);
    assert!(history.travel(restored.clone(), false).is_none());
    assert_eq!(history.travel(restored, true), Some(current));
}

#[test]
fn selected_deletion_across_marks_and_blocks_never_readdresses_wire_offsets() {
    let before = state("**Ä🧭** _文_\n- tail", 2..10);
    let next = replacement(
        &before,
        None,
        Some(1..5),
        Insertion::Commit(""),
        MAX_BYTES,
        true,
    )
    .unwrap();
    assert_eq!(next.state.text.as_ref(), "Ä\ntail");
    assert_eq!(next.state.selection, 2..2);
    assert_eq!(next.state.value(), "**Ä**\n- tail");
    assert_eq!(load(next.state.value()).text, next.state.text);
}

#[test]
fn serialized_limit_rejects_formatting_growth_and_preserves_prior_document() {
    let doc = load(&"x".repeat(4096));
    assert!(doc.toggle(0..4096, Format::Bold, MAX_BYTES).is_none());
    assert!(doc.replace(4096..4096, "🧭", true, MAX_BYTES).is_none());
    assert_eq!(doc.text.len(), 4096);
    assert_eq!(doc.wire.len(), 4096);
    assert!(Document::import(&"*".repeat(4096), MAX_BYTES).is_none());
    assert!(Document::import(&"\n".repeat(256), MAX_BYTES).is_none());
    let last = load(&"x".repeat(4092))
        .toggle(0..4092, Format::Bold, MAX_BYTES)
        .unwrap();
    assert_eq!(last.wire.len(), 4096);
    assert_eq!(load(&last.wire).text, last.text);
}

#[test]
fn markdown_paste_is_one_semantic_edit_and_code_paste_is_literal() {
    let before = state("a end", 2..2);
    let next = replacement(
        &before,
        None,
        None,
        Insertion::Paste("**Ä**\n- b"),
        MAX_BYTES,
        true,
    )
    .unwrap();
    assert_eq!(next.state.text.as_ref(), "a Ä\nbend");
    assert_eq!(load(next.state.value()).text, next.state.text);
    let doc = load("`code`");
    let end = doc.text.len();
    let (doc, _) = doc.paste(end..end, "**x**", MAX_BYTES).unwrap();
    assert_eq!(doc.text.as_ref(), "code**x**");
    assert!(doc.wire.contains("code**x**"));
}

#[test]
fn successive_pastes_release_dead_link_and_code_tables() {
    let mut doc = load("");
    for n in 0..80 {
        doc = doc
            .paste(
                0..doc.text.len(),
                &format!(concat!("[x](https:", "//example.test/{n})"), n = n),
                MAX_BYTES,
            )
            .unwrap()
            .0;
        assert_eq!(doc.links.len(), 1);
    }
    for _ in 0..80 {
        doc = doc
            .paste(0..doc.text.len(), "```\nx\n```", MAX_BYTES)
            .unwrap()
            .0;
        assert_eq!(doc.codes.len(), 1);
    }
}

#[test]
fn block_and_inline_code_fences_cannot_be_closed_by_literal_backticks() {
    let doc = load("````\n```\nend\n````");
    assert_eq!(load(&doc.wire).text, doc.text);
    let doc = load("abc").toggle(0..3, Format::Code, MAX_BYTES).unwrap();
    let (doc, _) = doc.replace(1..1, "````", true, MAX_BYTES).unwrap();
    assert_eq!(doc.text.as_ref(), "a````bc");
    assert_eq!(load(&doc.wire).text, doc.text);
}

#[test]
fn deterministic_edits_keep_unicode_and_all_native_ranges_within_visible_text() {
    let mut doc = load("**Ä🧭** _文_\n- list");
    let mut seed = 17u32;
    for _ in 0..200 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let offsets: Vec<_> = doc
            .text
            .char_indices()
            .map(|(i, _)| i)
            .chain([doc.text.len()])
            .collect();
        let at = offsets[seed as usize % offsets.len()];
        let text = ["🧭", "e\u{301}", "文", "*", " ", ""][seed as usize % 6];
        if let Some((next, selection)) = doc.replace(at..at, text, true, MAX_BYTES) {
            assert!(next.text.is_char_boundary(selection.start));
            assert!(next.text.is_char_boundary(selection.end));
            assert_eq!(next.marks.len(), next.text.len());
            assert!(next.wire.len() <= MAX_BYTES);
            assert_eq!(load(&next.wire).text, next.text, "{}", next.wire);
            doc = next;
        }
    }
}

#[test]
fn arbitrary_unicode_mark_boundaries_export_without_invisible_character_injection() {
    let doc = load("**Ä🧭**");
    let doc = type_text(doc, "z");
    assert_eq!(doc.text.as_ref(), "Ä🧭z");
    assert_eq!(load(&doc.wire).text, doc.text);
    let rendered = talk_core::ui::markup::parse(&doc.wire);
    assert_eq!(rendered.text.as_str(), doc.text.as_ref());
    assert!(rendered.spans.as_slice().iter().any(|span| span.style.bold));
    let doc = load("a!b").toggle(1..2, Format::Bold, MAX_BYTES).unwrap();
    assert_eq!(load(&doc.wire).text, doc.text);
    let rendered = talk_core::ui::markup::parse(&doc.wire);
    assert_eq!(rendered.text.as_str(), "a!b");
    assert!(
        rendered
            .spans
            .as_slice()
            .iter()
            .any(|span| span.style.bold && span.range == (1..2))
    );
}

#[test]
fn entity_import_and_literal_entity_typing_keep_their_distinct_meanings() {
    let doc = load("&#129517; &amp; &#x6587; &nbsp;");
    assert_eq!(doc.text.as_ref(), "🧭 & 文 \u{a0}");
    assert_eq!(load(&doc.wire).text, doc.text);
    let doc = type_text(load(""), "&#129517;");
    assert_eq!(doc.text.as_ref(), "&#129517;");
    assert_eq!(load(&doc.wire).text, doc.text);
    let doc = load("    literal indentation");
    assert_eq!(doc.text.as_ref(), "literal indentation");
    assert!(doc.blocks[0].code.is_some());
    assert_eq!(load(&doc.wire).text, doc.text);
}

#[test]
fn code_line_endings_and_rejected_preedit_remain_transactional() {
    let doc = load("```\r\n  a\r\n\r\n```\r\nafter");
    assert_eq!(doc.text.as_ref(), "  a\n\nafter");
    assert_eq!(load(&doc.wire).text, doc.text);
    let before = state("**text**", 4..4);
    let mut history = EditHistory::default();
    let marked = replacement(
        &before,
        None,
        None,
        Insertion::Mark {
            text: "文",
            selection_utf16: Some(0..1),
        },
        MAX_BYTES,
        true,
    )
    .unwrap();
    history.begin_composition(before.clone());
    assert!(
        replacement(
            &marked.state,
            marked.marked.as_ref(),
            None,
            Insertion::Mark {
                text: &"🧭".repeat(1024),
                selection_utf16: None
            },
            MAX_BYTES,
            true
        )
        .is_none()
    );
    assert_eq!(
        history.committed_value(marked.state.value()),
        before.value()
    );
    assert_eq!(marked.state.text.as_ref(), "text文");
    let (doc, selection) = marked
        .state
        .rich
        .as_ref()
        .unwrap()
        .commit_preedit(marked.state.selection.clone(), MAX_BYTES)
        .unwrap();
    assert_eq!(selection, 4..7);
    history.finish_composition(&doc.wire);
    assert_eq!(
        history.travel(
            EditSnapshot {
                text: doc.text.clone(),
                rich: Some(doc),
                selection,
                reversed: false
            },
            false
        ),
        Some(before)
    );
}

#[test]
fn literal_or_inline_code_prefixes_never_become_block_shortcuts() {
    for source in [
        r"\#",
        r"\-",
        r"\>",
        r"\`\`\`",
        "`#`",
        "`-`",
        "`>`",
        "`` ``` ``",
    ] {
        let doc = load(source);
        let expected = format!("{} ", doc.text);
        let doc = type_text(doc, " ");
        assert_eq!(doc.text.as_ref(), expected, "{source}");
        assert_eq!(doc.blocks[0], Block::default(), "{source}");
        assert_eq!(load(&doc.wire).text, doc.text);
    }
}

#[test]
fn sequential_task_typing_converts_an_empty_list_item_and_respects_literal_markers() {
    for (source, number, checked) in [
        ("- [ ] task", None, false),
        ("- [x] task", None, true),
        ("- [X] task", None, true),
        ("1. [ ] task", Some(1), false),
    ] {
        let mut doc = load("");
        for ch in source.chars() {
            doc = type_text(doc, &ch.to_string());
        }
        assert_eq!(doc.text.as_ref(), "task", "{source}");
        assert_eq!(doc.blocks[0].list.unwrap().number, number);
        assert_eq!(doc.blocks[0].list.unwrap().task, Some(checked));
        assert_eq!(load(&doc.wire).text, doc.text);
        let (continued, _) = doc.newline(4..4, MAX_BYTES).unwrap();
        assert_eq!(continued.blocks[1].list.unwrap().task, Some(false));
    }
    let doc = type_text(load(r"- \[ ]"), " ");
    assert_eq!(doc.text.as_ref(), "[ ] ");
    assert_eq!(doc.blocks[0].list.unwrap().task, None);
}

#[test]
fn shortcut_code_block_round_trips_without_adding_or_dropping_editable_newlines() {
    let block = type_text(load(""), "``` ");
    for value in ["", "x", " x  ", "x\n", "x\n\n", "\n"] {
        let doc = type_text(block.clone(), value);
        let restored = load(&doc.wire);
        assert_eq!(restored.text.as_ref(), value, "wire={:?}", doc.wire);
        assert_eq!(restored.blocks, doc.blocks, "wire={:?}", doc.wire);
        assert_eq!(restored.wire, doc.wire);
    }
    let doc = type_text(block, "x");
    let (doc, _) = doc.newline(1..1, MAX_BYTES).unwrap();
    let doc = doc.leave_empty_block(2, MAX_BYTES).unwrap();
    let restored = load(&doc.wire);
    assert_eq!(doc.text.as_ref(), "x\n");
    assert_eq!(restored.text, doc.text);
    assert_eq!(restored.blocks, doc.blocks);
}

#[test]
fn hidden_fence_rows_do_not_consume_logical_block_capacity() {
    for count in [255, 256] {
        let body = vec!["x"; count].join("\n");
        let source = format!("```\n{body}\n```");
        let doc = load(&source);
        assert_eq!(doc.text.as_ref(), body);
        assert_eq!(doc.blocks.len(), count);
        assert_eq!(load(&doc.wire).text, doc.text);
    }
    let body = vec!["x"; 257].join("\n");
    assert!(Document::import(&format!("```\n{body}\n```"), MAX_BYTES).is_none());
}
