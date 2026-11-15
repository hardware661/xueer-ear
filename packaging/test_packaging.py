"""Packaging safety checks use synthetic files and mocked gh; no service or GitHub calls."""

import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parent.parent

import publish_release
import verify_payload


class PackagingTests(unittest.TestCase):
    def test_zip_rejects_sensitive_extra_and_nested_files(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "portable.zip"
            with zipfile.ZipFile(archive, "w") as output:
                for name in verify_payload.payload_names("1.2.3"):
                    output.writestr(name, b"test payload")
            verify_payload.check_zip(archive, "1.2.3")
            with zipfile.ZipFile(archive, "a") as output:
                output.writestr("private/device.json", b"must never ship")
            with self.assertRaises(AssertionError):
                verify_payload.check_zip(archive, "1.2.3")

    def test_publishing_keeps_draft_until_both_uploads_are_visible(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            files = ["xueer-ear-1.2.3-windows-x64-portable.zip", "xueer-ear-1.2.3-windows-x64-setup.exe"]
            sums = []
            for name in files:
                data = ("synthetic " + name).encode()
                (directory / name).write_bytes(data)
                sums.append(hashlib.sha256(data).hexdigest() + "  " + name)
            (directory / "SHA256SUMS.txt").write_text("\n".join(sums) + "\n")
            calls = []

            def fake_gh(*args, **kwargs):
                calls.append(args)
                if args[:2] == ("release", "view"):
                    if len(calls) == 1:
                        return subprocess.CompletedProcess(args, 1, "", "not found")
                    assets = [{"name": p.name, "size": p.stat().st_size} for p in directory.iterdir()]
                    return subprocess.CompletedProcess(args, 0, json.dumps({"isDraft": True, "assets": assets}), "")
                return subprocess.CompletedProcess(args, 0, "", "")

            with patch.object(publish_release, "gh", fake_gh):
                publish_release.publish(directory, "1.2.3", "v1.2.3", "owner/repo")
            self.assertIn("--draft", calls[1])
            self.assertEqual(calls[2][:2], ("release", "upload"))
            self.assertEqual(calls[-1][:2], ("release", "edit"))
            self.assertIn("--draft=false", calls[-1])

    def test_bad_checksum_never_contacts_github(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            for suffix in ["portable.zip", "setup.exe"]:
                (directory / ("xueer-ear-1.2.3-windows-x64-" + suffix)).write_bytes(b"changed")
            (directory / "SHA256SUMS.txt").write_text("0" * 64 + "  xueer-ear-1.2.3-windows-x64-portable.zip\n")
            with patch.object(publish_release, "gh") as command:
                with self.assertRaises(ValueError):
                    publish_release.publish(directory, "1.2.3", "v1.2.3", "owner/repo")
                command.assert_not_called()

    def test_installer_cleans_up_bridges_from_older_versions(self):
        # 审查 R13：桥接 DLL 文件名带版本号，升级后旧版本的一直留在安装目录，卸载也删不掉
        script = (ROOT / "packaging" / "xueer-ear.iss").read_text(encoding="utf-8")
        self.assertIn("RemoveOldBridges", script)
        self.assertIn("xueer_ear_bridge-*.dll", script)
        # 升级时（装完文件后）和卸载时都要清
        self.assertRegex(script, r"CurStepChanged[\s\S]*ssPostInstall[\s\S]*RemoveOldBridges")
        self.assertRegex(script, r"CurUninstallStepChanged[\s\S]*usPostUninstall[\s\S]*RemoveOldBridges")
        # 按当前用户装、没有管理员权限：RestartReplace 会抛异常，把整个清理中断（1.0.1 实测）
        self.assertNotIn("RestartReplace(", script)

    def test_missing_remote_installer_never_publishes_draft(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            sums = []
            for suffix in ["portable.zip", "setup.exe"]:
                name = "xueer-ear-1.2.3-windows-x64-" + suffix
                (directory / name).write_bytes(b"synthetic")
                sums.append(hashlib.sha256(b"synthetic").hexdigest() + "  " + name)
            (directory / "SHA256SUMS.txt").write_text("\n".join(sums) + "\n")
            calls = []

            def incomplete(*args, **kwargs):
                calls.append(args)
                if args[:2] == ("release", "view"):
                    assets = [{"name": p.name, "size": p.stat().st_size}
                              for p in directory.iterdir() if p.suffix != ".exe"]
                    return subprocess.CompletedProcess(args, 0, json.dumps({"isDraft": True, "assets": assets}), "")
                return subprocess.CompletedProcess(args, 0, "", "")

            with patch.object(publish_release, "gh", incomplete):
                with self.assertRaises(ValueError):
                    publish_release.publish(directory, "1.2.3", "v1.2.3", "owner/repo")
            self.assertFalse(any(args[:2] == ("release", "edit") for args in calls))


if __name__ == "__main__":
    unittest.main()
