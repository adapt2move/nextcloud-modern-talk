//! Bounded Markdown subset for the editable document. Unsupported constructs
//! remain literal. No HTML, image, URL loading or platform work occurs here.
use super::*;

const MAX_DEPTH: usize = 16;
const MAX_PARSE_STEPS: usize = 65_536;

pub(super) fn import(source: &str, limit: usize) -> Option<Document> {
    if source.len() > limit || source.contains(['\0', '\r']) {
        return None;
    }
    // Wire rows include hidden fences and cannot consume the logical block
    // budget. This temporary index has at most MAX_BYTES + 1 entries; admit
    // visible rows against MAX_BLOCKS in append_row after syntax is removed.
    let rows: Vec<_> = source.split('\n').collect();
    let mut result = Document {
        text: "".into(),
        marks: Vec::new(),
        blocks: Vec::new(),
        links: Vec::new(),
        codes: Vec::new(),
        wire: "".into(),
        spans: Vec::new(),
        decorations: Vec::new(),
        typing: None,
    };
    let mut text = String::new();
    let mut i = 0;
    while i < rows.len() {
        let (block, body) = prefix(rows[i]);
        if block == Block::default() && (rows[i].starts_with("    ") || rows[i].starts_with('\t')) {
            if result.codes.len() == MAX_CODES {
                return None;
            }
            let code = Block {
                code: Some(result.codes.len() as u8),
                ..Block::default()
            };
            result.codes.push("".into());
            while i < rows.len() {
                let body = rows[i]
                    .strip_prefix("    ")
                    .or_else(|| rows[i].strip_prefix('\t'));
                let body = if let Some(body) = body {
                    body
                } else if rows[i].trim().is_empty() && i + 1 < rows.len() {
                    ""
                } else {
                    break;
                };
                append_row(
                    &mut result,
                    &mut text,
                    body,
                    &vec![Inline::default(); body.len()],
                    code,
                )?;
                i += 1;
            }
            continue;
        }
        if block == Block::default()
            && let Some((fence, count, language)) = fence(body)
        {
            if result.codes.len() == MAX_CODES || language.len() > 128 {
                return None;
            }
            let id = result.codes.len() as u8;
            result.codes.push(language.to_owned().into());
            let mut close = i + 1;
            while close < rows.len() && !closing(rows[close], fence, count) {
                close += 1;
            }
            let code = Block {
                code: Some(id),
                ..Block::default()
            };
            if close == i + 1 {
                append_row(&mut result, &mut text, "", &[], code)?;
            } else {
                for line in &rows[i + 1..close] {
                    append_row(
                        &mut result,
                        &mut text,
                        line,
                        &vec![Inline::default(); line.len()],
                        code,
                    )?;
                }
                // split rows exclude the LF belonging to the closing fence.
                // Only explicit extra empty code rows become editable newlines.
            }
            i = close.saturating_add(1);
            continue;
        }
        let parsed = inline(
            body,
            &vec![Inline::default(); body.len()],
            &mut result.links,
            false,
        )?;
        append_row(&mut result, &mut text, &parsed.text, &parsed.marks, block)?;
        i += 1;
    }
    if result.blocks.is_empty() {
        result.blocks.push(Block::default());
    }
    result.text = text.into();
    result.finish(limit)?;
    Some(result)
}

fn append_row(
    doc: &mut Document,
    text: &mut String,
    row: &str,
    marks: &[Inline],
    block: Block,
) -> Option<()> {
    if doc.blocks.len() == MAX_BLOCKS
        || text.len() + row.len() + usize::from(!doc.blocks.is_empty()) > MAX_BYTES
    {
        return None;
    }
    if !doc.blocks.is_empty() {
        text.push('\n');
        doc.marks.push(Inline::default());
    }
    text.push_str(row);
    doc.marks.extend_from_slice(marks);
    doc.blocks.push(block);
    Some(())
}

fn prefix(mut source: &str) -> (Block, &str) {
    let mut block = Block::default();
    while block.quote < 8 {
        let Some(rest) = source
            .strip_prefix("> ")
            .or_else(|| (source == ">").then_some(""))
        else {
            break;
        };
        block.quote += 1;
        source = rest;
    }
    let spaces = source.bytes().take_while(|b| *b == b' ').count();
    if spaces <= 16 {
        let body = &source[spaces..];
        let bullet = ["- ", "+ ", "* "].iter().find_map(|p| body.strip_prefix(p));
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        let ordered = (1..=9)
            .contains(&digits)
            .then(|| {
                let number = body[..digits].parse::<u32>().ok()?;
                let rest = body[digits..]
                    .strip_prefix(". ")
                    .or_else(|| body[digits..].strip_prefix(") "))?;
                Some((number, rest))
            })
            .flatten();
        if let Some(rest) = bullet.or_else(|| ordered.map(|(_, rest)| rest)) {
            let (task, rest) = if let Some(rest) = rest.strip_prefix("[ ] ") {
                (Some(false), rest)
            } else if let Some(rest) = rest
                .strip_prefix("[x] ")
                .or_else(|| rest.strip_prefix("[X] "))
            {
                (Some(true), rest)
            } else {
                (None, rest)
            };
            block.list = Some(List {
                depth: (spaces / 2) as u8,
                number: ordered.map(|(n, _)| n),
                task,
            });
            source = rest;
        }
    }
    if block.list.is_none() {
        let hashes = source.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && source.as_bytes().get(hashes) == Some(&b' ') {
            block.heading = hashes as u8;
            source = &source[hashes + 1..];
        }
    }
    (block, source)
}

fn fence(source: &str) -> Option<(u8, usize, &str)> {
    let first = *source.as_bytes().first()?;
    if !matches!(first, b'`' | b'~') {
        return None;
    }
    let count = source.bytes().take_while(|b| *b == first).count();
    let language = source[count..].trim();
    if count < 3 || language.contains('`') {
        return None;
    }
    Some((first, count, language))
}
fn closing(source: &str, byte: u8, count: usize) -> bool {
    let source = source.trim_start_matches(' ');
    let run = source.bytes().take_while(|b| *b == byte).count();
    run >= count && source[run..].trim().is_empty()
}

pub(super) fn shortcut(source: &str, codes: &mut Vec<SharedString>) -> Option<Block> {
    let (block, body) = prefix(source);
    if body.is_empty() && block != Block::default() {
        return Some(block);
    }
    if let Some((_, _, language)) = fence(source.trim_end()) {
        if codes.len() == MAX_CODES || language.len() > 128 {
            return None;
        }
        let code = codes.len() as u8;
        codes.push(language.to_owned().into());
        return Some(Block {
            code: Some(code),
            ..Block::default()
        });
    }
    None
}

pub(super) struct Parsed {
    pub text: String,
    pub marks: Vec<Inline>,
    pub map: Vec<usize>,
}
struct Parser<'a> {
    source: &'a str,
    marks: &'a [Inline],
    links: &'a mut Vec<SharedString>,
    output: Parsed,
    steps: usize,
    emphasis: Vec<Style>,
    decode_entities: bool,
}

pub(super) fn inline(
    source: &str,
    marks: &[Inline],
    links: &mut Vec<SharedString>,
    wait_for_complete_run: bool,
) -> Option<Parsed> {
    if source.len() > MAX_BYTES || marks.len() != source.len() {
        return None;
    }
    // Reuse the pinned CommonMark owner's delimiter decisions, rather than
    // inventing competing rules for triple runs, intraword emphasis or nesting.
    // Existing semantic code/escaped punctuation is masked byte-for-byte so it
    // cannot supply delimiters for a new conversion elsewhere in the line.
    let mut masked = source.as_bytes().to_vec();
    for (i, mark) in marks.iter().enumerate() {
        if mark.flags & (CODE | LITERAL) != 0 {
            masked[i] = b'x';
        }
    }
    let masked = String::from_utf8(masked).ok()?;
    let parsed = talk_core::ui::markup::parse(&masked);
    let mut emphasis = vec![Style::default(); source.len()];
    for span in parsed.source_spans.as_slice() {
        if let Some(target) = emphasis.get_mut(span.range.clone()) {
            target.fill(span.style);
        }
    }
    // While typing `**bold*`, CommonMark accepts a literal star plus
    // emphasis. Wait for the whole opening delimiter run to be balanced before
    // consuming any of it; otherwise the second closing star has no partner.
    let mut at = 0;
    let mut partial = false;
    while at < source.len() {
        let byte = source.as_bytes()[at];
        let run = source.as_bytes()[at..]
            .iter()
            .take_while(|b| **b == byte)
            .count();
        if matches!(byte, b'*' | b'_') && run > 1 {
            let first = emphasis[at];
            partial |= emphasis[at..at + run]
                .iter()
                .any(|style| style.marker != first.marker);
        }
        at += run;
    }
    if wait_for_complete_run && partial {
        for style in &mut emphasis {
            style.bold = false;
            style.italic = false;
        }
    }
    let mut parser = Parser {
        source,
        marks,
        links,
        steps: MAX_PARSE_STEPS,
        emphasis,
        decode_entities: !wait_for_complete_run,
        output: Parsed {
            text: String::new(),
            marks: Vec::new(),
            map: vec![0; source.len() + 1],
        },
    };
    parser.parse(0..source.len(), Inline::default(), 0)?;
    Some(parser.output)
}

impl Parser<'_> {
    fn spend(&mut self, amount: usize) -> Option<()> {
        self.steps = self.steps.checked_sub(amount)?;
        Some(())
    }
    fn remove(&mut self, range: Range<usize>) {
        self.output.map[range.start..=range.end].fill(self.output.text.len());
    }
    fn emit(&mut self, range: Range<usize>, extra: Inline) {
        for (relative, ch) in self.source[range.clone()].char_indices() {
            let offset = range.start + relative;
            let mut mark = self.marks[offset];
            mark.flags |= extra.flags;
            if extra.link > 0 {
                mark.link = extra.link;
            }
            self.output.map[offset..offset + ch.len_utf8()].fill(self.output.text.len());
            self.output.text.push(ch);
            self.output
                .marks
                .extend(std::iter::repeat_n(mark, ch.len_utf8()));
            self.output.map[offset + ch.len_utf8()] = self.output.text.len();
        }
    }
    fn decoded(&mut self, range: Range<usize>, text: &str, mark: Inline) {
        self.remove(range.clone());
        self.output.text.push_str(text);
        self.output.marks.extend(std::iter::repeat_n(
            Inline {
                flags: mark.flags | LITERAL,
                ..mark
            },
            text.len(),
        ));
        self.output.map[range.end] = self.output.text.len();
    }
    fn literal(&self, offset: usize) -> bool {
        self.marks[offset].flags & (CODE | LITERAL) != 0
    }
    fn parse(&mut self, range: Range<usize>, extra: Inline, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut i = range.start;
        while i < range.end {
            self.spend(1)?;
            let rest = &self.source[i..range.end];
            let ch = rest.chars().next()?;
            let style = self.emphasis[i];
            let flags = (u8::from(style.bold) * BOLD)
                | (u8::from(style.italic) * ITALIC)
                | (u8::from(style.strike) * STRIKE);
            let extra = Inline {
                flags: extra.flags | flags,
                ..extra
            };
            if !self.literal(i) && style.marker && flags != 0 && matches!(ch, '*' | '_' | '~') {
                self.remove(i..i + ch.len_utf8());
                i += ch.len_utf8();
                continue;
            }
            if self.literal(i) {
                self.emit(i..i + ch.len_utf8(), extra);
                i += ch.len_utf8();
                continue;
            }
            if self.decode_entities
                && ch == '&'
                && let Some(end) = rest.find(';').filter(|end| *end <= 32)
            {
                let entity = &rest[..=end];
                self.spend(entity.len())?;
                let decoded = talk_core::ui::markup::parse(entity);
                if !decoded.text.as_str().is_empty() && decoded.text.as_str() != entity {
                    self.decoded(i..i + end + 1, decoded.text.as_str(), extra);
                    i += end + 1;
                    continue;
                }
            }
            if ch == '\\'
                && let Some(next) = rest.chars().nth(1)
                && next.is_ascii_punctuation()
            {
                self.remove(i..i + 1);
                self.emit(
                    i + 1..i + 1 + next.len_utf8(),
                    Inline {
                        flags: extra.flags | LITERAL,
                        ..extra
                    },
                );
                i += 1 + next.len_utf8();
                continue;
            }
            if ch == '`' {
                let count = rest.bytes().take_while(|b| *b == b'`').count();
                self.spend(rest.len())?;
                let mut search = i + count;
                let mut close = None;
                while search < range.end {
                    let Some(offset) = self.source[search..range.end].find('`') else {
                        break;
                    };
                    let at = search + offset;
                    let run = self.source[at..range.end]
                        .bytes()
                        .take_while(|b| *b == b'`')
                        .count();
                    if run == count && !self.literal(at) {
                        close = Some(at);
                        break;
                    }
                    search = at + run;
                }
                if let Some(close) = close {
                    let mut body = i + count..close;
                    let code = &self.source[body.clone()];
                    // CommonMark's delimiter padding is not code content.
                    if code.starts_with(' ')
                        && code.ends_with(' ')
                        && code.bytes().any(|b| b != b' ')
                    {
                        body.start += 1;
                        body.end -= 1;
                    }
                    self.remove(i..body.start);
                    self.emit(
                        body.clone(),
                        Inline {
                            flags: extra.flags | CODE,
                            ..extra
                        },
                    );
                    self.remove(body.end..close + count);
                    i = close + count;
                    continue;
                }
                self.emit(i..i + count, extra);
                i += count;
                continue;
            }
            if ch == '['
                && i.checked_sub(1).and_then(|j| self.source.as_bytes().get(j)) != Some(&b'!')
            {
                self.spend(rest.len())?;
                if let Some(label_end) = rest.find("](") {
                    let label_end = i + label_end;
                    let dest_start = label_end + 2;
                    let mut end = dest_start;
                    let mut nesting = 0usize;
                    let mut escaped = false;
                    while end < range.end {
                        let c = self.source.as_bytes()[end];
                        if !escaped && c == b')' && nesting == 0 {
                            break;
                        }
                        if !escaped && c == b'(' {
                            nesting += 1;
                        }
                        if !escaped && c == b')' {
                            nesting = nesting.saturating_sub(1);
                        }
                        escaped = !escaped && c == b'\\';
                        end += 1;
                    }
                    if end < range.end && end - dest_start <= 2048 && label_end > i + 1 {
                        let destination = unescape(&self.source[dest_start..end]);
                        if !destination.chars().any(char::is_whitespace) && !destination.is_empty()
                        {
                            let link = if let Some(index) =
                                self.links.iter().position(|d| d.as_ref() == destination)
                            {
                                index
                            } else {
                                if self.links.len() == MAX_LINKS {
                                    return None;
                                }
                                self.links.push(destination.into());
                                self.links.len() - 1
                            };
                            self.remove(i..i + 1);
                            self.parse(
                                i + 1..label_end,
                                Inline {
                                    link: link as u8 + 1,
                                    ..extra
                                },
                                depth + 1,
                            )?;
                            self.remove(label_end..end + 1);
                            i = end + 1;
                            continue;
                        }
                    }
                }
            }
            // HTML/image syntax stays visible and inert, including punctuation
            // inside an HTML tag. No images are fetched or embedded.
            if ch == '<'
                && let Some(end) = rest.find('>')
            {
                self.emit(
                    i..i + end + 1,
                    Inline {
                        flags: extra.flags | LITERAL,
                        ..extra
                    },
                );
                i += end + 1;
                continue;
            }
            self.emit(i..i + ch.len_utf8(), extra);
            i += ch.len_utf8();
        }
        Some(())
    }
}
fn unescape(source: &str) -> String {
    let mut result = String::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            if let Some(ch) = chars.next() {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

struct Writer {
    value: String,
    limit: usize,
}
impl Writer {
    fn push(&mut self, value: &str) -> Option<()> {
        if value.len() > self.limit.saturating_sub(self.value.len()) {
            return None;
        }
        self.value.push_str(value);
        Some(())
    }
    fn repeated(&mut self, ch: &str, count: usize) -> Option<()> {
        for _ in 0..count {
            self.push(ch)?;
        }
        Some(())
    }
}

pub(super) fn export(doc: &Document, limit: usize) -> Option<String> {
    let rows: Vec<_> = doc.text.split('\n').collect();
    let mut writer = Writer {
        value: String::new(),
        limit,
    };
    let mut row = 0;
    let mut offset = 0;
    while row < rows.len() {
        if let Some(code) = doc.blocks[row].code {
            let language = doc.codes.get(usize::from(code))?;
            let first = row;
            while row < rows.len() && doc.blocks[row].code == Some(code) {
                row += 1;
            }
            let length = rows[first..row].iter().map(|s| s.len()).sum::<usize>() + row - first - 1;
            let end = offset + length;
            let followed = row < rows.len();
            let body = doc.text.get(offset..end)?;
            let fence = longest_backticks(body).max(2) + 1;
            writer.repeated("`", fence)?;
            writer.push(language)?;
            writer.push("\n")?;
            writer.push(body)?;
            if !body.is_empty() {
                // The closing fence needs a structural LF distinct from any
                // editable trailing LF already present in the code body.
                writer.push("\n")?;
            }
            writer.repeated("`", fence)?;
            offset = end + usize::from(followed);
            if followed {
                writer.push("\n")?;
            }
            continue;
        }
        let block = doc.blocks[row];
        writer.repeated("> ", usize::from(block.quote))?;
        if let Some(list) = block.list {
            writer.repeated("  ", usize::from(list.depth))?;
            if let Some(number) = list.number {
                writer.push(&format!("{number}. "))?;
            } else {
                writer.push("- ")?;
            }
            if let Some(checked) = list.task {
                writer.push(if checked { "[x] " } else { "[ ] " })?;
            }
        }
        if block.heading > 0 {
            writer.repeated("#", usize::from(block.heading))?;
            writer.push(" ")?;
        }
        let inline_start = writer.value.len();
        let range = offset..offset + rows[row].len();
        export_inline(doc, range.clone(), &mut writer, false)?;
        if !matches_inline(doc, range.clone(), &writer.value[inline_start..]) {
            // CommonMark flanking rules use source punctuation. Encoding text
            // as numeric entities permits punctuation/emoji at arbitrary rich
            // mark boundaries without adding spaces or invisible Unicode to
            // user content. Use it only when ordinary delimiters are ambiguous.
            writer.value.truncate(inline_start);
            export_inline(doc, range.clone(), &mut writer, true)?;
            if !matches_inline(doc, range, &writer.value[inline_start..]) {
                return None;
            }
        }
        offset += rows[row].len();
        row += 1;
        if row < rows.len() {
            writer.push("\n")?;
            offset += 1;
        }
    }
    Some(writer.value)
}

fn longest_backticks(source: &str) -> usize {
    source.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}
fn tokens(mark: Inline) -> Vec<&'static str> {
    let mut tokens = Vec::with_capacity(3);
    if mark.flags & BOLD != 0 {
        tokens.push("**");
    }
    if mark.flags & ITALIC != 0 {
        tokens.push("*");
    }
    if mark.flags & STRIKE != 0 {
        tokens.push("~~");
    }
    tokens
}
fn matches_inline(doc: &Document, range: Range<usize>, wire: &str) -> bool {
    let rendered = talk_core::ui::markup::parse(wire);
    let visible = &doc.text[range.clone()];
    if rendered.text.as_str().trim_end_matches([' ', '\t']) != visible.trim_end_matches([' ', '\t'])
    {
        return false;
    }
    let mut span = 0;
    for (offset, _) in visible.char_indices().filter(|(_, ch)| !ch.is_whitespace()) {
        while rendered
            .spans
            .as_slice()
            .get(span)
            .is_some_and(|s| s.range.end <= offset)
        {
            span += 1;
        }
        let Some(style) = rendered.spans.as_slice().get(span).map(|s| s.style) else {
            return false;
        };
        let flags = doc.marks[range.start + offset].flags;
        if style.bold != (flags & BOLD != 0)
            || style.italic != (flags & ITALIC != 0)
            || style.strike != (flags & STRIKE != 0)
            || style.code != (flags & CODE != 0)
        {
            return false;
        }
    }
    true
}

fn export_inline(
    doc: &Document,
    range: Range<usize>,
    out: &mut Writer,
    encode: bool,
) -> Option<()> {
    let mut i = range.start;
    let mut active: Vec<&'static str> = Vec::new();
    let mut link = 0;
    while i < range.end {
        let ch = doc.text[i..range.end].chars().next()?;
        let mut mark = doc.marks[i];
        let mut end = i + ch.len_utf8();
        if mark.flags & CODE != 0 {
            while end < range.end && doc.marks[end] == mark {
                end += doc.text[end..range.end].chars().next()?.len_utf8();
            }
        }
        // Whitespace only keeps a mark shared by both neighboring content
        // scalars. This keeps delimiters outside leading/trailing spaces.
        if ch.is_whitespace() && mark.flags & CODE == 0 {
            let previous = doc.text[range.start..i]
                .char_indices()
                .rev()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(o, _)| range.start + o);
            let next = doc.text[end..range.end]
                .char_indices()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(o, _)| end + o);
            let shared = previous
                .zip(next)
                .map_or(0, |(a, b)| doc.marks[a].flags & doc.marks[b].flags);
            mark.flags &= !7 | shared;
        }
        let desired = tokens(mark);
        let same = if mark.link == link {
            active
                .iter()
                .zip(&desired)
                .take_while(|(a, b)| a == b)
                .count()
        } else {
            0
        };
        for token in active[same..].iter().rev() {
            out.push(token)?;
        }
        if mark.link != link {
            if link > 0 {
                close_link(doc, link, out)?;
            }
            if mark.link > 0 {
                out.push("[")?;
            }
            link = mark.link;
        }
        for token in &desired[same..] {
            out.push(token)?;
        }
        active = desired;
        if mark.flags & CODE != 0 {
            let code = &doc.text[i..end];
            let fence = longest_backticks(code) + 1;
            let padding = code.starts_with('`')
                || code.ends_with('`')
                || (code.starts_with(' ')
                    && code.ends_with(' ')
                    && code.bytes().any(|b| b != b' '));
            out.repeated("`", fence)?;
            if padding {
                out.push(" ")?;
            }
            out.push(code)?;
            if padding {
                out.push(" ")?;
            }
            out.repeated("`", fence)?;
        } else {
            // Escape syntax represented literally in the semantic model. This
            // prevents typed code/literal punctuation turning into new syntax
            // when a draft is restored or the server parses canonical Markdown.
            if encode {
                out.push(&format!("&#{};", ch as u32))?;
            } else {
                let line_start = doc.text[range.start..i]
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ' ');
                if "\\`*_~[]<>!&#".contains(ch)
                    || (line_start && matches!(ch, '-' | '+' | '=' | '.' | ')'))
                {
                    out.push("\\")?;
                }
                out.push(&doc.text[i..end])?;
            }
        }
        i = end;
    }
    for token in active.iter().rev() {
        out.push(token)?;
    }
    if link > 0 {
        close_link(doc, link, out)?;
    }
    Some(())
}
fn close_link(doc: &Document, link: u8, out: &mut Writer) -> Option<()> {
    let destination = doc.links.get(usize::from(link) - 1)?;
    out.push("](")?;
    for ch in destination.chars() {
        if matches!(ch, '(' | ')' | '\\') {
            out.push("\\")?;
        }
        let mut buffer = [0; 4];
        out.push(ch.encode_utf8(&mut buffer))?;
    }
    out.push(")")
}

pub(super) fn compact(doc: &mut Document) {
    let mut links = Vec::new();
    let mut link_map = [0u8; MAX_LINKS + 1];
    for mark in &mut doc.marks {
        if mark.link == 0 {
            continue;
        }
        let slot = &mut link_map[usize::from(mark.link)];
        if *slot == 0 {
            links.push(doc.links[usize::from(mark.link) - 1].clone());
            *slot = links.len() as u8;
        }
        mark.link = *slot;
    }
    if let Some(typing) = &mut doc.typing {
        typing.link = link_map[usize::from(typing.link)];
    }
    doc.links = links;
    let mut codes = Vec::new();
    let mut code_map = [None; MAX_CODES];
    for block in &mut doc.blocks {
        if let Some(code) = &mut block.code {
            let slot = &mut code_map[usize::from(*code)];
            let replacement = *slot.get_or_insert_with(|| {
                codes.push(doc.codes[usize::from(*code)].clone());
                (codes.len() - 1) as u8
            });
            *code = replacement;
        }
    }
    doc.codes = codes;
}
