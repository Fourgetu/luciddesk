"""Regression checks for version-specific bilingual Release notes."""
import importlib.util
from pathlib import Path
import re
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("release_notes", ROOT / "tools/release-notes.py")
notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notes)


class ReleaseNotesTests(unittest.TestCase):
    def test_both_languages_and_tagged_links(self):
        zh = "## 1.2.3 · 2026-09-30\n\n- 中文 [指南](docs/usage.md)\n\n## 1.2.2\n\nOLD"
        en = "## 1.2.3 · 2026-09-30\n\n- English [Guide](docs/usage.md) [Web](https://example.com)\n"
        result = notes.bilingual(zh, en, "v1.2.3", "owner/repo")
        self.assertLess(result.index("## 简体中文"), result.index("## English"))
        self.assertIn("### 1.2.3 · 2026-09-30", result)
        self.assertEqual(result.count("https://github.com/owner/repo/blob/v1.2.3/docs/usage.md"), 2)
        self.assertIn("https://example.com", result)
        self.assertNotIn("OLD", result)

    def test_either_language_must_have_one_nonempty_section(self):
        valid = "## 1.2.3\n\n- Change\n"
        for invalid in ["## 1.2.2\n\n- Older", "## 1.2.3\n", valid + valid]:
            for zh, en in [(valid, invalid), (invalid, valid)]:
                with self.subTest(zh=zh, en=en), self.assertRaises(ValueError):
                    notes.bilingual(zh, en, "v1.2.3", "owner/repo")

    def test_invalid_tag(self):
        with self.assertRaises(ValueError):
            notes.extract("## 1.2.3\n\n- Change", "main", "owner/repo")

    def test_repository_versions_and_history_match(self):
        zh = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8-sig")
        en = (ROOT / "CHANGELOG.en.md").read_text(encoding="utf-8-sig")
        versions = lambda text: re.findall(r"^## (\d+\.\d+\.\d+)\b", text, re.M)
        self.assertEqual(versions(zh), versions(en))
        version = tomllib.loads((ROOT / "app/Cargo.toml").read_text())["package"]["version"]
        self.assertEqual(versions(zh)[0], version)
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        self.assertEqual(next(p["version"] for p in lock["package"] if p["name"] == "luciddesk"), version)
        for name in ["README.md", "README.en.md"]:
            badges = re.findall(r"badge/version-([0-9]+\.[0-9]+\.[0-9]+)-", (ROOT / name).read_text(encoding="utf-8"))
            self.assertEqual(badges, [version])
        self.assertIn(f"**{version}**", zh.split("\n## ")[0])
        self.assertIn(f"**{version}**", en.split("\n## ")[0])

        for ver in versions(zh):
            notes.bilingual(zh, en, "v" + ver, "Yuch3nE/luciddesk")


if __name__ == "__main__":
    unittest.main()
