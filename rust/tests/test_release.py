"""Manual release input and manifest/lockfile consistency regression tests."""
import importlib.util
from pathlib import Path
import tempfile
import tomllib
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "prepare_release.py"
SPEC = importlib.util.spec_from_file_location("prepare_release", SCRIPT)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_channel_versions(self):
        for raw, channel, expected in [
            ("0.2.0", "stable", "0.2.0"),
            ("v1.2.3", "stable", "1.2.3"),
            ("0.2.0", "beta", "0.2.0-beta.1"),
            ("0.2.0", "alpha", "0.2.0-alpha.1"),
            ("v0.2.0-beta.3", "beta", "0.2.0-beta.3"),
            ("0.2.0-alpha", "alpha", "0.2.0-alpha.1"),
        ]:
            with self.subTest(raw=raw, channel=channel):
                self.assertEqual(release.release_version(raw, channel), expected)

    def test_invalid_inputs(self):
        for raw, channel in [
            ("1.2", "stable"), ("01.2.3", "stable"),
            ("1.2.3-beta.1", "stable"), ("1.2.3-alpha.1", "beta"),
            ("1.2.3-beta.01", "beta"), ("1.2.3+local", "stable"),
            ("1.2.3\nother=value", "stable"), ("$(touch bad)", "stable"),
            ("1.2.3", "unknown"),
        ]:
            with self.subTest(raw=raw, channel=channel):
                with self.assertRaises(ValueError):
                    release.release_version(raw, channel)

    def test_stamps_both_workspace_packages_without_changing_dependencies(self):
        workspace = SCRIPT.parents[1]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("Cargo.toml", "Cargo.lock"):
                (root / name).write_bytes((workspace / name).read_bytes())
            before = tomllib.loads((root / "Cargo.lock").read_text())
            release.stamp_workspace(root, "2.3.4-beta.2")
            manifest = tomllib.loads((root / "Cargo.toml").read_text())
            after = tomllib.loads((root / "Cargo.lock").read_text())
            self.assertEqual(manifest["workspace"]["package"]["version"], "2.3.4-beta.2")
            for old, new in zip(before["package"], after["package"], strict=True):
                if old["name"] in ("eagler-ws", "eaglerx-pumpkin"):
                    self.assertEqual(new["version"], "2.3.4-beta.2")
                    new["version"] = old["version"]
                self.assertEqual(old, new)


if __name__ == "__main__":
    unittest.main()
