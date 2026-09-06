from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from check_markdown_links import broken_links, markdown_files  # noqa: E402


class MarkdownLinkTests(unittest.TestCase):
    def test_encoded_target_and_fenced_example(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "with space.md").write_text("# Target\n", encoding="utf-8")
            (root / "a.md").write_text(
                '[ok](<with%20space.md> "title")\n```text\n[x](missing.md)\n```\n',
                encoding="utf-8",
            )
            self.assertEqual(broken_links(root), [])

    def test_wrong_case_and_outside_repository_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            root.mkdir()
            (root / "Guide.md").write_text("# Guide\n", encoding="utf-8")
            (root.parent / "private.md").write_text("private", encoding="utf-8")
            (root / "a.md").write_text(
                "[wrong](guide.md)\n[outside](../private.md)\n", encoding="utf-8"
            )
            findings = broken_links(root)
            self.assertEqual(len(findings), 2)
            self.assertIn("outside_repository", {item["reason"] for item in findings})

    def test_broken_local_link_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "a.md"
            doc.write_text("[gone](missing.md) and [here](a.md)\n", encoding="utf-8")
            findings = broken_links(root)
            self.assertEqual(len(findings), 1)
            self.assertEqual(findings[0]["path"], "a.md")
            self.assertTrue(findings[0]["target"].endswith("missing.md"))

    def test_external_anchors_and_existing_files_are_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a.md").write_text(
                "[ext](https://example.com) [anchor](#x) [ok](b.md)\n",
                encoding="utf-8",
            )
            (root / "b.md").write_text("# B\n", encoding="utf-8")
            self.assertEqual(broken_links(root), [])
            self.assertEqual({p.name for p in markdown_files(root)}, {"a.md", "b.md"})


if __name__ == "__main__":
    unittest.main()
