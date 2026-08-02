from __future__ import annotations

import ast
import unittest
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[1] / "src" / "doudou"


def imported_modules(filename: Path) -> set[str]:
    tree = ast.parse(filename.read_text(encoding="utf-8"), filename=str(filename))
    modules: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            modules.add("." * node.level + (node.module or ""))
    return modules


class ArchitectureTests(unittest.TestCase):
    def test_package_root_contains_only_entrypoints(self) -> None:
        root_modules = sorted(path.name for path in SOURCE.glob("*.py"))
        self.assertEqual(root_modules, ["__init__.py", "cli.py"])

    def test_runtime_does_not_import_feature_adapters(self) -> None:
        for filename in (SOURCE / "runtime").rglob("*.py"):
            imports = imported_modules(filename)
            self.assertFalse(
                any("features" in module for module in imports),
                f"{filename} imports features",
            )

    def test_storage_and_protocol_do_not_import_browser_or_ui(self) -> None:
        for package in ("storage", "protocol"):
            for filename in (SOURCE / package).rglob("*.py"):
                imports = imported_modules(filename)
                forbidden = [
                    module
                    for module in imports
                    if "runtime.browser" in module or ".ui" in module
                ]
                self.assertEqual(
                    forbidden, [], f"{filename} crosses architecture boundary"
                )

    def test_cli_does_not_embed_protocol_fields_or_page_selectors(self) -> None:
        content = (SOURCE / "cli.py").read_text(encoding="utf-8")
        for forbidden in (
            "contenteditable",
            "msg-item-content",
            "decode_fields",
            "message_type",
        ):
            self.assertNotIn(forbidden, content)

    def test_page_selectors_are_centralized_in_versioned_adapters(self) -> None:
        for filename in SOURCE.rglob("*.py"):
            if (
                filename == SOURCE / "ui" / "adapters.py"
                or (SOURCE / "ui" / "components") in filename.parents
            ):
                continue
            content = filename.read_text(encoding="utf-8")
            for forbidden in (
                "[data-e2e",
                ".second-verify-mask",
                ".comment-input-container",
                ".semi-upload-add",
            ):
                self.assertNotIn(
                    forbidden, content, f"{filename} embeds a page selector"
                )

    def test_write_modules_use_component_adapters(self) -> None:
        for name in ("chat.py", "video.py"):
            content = (SOURCE / "ui" / name).read_text(encoding="utf-8")
            self.assertIn(".components import", content)
            for forbidden in (
                "CHAT_ADAPTER.locator",
                "VIDEO_ADAPTER.locator",
                'locator("svg',
                'locator(".comment',
            ):
                self.assertNotIn(forbidden, content)

    def test_protocol_reads_do_not_call_page_response_json(self) -> None:
        for filename in (SOURCE / "reads").rglob("*.py"):
            self.assertNotIn(
                "response.json()",
                filename.read_text(encoding="utf-8"),
                f"{filename} bypasses response classification",
            )


if __name__ == "__main__":
    unittest.main()
