#!/usr/bin/env python3
"""Pure Python import/string fixtures; never inspect or execute application code.

Use --checker PATH to exercise an isolated checker or its unchanged preimage.
Load declarations and the actual core/native guard loops via AST so the checker's
top-level whole-workspace check cannot run against this active checkout.
"""
import argparse
import ast
from pathlib import Path
import tempfile
import unittest


def load_checker(path):
    tree = ast.parse(path.read_text(), filename=str(path))
    declarations = [node for node in tree.body if isinstance(
        node, (ast.Import, ast.ImportFrom, ast.Assign, ast.AnnAssign, ast.FunctionDef))]
    namespace = {'__file__': str(path)}
    exec(compile(ast.Module(declarations, type_ignores=[]), str(path), 'exec'), namespace)
    check = next(node for node in declarations if isinstance(node, ast.FunctionDef)
                 and node.name == 'check')
    # Reuse production rules unchanged, only on synthetic Rust files in a temp root.
    selected = []
    for node in check.body:
        if (isinstance(node, ast.For) and isinstance(node.target, ast.Name)
                and node.target.id == 'path'
                and ('crates/nextcloud-talk-api/src' in ast.unparse(node.iter)
                     or 'crates/talk-core/src' in ast.unparse(node.iter)
                     or 'native' in ast.unparse(node.iter))):
            selected.append(node)
        elif (isinstance(node, ast.Assign) and any(isinstance(target, ast.Name)
              and target.id in {'native', 'ui_files'} for target in node.targets)):
            selected.append(node)
        elif (isinstance(node, ast.For) and isinstance(node.iter, ast.Name)
              and node.iter.id == 'ui_files'):
            selected.append(node)
    namespace['fixture_rules'] = compile(ast.Module(selected, type_ignores=[]), str(path), 'exec')
    return namespace


class ImportStrings(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.guard = load_checker(CHECKER)

    def setUp(self):
        self.guard['failures'].clear()

    def native(self, source, core='', core_ui=''):
        with tempfile.TemporaryDirectory(prefix='architecture-strings-') as directory:
            root = Path(directory)
            native = root / 'apps/desktop/src/presentation'
            native.mkdir(parents=True)
            (native / 'fixture.rs').write_text(source)
            core_dir = root / 'crates/talk-core/src'
            core_dir.mkdir(parents=True)
            (core_dir / 'fixture.rs').write_text(core)
            (core_dir / 'ui').mkdir()
            (core_dir / 'ui/presentation.rs').write_text(core_ui)
            self.guard['ROOT'] = root
            self.guard['failures'].clear()
            exec(self.guard['fixture_rules'], self.guard)
            return list(self.guard['failures'])

    def test_api_is_independent(self):
        with tempfile.TemporaryDirectory(prefix='api-boundary-') as directory:
            root = Path(directory)
            api = root / 'crates/nextcloud-talk-api/src'
            api.mkdir(parents=True)
            self.guard['ROOT'] = root
            for source, forbidden in [('use reqwest::Client;', False), ('use talk_core::policy::Decision;', True), ('use talk_adapters::media::wire;', True), ('use gpui::Window;', True), ('use rusqlite::Connection;', True)]:
                (api/'lib.rs').write_text(source)
                self.guard['failures'].clear()
                exec(self.guard['fixture_rules'], self.guard)
                failures = [f for f in self.guard['failures'] if 'API contains' in f]
                self.assertEqual(bool(failures), forbidden)

    def test_reported_settings_sentence(self):
        source = 'use gpui::div;\nconst COPY: &str = "Both will use GitHub Releases in the future.";'
        self.assertEqual(self.native(source), [])

    def test_permitted_nested_grouped_imports(self):
        self.assertEqual(self.native('use gpui::{div, prelude::*};\n'
                                     'use talk_core::{ui::{Text, Page}};\n'
                                     'use std::{fmt::{Debug, Display}, cmp::Ordering};'), [])
        self.guard['checked_imports']('use talk_core::{ui::{Text, Page}};', False)
        self.assertEqual(self.guard['failures'], [])
        self.assertEqual(self.native('', core_ui='use super::{Text, Page};\n'
                                     'const S: &str = "Both use releases;";'), [])

    def test_literal_forms_do_not_hide_forbidden_imports(self):
        literals = [
            '"Both use releases; später nutzen; después usar"',
            r'"say \"use releases;\" then \\ use again;"',
            'r##"use releases; " // still literal; use more;"##',
            'b"use releases;"', 'br#"use releases; " more"#',
            'c"use releases;"', 'cr##"use releases; " more"##',
        ]
        for literal in literals:
            for forbidden in ('use talk_adapters::Worker;', 'use ::std::{fmt::Debug, fs::{File}};',
                              'use talk_core::{ui::Text, protocol::Wire};'):
                with self.subTest(literal=literal, forbidden=forbidden):
                    prefix = f'const COPY: &str = {literal};\n'
                    self.assertEqual(self.native(prefix + 'use gpui::div;'), [])
                    self.assertIn('native presentation import must use core UI, GPUI or local view types',
                                  self.native(prefix + forbidden))

    def test_comment_character_and_lifetime_adjacency(self):
        prefixes = ['// " quote\n', '/* " quote */ ', "let quote = '\"'; ",
                    r"let quote = '\''; ", r"let quote = b'\x22'; ",
                    r"let quote = '\u{22}'; ", "fn f<'a>(x: &'a str) {} ",
                    "let quote = '\"'; // \"\n", "/* ' */ let q = '\"'; "]
        for prefix in prefixes:
            with self.subTest(prefix=prefix):
                self.assertIn('native presentation import must use core UI, GPUI or local view types',
                              self.native(prefix + 'use talk_adapters::Worker;'))

    def test_import_tokens_only_see_code(self):
        source = 'use talk_core::ui::Text;\nconst S: &str = "use strings; extern crate secret;";'
        remainder = self.guard['checked_imports'](source, False)
        self.assertEqual(self.guard['failures'], [])
        self.assertEqual(remainder.index('const S'), source.index('const S'))
        self.assertEqual(remainder.splitlines()[1], source.splitlines()[1])
        for code in ('use talk_core::ui::*;', 'extern crate forbidden;',
                     'use talk_core::ui::Text as Alias;'):
            with self.subTest(code=code):
                self.guard['failures'].clear()
                self.guard['checked_imports'](source + code, False)
                self.assertTrue(self.guard['failures'])

    def test_content_rules_retain_literals_and_offsets(self):
        for literal in ('"use https://example.invalid/ocs/;"',
                        'r#"use https://example.invalid/ocs/; " // literal"#'):
            with self.subTest(literal=literal):
                source = f'use talk_core::ui::Text;\nconst S: &str = {literal};'
                uncommented = self.guard['without_comments'](source)
                remainder = self.guard['checked_imports'](uncommented, False)
                self.assertEqual(len(remainder), len(source))
                self.assertEqual(remainder[source.index(literal):], source[source.index(literal):])
                failures = self.native(source, core=source)
                self.assertIn('native presentation contains worker/secret/network/file/media I/O or route', failures)
                self.assertIn('crates/talk-core/src/fixture.rs: raw protocol route leaked into core', failures)
        self.assertIn('native presentation contains worker/secret/network/file/media I/O or route',
                      self.native('const S: &str = "use releases; reqwest";'))
        self.assertIn('native presentation contains worker/secret/network/file/media I/O or route',
                      self.native('let file = ::std::fs::File;'))
        self.assertIn('renderer contract contains secret/transport-bearing identifiers',
                      self.native('', core_ui='const S: &str = "use releases; password";'))
        malformed = 'use talk_core::ui::Text "https://example.invalid/";'
        self.guard['failures'].clear()
        self.assertEqual(self.guard['checked_imports'](malformed, False), malformed)
        self.assertTrue(self.guard['failures'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--checker', type=Path, default=Path(__file__).with_name('check_architecture.py'))
    options, unittest_args = parser.parse_known_args()
    CHECKER = options.checker.resolve()
    unittest.main(argv=[__file__, *unittest_args])
