#!/usr/bin/env python3
"""Check local Markdown links without network access or third-party packages.

This intentionally supports the repository's documentation conventions, not all
CommonMark extensions. See docs/development.md for coverage and limitations.
"""

from __future__ import annotations

import argparse
import html
from html.parser import HTMLParser
import os
from pathlib import Path
import re
import string
import sys
import unicodedata
from urllib.parse import unquote, urlsplit


MARKDOWN = {".md", ".markdown"}
HTML = {".html", ".htm", ".svg"}
SKIP_DIRS = {".git", ".venv", "venv", "node_modules", "target", "__pycache__"}
RESEARCH = (Path("design/research"), Path("docs/design/research"))
ESCAPE = re.compile(r"\\([" + re.escape(string.punctuation) + r"])")
DEFINITION = re.compile(r"(?m)^ {0,3}\[([^\]\n]+)\]:[ \t]*")
INLINE_CODE = re.compile(r"(?<!`)(`+)(?!`)([\s\S]*?)(?<!`)\1(?!`)")


def blank(text: str) -> str:
    """Mask examples without changing source offsets or line numbers."""
    return "".join("\n" if char == "\n" else " " for char in text)


def prose(text: str) -> str:
    text = re.sub(r"<!--[\s\S]*?-->", lambda match: blank(match[0]), text)
    lines = []
    fence = ""
    for line in text.splitlines(keepends=True):
        marker = re.match(r"^ {0,3}(`{3,}|~{3,})(.*)$", line)
        if fence:
            lines.append(blank(line))
            if (marker and marker[1][0] == fence[0]
                    and len(marker[1]) >= len(fence) and not marker[2].strip()):
                fence = ""
        elif marker:
            fence = marker[1]
            lines.append(blank(line))
        elif line.startswith(("    ", "\t")):
            lines.append(blank(line))
        else:
            lines.append(line)
    return "".join(lines)


def unescape(value: str) -> str:
    return html.unescape(ESCAPE.sub(r"\1", value))


def label(value: str) -> str:
    return " ".join(unescape(value).split()).casefold()


def destination(text: str, start: int) -> tuple[str, int] | None:
    """Read a Markdown destination; leave its title/outer ')' to the caller."""
    pos = start
    while pos < len(text) and text[pos].isspace():
        pos += 1
    begin = pos
    if pos < len(text) and text[pos] == "<":
        pos += 1
        begin = pos
        while pos < len(text) and text[pos] not in ">\n":
            pos += 2 if text[pos] == "\\" else 1
        if pos < len(text) and text[pos] == ">":
            return unescape(text[begin:pos]), pos + 1
        return None
    depth = 0
    while pos < len(text):
        char = text[pos]
        if char == "\\" and pos + 1 < len(text):
            pos += 2
            continue
        if char.isspace() or (char == ")" and depth == 0):
            break
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        pos += 1
    if depth:
        return None
    return unescape(text[begin:pos]), pos


class HTMLReferences(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.anchors: set[str] = set()
        self.links: list[tuple[str, int]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        for name in (("id", "name") if tag == "a" else ("id",)):
            if values.get(name):
                self.anchors.add(values[name])
        attr = "href" if tag == "a" else "src" if tag == "img" else None
        if attr and values.get(attr):
            self.links.append((values[attr], self.getpos()[0]))


def markdown_links(text: str) -> tuple[list[tuple[str, int]], list[tuple[str, int]]]:
    text = INLINE_CODE.sub(lambda match: blank(match[0]), prose(text))
    refs: dict[str, str] = {}
    links: list[tuple[str, int]] = []
    missing: list[tuple[str, int]] = []
    definitions = list(DEFINITION.finditer(text))
    for match in definitions:
        parsed = destination(text, match.end())
        if parsed and not match[1].startswith("^"):
            refs.setdefault(label(match[1]), parsed[0])
            links.append((parsed[0], text.count("\n", 0, match.start()) + 1))
    # Definitions themselves must not be mistaken for shortcut links.
    text = DEFINITION.sub(lambda match: blank(match[0]), text)
    parser = HTMLReferences()
    parser.feed(text)
    links.extend(parser.links)
    stack: list[int] = []
    pos = 0
    while pos < len(text):
        char = text[pos]
        if char == "\\":
            pos += 2
            continue
        if char == "[":
            stack.append(pos)
        elif char == "]" and stack:
            start = stack.pop()
            name = text[start + 1:pos]
            line = text.count("\n", 0, start) + 1
            following = pos + 1
            if text[following:following + 1] == "(":
                parsed = destination(text, following + 1)
                if parsed:
                    value, end = parsed
                    close = re.match(r'''(?:\s+(?:"[^"]*"|'[^']*'|\([^)]*\)))?\s*\)''', text[end:])
                    if close:
                        links.append((value, line))
                        pos = end + close.end()
                        continue
            elif text[following:following + 1] == "[":
                end = text.find("]", following + 1)
                if end != -1 and "\n\n" not in text[following:end]:
                    key = label(text[following + 1:end] or name)
                    if key in refs:
                        links.append((refs[key], line))
                    else:
                        missing.append((key, line))
                    pos = end + 1
                    continue
            elif label(name) in refs:
                links.append((refs[label(name)], line))
        pos += 1
    return links, missing


def heading_slug(title: str) -> str:
    title = re.sub(r"!?\[([^\]]+)\]\([^)]*\)", r"\1", title)
    title = re.sub(r"<[^>]*>", "", title)
    title = re.sub(r"(?<!\w)_{1,3}(.+?)_{1,3}(?!\w)", r"\1", title)
    title = unescape(title).lower().strip()
    # Preserve Unicode letters/marks, underscores and hyphens; remove punctuation.
    title = "".join(char for char in title if char.isalnum() or char in "_ -"
                    or unicodedata.category(char).startswith("M"))
    return title.replace(" ", "-")


def anchors(text: str, markdown: bool) -> set[str]:
    if markdown:
        text = prose(text)
    parser = HTMLReferences()
    parser.feed(INLINE_CODE.sub(lambda match: blank(match[0]), text) if markdown else text)
    result = set(parser.anchors)
    if not markdown:
        return result
    used: set[str] = set()
    previous = ""
    for line in text.splitlines():
        heading = re.match(r"^ {0,3}#{1,6}[ \t]+(.+?)\s*$", line)
        title = None
        if heading:
            title = re.sub(r"[ \t]+#+[ \t]*$", "", heading[1])
        elif (previous.strip() and not previous.lstrip().startswith(("#", ">", "|"))
              and re.fullmatch(r" {0,3}(?:=+|-+)[ \t]*", line)):
            title = previous.strip()
        if title is not None:
            base = heading_slug(title)
            slug = base
            suffix = 0
            while slug in used:
                suffix += 1
                slug = f"{base}-{suffix}"
            used.add(slug)
            result.add(slug)
        previous = line
    return result


def documents(root: Path):
    for directory, dirs, files in os.walk(root, followlinks=False):
        current = Path(directory)
        dirs[:] = sorted(name for name in dirs if name not in SKIP_DIRS
                         and not (current / name).is_symlink()
                         and (current / name).relative_to(root) not in RESEARCH)
        for name in sorted(files):
            path = current / name
            if path.suffix.lower() in MARKDOWN and not path.is_symlink():
                yield path


def main() -> int:
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1],
                      help="documentation root (defaults to this script's repository)")
    root = args.parse_args().root.resolve()
    if not root.is_dir():
        args.error("the selected root must be a directory")
    texts: dict[Path, str] = {}
    anchor_cache: dict[Path, set[str]] = {}
    errors: list[str] = []
    checked = unchecked = count = 0

    def read(path: Path) -> str:
        if path not in texts:
            texts[path] = path.read_text(encoding="utf-8")
        return texts[path]

    def check(source: Path, value: str) -> str:
        nonlocal checked, unchecked
        url = urlsplit(value)
        if url.scheme or url.netloc or value.startswith("//"):
            return ""
        checked += 1
        relative = unquote(url.path)
        if not relative:
            target = source
        else:
            target = root / relative.lstrip("/") if relative.startswith("/") else source.parent / relative
        target = target.resolve()
        if not target.is_relative_to(root):
            return "target escapes the repository root"
        if not target.exists():
            return "missing target"
        fragment = unquote(url.fragment)
        if not fragment:
            return ""
        if target.is_dir():
            indexes = [target / name for name in
                       ("README.md", "readme.md", "README.markdown", "index.md", "index.html")]
            target = next((path.resolve() for path in indexes if path.is_file()), target)
            if not target.is_file():
                return "directory fragment has no README or index"
            if not target.is_relative_to(root):
                return "index escapes the repository root"
        extension = target.suffix.lower()
        if extension in MARKDOWN | HTML:
            if target not in anchor_cache:
                anchor_cache[target] = anchors(read(target), extension in MARKDOWN)
            return "" if fragment in anchor_cache[target] else "missing fragment"
        lines = re.fullmatch(r"L(\d+)(?:-L(\d+))?", fragment)
        if lines:
            first, last = int(lines[1]), int(lines[2] or lines[1])
            return "" if 1 <= first <= last <= len(read(target).splitlines()) else "line range outside target"
        unchecked += 1
        return ""

    for source in documents(root):
        count += 1
        name = source.relative_to(root).as_posix()
        try:
            links, missing = markdown_links(read(source))
        except (OSError, UnicodeError) as error:
            errors.append(f"{name}: cannot read document ({type(error).__name__})")
            continue
        for key, line in missing:
            errors.append(f"{name}:{line}: undefined reference label [{key}]")
        for value, line in links:
            try:
                problem = check(source, value)
            except (OSError, UnicodeError, ValueError, RuntimeError) as error:
                problem = f"cannot inspect target ({type(error).__name__})"
            if problem:
                errors.append(f"{name}:{line}: {problem}: {value!r}")
    if not count:
        errors.append("no Markdown documents found; nothing was checked")
    for error in errors:
        print(error, file=sys.stderr)
    print(f"Documentation: {count} Markdown files, {checked} local references, "
          f"{unchecked} non-document fragments unchecked, {len(errors)} errors.")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
