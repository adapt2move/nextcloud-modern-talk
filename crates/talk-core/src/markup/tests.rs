use super::*;

fn assert_document(source: &str, document: &Document) {
    let text = document.text.as_str();
    assert!(text.len() <= MAX_TEXT_BYTES);
    assert!(document.lines.as_slice().len() <= MAX_LINES);
    assert!(document.spans.as_slice().len() <= MAX_SPANS);
    assert!(document.source_spans.as_slice().len() <= MAX_SPANS);
    assert!(document.links.as_slice().len() <= MAX_LINKS);
    for (value, spans) in [
        (text, document.spans.as_slice()),
        (source, document.source_spans.as_slice()),
    ] {
        let mut end = 0;
        for span in spans {
            assert_eq!(span.range.start, end, "gap/overlap in {value:?}");
            assert!(span.range.start < span.range.end);
            assert!(
                value.get(span.range.clone()).is_some(),
                "invalid range {span:?} in {value:?}"
            );
            end = span.range.end;
        }
        if value.len() <= MAX_SOURCE_BYTES || value == text {
            assert_eq!(end, value.len());
        }
    }
    let mut previous_end = 0;
    for line in document.lines.as_slice() {
        assert!(line.range.start >= previous_end);
        let content = text.get(line.range.clone()).expect("valid line range");
        assert!(!content.contains(['\n', '\r']));
        assert!(line.heading <= 6);
        previous_end = line.range.end;
    }
    previous_end = 0;
    for link in document.links.as_slice() {
        assert!(link.range.start >= previous_end);
        assert!(text.get(link.range.clone()).is_some());
        assert!(!link.range.is_empty());
        assert!(web_destination(link.destination.as_str()).is_some());
        previous_end = link.range.end;
    }
}

fn style_at(spans: &[Span], offset: usize) -> Style {
    spans
        .iter()
        .find(|span| span.range.contains(&offset))
        .expect("style covering byte")
        .style
}

#[test]
fn unicode_nested_emphasis_and_exact_source_offsets() {
    let source = "# Grüße **👩🏽‍💻 *e\u{301}* ~~sí~~**\n> quoted **bold**";
    let unchanged = source.to_owned();
    let document = parse(source);
    assert_eq!(document.text.as_str(), "Grüße 👩🏽‍💻 e\u{301} sí\nquoted bold");
    assert_document(source, &document);
    assert_eq!(source, unchanged);
    let combined = document
        .text
        .as_str()
        .find("e\u{301}")
        .expect("combining text");
    let style = style_at(document.spans.as_slice(), combined);
    assert!(style.bold && style.italic && style.heading == 1);
    let source_offset = source.find("e\u{301}").expect("source combining text");
    let style = style_at(document.source_spans.as_slice(), source_offset);
    assert!(style.bold && style.italic && !style.marker);
    assert!(style_at(document.source_spans.as_slice(), 0).marker);
    assert!(
        style_at(
            document.source_spans.as_slice(),
            source.find("**").expect("delimiter")
        )
        .marker
    );
    assert_eq!(document.lines.as_slice()[0].heading, 1);
    assert!(document.lines.as_slice()[1].quote);
    // Styles only describe the caller's bytes; UTF-16/IME positions stay those
    // of the original source, including delimiters and surrogate pairs.
    let utf16_offset = source
        .get(..source_offset)
        .expect("source prefix")
        .encode_utf16()
        .count();
    assert_eq!(
        utf16_offset,
        unchanged
            .get(..source_offset)
            .expect("unchanged prefix")
            .encode_utf16()
            .count()
    );
}

#[test]
fn escapes_entities_and_unsupported_syntax_remain_readable() {
    let source = "\\*literal\\* &amp; \\_plain\\_\n| a | b |\n|---|---|\n$math$ [[wiki]]";
    let document = parse(source);
    assert_eq!(
        document.text.as_str(),
        "*literal* & _plain_\n| a | b |\n|---|---|\n$math$ [[wiki]]"
    );
    assert_document(source, &document);
    assert!(
        !document
            .spans
            .as_slice()
            .iter()
            .any(|span| span.style.bold || span.style.italic)
    );
    assert!(style_at(document.source_spans.as_slice(), 0).marker);
    assert!(!style_at(document.source_spans.as_slice(), 1).marker);
}

#[test]
fn lists_tasks_quotes_headings_and_code_keep_structure() {
    let source = "## Header\n\n> quoted\n> next\n\n3. first\n   - nested\n4. second\n\n- [x] done\n- [ ] todo\n\n```rust\n  a\t b\n\nhttps://code.test\n```";
    let document = parse(source);
    assert_eq!(
        document.text.as_str(),
        "Header\nquoted\nnext\n\n3. first\n  • nested\n4. second\n• [x] done\n• [ ] todo\n  a\t b\n\nhttps://code.test\n"
    );
    assert_document(source, &document);
    assert!(document.links.as_slice().is_empty());
    assert!(
        document
            .lines
            .as_slice()
            .iter()
            .any(|line| line.code
                && document.text.as_str().get(line.range.clone()) == Some("  a\t b"))
    );
    assert!(
        style_at(
            document.source_spans.as_slice(),
            source.find("```rust").expect("fence")
        )
        .marker
    );
    assert!(
        !style_at(
            document.source_spans.as_slice(),
            source.find("  a\t b").expect("code")
        )
        .marker
    );
    assert!(
        style_at(
            document.source_spans.as_slice(),
            source.find("  a\t b").expect("code")
        )
        .code
    );
}

#[test]
fn inline_code_does_not_make_prose_lines_block_code() {
    let source = "Normal prose `code` more prose";
    let document = parse(source);
    assert_document(source, &document);
    assert_eq!(document.text.as_str(), "Normal prose code more prose");
    assert_eq!(document.lines.as_slice().len(), 1);
    assert!(!document.lines.as_slice()[0].code);
    let inline = document.text.as_str().find("code").expect("inline code");
    assert!(style_at(document.spans.as_slice(), inline).code);
    assert!(!style_at(document.spans.as_slice(), 0).code);
    assert!(
        style_at(
            document.source_spans.as_slice(),
            source.find("code").expect("source inline code")
        )
        .code
    );

    let source = "Before `inline` after\n\n```text\nfenced\n\n  fence tail\n```\n\nMiddle `span` prose\n\n    indented\n    second\n\nAfter `tail` done";
    let document = parse(source);
    assert_document(source, &document);
    let lines: Vec<_> = document
        .lines
        .as_slice()
        .iter()
        .filter_map(|line| {
            let text = document.text.as_str().get(line.range.clone())?;
            (!text.is_empty()).then_some((text, line.code))
        })
        .collect();
    assert_eq!(
        lines,
        [
            ("Before inline after", false),
            ("fenced", true),
            ("  fence tail", true),
            ("Middle span prose", false),
            ("indented", true),
            ("second", true),
            ("After tail done", false),
        ]
    );
    let blank_code = document
        .text
        .as_str()
        .find("\n\n  fence tail")
        .expect("blank code line")
        + 1;
    assert!(
        document
            .lines
            .as_slice()
            .iter()
            .any(|line| line.range == (blank_code..blank_code) && line.code)
    );
    for phrase in ["Before", " after", "Middle", " prose", "After", " done"] {
        let offset = document
            .text
            .as_str()
            .find(phrase)
            .expect("surrounding prose");
        assert!(!style_at(document.spans.as_slice(), offset).code);
        assert!(
            !style_at(
                document.source_spans.as_slice(),
                source.find(phrase).expect("source prose")
            )
            .code
        );
    }
}

#[test]
fn prose_urls_trim_punctuation_and_preserve_balanced_delimiters() {
    let source = "See (https://example.test/a_(b)). www.example.test/path, HTTPS://EXAMPLE.TEST/x!\n`https://code.test`\n```\nwww.code.test\n```";
    let document = parse(source);
    assert_document(source, &document);
    let links = document.links.as_slice();
    assert_eq!(links.len(), 3);
    assert_eq!(
        document.text.as_str().get(links[0].range.clone()),
        Some("https://example.test/a_(b)")
    );
    assert_eq!(links[0].destination.as_str(), "https://example.test/a_(b)");
    assert_eq!(
        links[1].destination.as_str(),
        "https://www.example.test/path"
    );
    assert_eq!(links[2].destination.as_str(), "https://example.test/x");
    assert!(
        style_at(
            document.source_spans.as_slice(),
            source.find("www.example").expect("source URL")
        )
        .link
    );
}

#[test]
fn markdown_autolinks_references_and_nested_labels_are_clickable() {
    let source = "paragraph\n\n[**click** *here*](https://example.test/a \"title\") <https://example.test/b> [ref][r]\n\n[r]: https://example.test/c";
    let document = parse(source);
    assert_document(source, &document);
    assert_eq!(
        document.text.as_str(),
        "paragraph\n\nclick here https://example.test/b ref"
    );
    let links = document.links.as_slice();
    assert_eq!(links.len(), 3);
    assert_eq!(
        document.text.as_str().get(links[0].range.clone()),
        Some("click here")
    );
    assert_eq!(links[2].destination.as_str(), "https://example.test/c");
    let style = style_at(document.spans.as_slice(), links[0].range.start);
    assert!(style.bold && style.link);
    let label = source.find("click").expect("label");
    assert!(!style_at(document.source_spans.as_slice(), label).marker);
    assert!(style_at(document.source_spans.as_slice(), label).link);
    assert!(
        style_at(
            document.source_spans.as_slice(),
            source.find("https://example.test/a").expect("destination")
        )
        .marker
    );
}

#[test]
fn html_and_image_alt_are_visible_without_remote_objects() {
    let source = "<img src=\"https://remote.test/tracker\">\n\nHello <script>alert(1)</script> ![**Alt** 👋](https://remote.test/picture.png) ![](https://remote.test/empty)";
    let document = parse(source);
    assert_document(source, &document);
    assert!(
        document
            .text
            .as_str()
            .contains("<img src=\"https://remote.test/tracker\">")
    );
    assert!(
        document
            .text
            .as_str()
            .contains("<script>alert(1)</script> Alt 👋")
    );
    assert!(
        document
            .text
            .as_str()
            .contains("![](https://remote.test/empty)")
    );
    assert!(document.links.as_slice().is_empty());
}

#[test]
fn unsafe_destinations_have_no_activation_and_unicode_hosts_canonicalize() {
    for raw in [
        "javascript:alert(1)",
        "data:text/html,x",
        "file:///tmp/a",
        "mailto:a@example.test",
        "//example.test",
        "https:example.test",
        "https:///example.test",
        "https://",
        "https://@example.test",
        "https://user@example.test",
        "https://user:password@example.test",
        "https://good.test@evil.test",
        "https://example.test\\@evil.test",
        "https://example.test/\n",
        "https://example.test/\u{202e}evil",
        "https://example.test/\u{2066}evil",
        "https://example.test/%0a",
        "https://example.test/%5C",
        "https://example.test/%E2%80%AE",
        "https://example.test/%ff%E2%81%A6",
        "www.",
        "www./path",
    ] {
        assert!(web_destination(raw).is_none(), "accepted {raw:?}");
    }
    assert_eq!(
        web_destination("https://bücher.example/über")
            .expect("IDN")
            .as_str(),
        "https://xn--bcher-kva.example/%C3%BCber"
    );
    assert_eq!(
        web_destination("www.example.test").expect("www").as_str(),
        "https://www.example.test/"
    );
    assert!(web_destination("https://example.test/a%20b").is_some());
    assert!(web_destination(&"x".repeat(2049)).is_none());
    assert!(
        web_destination(&format!("https://example.test/{}", "ü".repeat(400))).is_none(),
        "canonical expansion is bounded"
    );
    let source = "[https://label.test](javascript:alert(1)) <mailto:a@example.test> [userinfo](https://u:p@example.test)";
    let document = parse(source);
    assert_document(source, &document);
    assert!(document.links.as_slice().is_empty());
}

#[test]
fn line_breaks_and_code_whitespace_are_preserved() {
    for source in [
        "one\ntwo\n\nthree\n",
        "\n\none\n\n",
        "```\na  \n\tb\n```",
        "one  \ntwo\\\nthree",
    ] {
        let document = parse(source);
        assert_document(source, &document);
        assert!(document.text.as_str().contains('\n'));
    }
    assert_eq!(
        parse("one\ntwo\n\nthree\n").text.as_str(),
        "one\ntwo\n\nthree\n"
    );
    assert_eq!(parse("\n\none\n\n").text.as_str(), "\n\none\n\n");
    assert_eq!(parse("```\na  \n\tb\n```").text.as_str(), "a  \n\tb\n");
    assert_eq!(
        parse("one  \ntwo\\\nthree").text.as_str(),
        "one\ntwo\nthree"
    );
    assert_document("", &parse(""));
    assert_document("a\r\nb\rc", &parse("a\r\nb\rc"));
}

#[test]
fn each_resource_budget_falls_back_without_dropping_bounded_source() {
    let overflows = [
        ("links", "https://e.test ".repeat(MAX_LINKS + 1)),
        ("source spans", "*a* ".repeat(150)),
        ("events", "*a* ".repeat(800)),
        ("nesting", format!("{}a", "> ".repeat(MAX_DEPTH + 1))),
        ("lines", "a\n".repeat(MAX_LINES + 1)),
        ("output expansion", format!("    {}", "\t".repeat(3000))),
    ];
    for (budget, source) in overflows {
        assert!(source.len() <= MAX_SOURCE_BYTES, "bounded fixture {budget}");
        let document = parse(&source);
        assert_document(&source, &document);
        assert_eq!(
            document.text.as_str(),
            source,
            "literal fallback for {budget}"
        );
        assert!(
            document.links.as_slice().is_empty(),
            "fallback does not partially activate links"
        );
        assert!(
            document
                .spans
                .as_slice()
                .iter()
                .all(|span| span.style == Style::default())
        );
    }
    let source = "a\n".repeat(MAX_LINES + 1);
    assert!(
        parse(&source).lines.as_slice().is_empty(),
        "renderer uses literal text block"
    );
    let at_link_limit = "https://e.test ".repeat(MAX_LINKS);
    assert_eq!(parse(&at_link_limit).links.as_slice().len(), MAX_LINKS);
}

#[test]
fn oversized_input_clips_on_utf8_boundary_before_parsing_or_styling() {
    let source = format!(
        "{}👩🏽‍💻{}",
        "a".repeat(MAX_SOURCE_BYTES - 1),
        "*".repeat(20_000)
    );
    let document = parse(&source);
    assert_document(&source, &document);
    assert_eq!(
        document.text.as_str(),
        format!("{}…", "a".repeat(MAX_SOURCE_BYTES - 1))
    );
    assert_eq!(
        document.source_spans.as_slice()[0].range.end,
        MAX_SOURCE_BYTES - 1
    );
    assert!(document.links.as_slice().is_empty());
}

#[test]
fn known_malformed_parser_regressions_remain_total_and_bounded() {
    // Upstream #1129, #1112, #1133, #1142, #1108 and #1084. Only task lists
    // and strikethrough are enabled; the others also guard extension isolation.
    for source in [
        "- [o]:u\n    \t",
        "+\t[\0]:d\n\u{b}",
        "<!S\r:",
        "<!n\r%",
        "%\n:\u{301}\n\t:",
        " \u{fffd}  ![[] ]()]]",
        "- [n]:Z\r\n\t\t",
        "[[a|]] [[a|]] [[a|]]",
        "* [ ] ~![=?\\*\x0c\x00\x00  \x0d* [  1=1\x00\x0d<!]:[=?\\\x0d\x0c\n* [ ] \x0d\x0c%    ",
    ] {
        let result = std::panic::catch_unwind(|| parse(source));
        assert!(result.is_ok(), "parser unwind escaped for {source:?}");
        assert_document(source, &result.expect("caught result"));
    }
}

#[test]
fn deterministic_malformed_unicode_probe_preserves_valid_ranges() {
    let alphabet = [
        '*', '_', '[', ']', '(', ')', '`', '\\', '<', '>', '\n', '\r', '\t', '\0', 'é', '👋',
        '\u{301}', ' ', ':', 'a',
    ];
    let mut state = 0x8712_a95d_u64;
    for length in 0..256 {
        let mut source = String::new();
        for _ in 0..length {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            source.push(alphabet[(state as usize) % alphabet.len()]);
        }
        assert_document(&source, &parse(&source));
    }
}
