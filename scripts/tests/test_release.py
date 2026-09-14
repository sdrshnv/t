import hashlib
import importlib.util
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("release", ROOT / "scripts" / "release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def make_archive(path, binary=b"#!/bin/sh\necho 't 0.1.0'\n"):
    with tarfile.open(path, "w:gz") as archive:
        for name, data, mode in [("t", binary, 0o755), ("README.md", b"t\n", 0o644)]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = mode
            archive.addfile(info, io.BytesIO(data))


class ReleaseTests(unittest.TestCase):
    def test_tag_must_match_stable_package_version(self):
        release.validate_tag("0.1.0", "v0.1.0")
        release.validate_tag("0.1.0", "")
        for tag in ["v0.2.0", "0.1.0", "v0.1.0-rc.1", "v../0.1.0"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_tag("0.1.0", tag)

    def test_complete_release_has_pinned_installer_and_valid_checksums(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp)
            for target in release.TARGETS:
                make_archive(output / release.archive_name("0.1.0", target))
            release.assemble("0.1.0", output)
            installer = (output / "install.sh").read_text()
            self.assertIn("version='v0.1.0'", installer)
            self.assertNotIn("@VERSION@", installer)
            lines = (output / "SHA256SUMS").read_text().splitlines()
            self.assertEqual(len(lines), 5)
            for line in lines:
                checksum, name = line.split()
                self.assertEqual(checksum, hashlib.sha256((output / name).read_bytes()).hexdigest())

    def test_incomplete_or_unexpected_archives_block_assembly(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp)
            with self.assertRaisesRegex(ValueError, "archive set mismatch"):
                release.assemble("0.1.0", output)
            for target in release.TARGETS:
                make_archive(output / release.archive_name("0.1.0", target))
            make_archive(output / "unexpected.tar.gz")
            with self.assertRaisesRegex(ValueError, "archive set mismatch"):
                release.assemble("0.1.0", output)
            self.assertFalse((output / "install.sh").exists())

    def test_invalid_archive_blocks_assembly(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp)
            for target in release.TARGETS:
                make_archive(output / release.archive_name("0.1.0", target))
            path = output / release.archive_name("0.1.0", release.TARGETS[0])
            with tarfile.open(path, "w:gz"):
                pass
            with self.assertRaisesRegex(ValueError, "unexpected archive contents"):
                release.assemble("0.1.0", output)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "tools"
        self.bin.mkdir()
        # Use a controlled PATH so missing-tool and checksum fallback tests are real.
        for name in ["sh", "grep", "awk", "tar", "gzip", "mktemp", "install", "mv", "mkdir", "rm", "cp", "chmod", "sha256sum", "shasum"]:
            tool = shutil.which(name)
            if tool:
                (self.bin / name).symlink_to(tool)
        self.fixtures = self.root / "downloads"
        self.fixtures.mkdir()
        self.work = self.root / "temporary"
        self.work.mkdir()
        self.destination = self.root / "custom bin"
        self.env = {
            **os.environ,
            "PATH": str(self.bin),
            "HOME": str(self.root / "home"),
            "TMPDIR": str(self.work),
            "FIXTURE_DIR": str(self.fixtures),
            "DOWNLOAD_LOG": str(self.root / "downloads.log"),
            "TEST_OS": "Linux",
            "TEST_ARCH": "x86_64",
            "TEST_TRANSLATED": "0",
        }
        self.write_tool("uname", 'case "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac')
        self.write_tool("sysctl", 'echo "$TEST_TRANSLATED"')
        self.write_tool("curl", '''
set -eu
url=
output=
while [ "$#" -gt 0 ]; do
    case "$1" in
        https://*) url=$1; shift ;;
        -o) output=$2; shift 2 ;;
        *) shift ;;
    esac
done
echo "$url" >> "$DOWNLOAD_LOG"
if [ "${FAIL_DOWNLOAD:-0}" = 1 ]; then exit 22; fi
cp "$FIXTURE_DIR/${url##*/}" "$output"
''')
        self.installer = self.root / "install.sh"
        self.installer.write_text((ROOT / "scripts" / "install.sh").read_text().replace("@VERSION@", "v0.1.0"))

    def write_tool(self, name, content):
        path = self.bin / name
        path.write_text("#!/bin/sh\n" + content + "\n")
        path.chmod(0o755)

    def prepare(self, target="x86_64-unknown-linux-musl", version="0.1.0"):
        archive = self.fixtures / release.archive_name(version, target)
        make_archive(archive, f"#!/bin/sh\necho 't {version}'\n".encode())
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        (self.fixtures / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n")

    def run_installer(self, *args):
        return subprocess.run(
            ["/bin/sh", str(self.installer), *args],
            env=self.env, text=True, capture_output=True,
        )

    def install(self, *args):
        return self.run_installer("--install-dir", str(self.destination), *args)

    def assert_clean(self):
        self.assertEqual(list(self.work.iterdir()), [])
        if self.destination.exists():
            self.assertEqual(list(self.destination.glob(".t.*")), [])

    def test_supported_platforms_and_rosetta(self):
        for os_name, arch, translated, target in [
            ("Linux", "x86_64", "0", "x86_64-unknown-linux-musl"),
            ("Linux", "aarch64", "0", "aarch64-unknown-linux-musl"),
            ("Darwin", "x86_64", "0", "x86_64-apple-darwin"),
            ("Darwin", "arm64", "0", "aarch64-apple-darwin"),
            ("Darwin", "x86_64", "1", "aarch64-apple-darwin"),
        ]:
            with self.subTest(os=os_name, arch=arch, translated=translated):
                self.prepare(target)
                self.env.update(TEST_OS=os_name, TEST_ARCH=arch, TEST_TRANSLATED=translated)
                result = self.install()
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(os.access(self.destination / "t", os.X_OK))
                self.assertIn(target, result.stdout)
                self.assertIn("PATH", result.stdout)
                self.assert_clean()

    def test_default_directory_and_explicit_version_upgrade(self):
        self.prepare()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        installed = Path(self.env["HOME"]) / ".local/bin/t"
        self.assertTrue(installed.is_file())
        self.prepare(version="0.2.0")
        result = self.run_installer("--version", "v0.2.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("t 0.2.0", installed.read_text())
        self.assertIn("/download/v0.2.0/", (self.root / "downloads.log").read_text())
        self.assert_clean()

    def test_checksum_and_download_failures_preserve_existing_binary(self):
        self.destination.mkdir()
        binary = self.destination / "t"
        binary.write_text("previous version")
        for failure in ["checksum", "missing checksum", "download", "extraction"]:
            with self.subTest(failure=failure):
                self.prepare()
                self.env.pop("FAIL_DOWNLOAD", None)
                if failure == "checksum":
                    (self.fixtures / "SHA256SUMS").write_text("0" * 64 + "  t-v0.1.0-x86_64-unknown-linux-musl.tar.gz\n")
                elif failure == "missing checksum":
                    (self.fixtures / "SHA256SUMS").write_text("")
                elif failure == "download":
                    self.env["FAIL_DOWNLOAD"] = "1"
                else:
                    archive = self.fixtures / release.archive_name("0.1.0", release.TARGETS[0])
                    archive.write_bytes(b"not an archive")
                    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
                    (self.fixtures / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n")
                result = self.install()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(binary.read_text(), "previous version")
                self.assert_clean()

    def test_shasum_fallback(self):
        if not (self.bin / "shasum").exists():
            self.skipTest("shasum is not installed")
        (self.bin / "sha256sum").unlink(missing_ok=True)
        self.prepare()
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_tools_and_unsupported_platforms(self):
        self.prepare()
        self.env["TEST_OS"] = "FreeBSD"
        result = self.install()
        self.assertIn("unsupported operating system", result.stderr)
        self.env.update(TEST_OS="Linux", TEST_ARCH="i686")
        result = self.install()
        self.assertIn("unsupported architecture", result.stderr)
        self.env["TEST_ARCH"] = "x86_64"
        (self.bin / "curl").unlink()
        result = self.install()
        self.assertIn("required tool not found: curl", result.stderr)
        self.assertFalse(self.destination.exists())

    def test_arguments_and_missing_home(self):
        for args in [("--version",), ("--install-dir",), ("--version", "v../bad"), ("--unknown",)]:
            with self.subTest(args=args):
                self.assertNotEqual(self.run_installer(*args).returncode, 0)
        self.env.pop("HOME", None)
        self.assertIn("set HOME", self.run_installer().stderr)
        self.prepare()
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(self.run_installer("--help").returncode, 0)


class PublishTests(unittest.TestCase):
    def run_publish(self, draft="true", fail_upload=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            gh = root / "gh"
            gh.write_text('''#!/bin/sh
echo "$*" >> "$CALL_LOG"
case "$2" in
    view) if [ "$DRAFT" = missing ]; then exit 1; else echo "$DRAFT"; fi ;;
    upload) if [ "$FAIL_UPLOAD" = 1 ]; then exit 1; fi ;;
esac
''')
            gh.chmod(0o755)
            log = root / "calls"
            result = subprocess.run(
                ["bash", str(ROOT / "scripts" / "publish-release.sh")],
                cwd=root, text=True, capture_output=True,
                env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}",
                     "GH_REPO": "sdrshnv/t", "RELEASE_TAG": "v0.1.0",
                     "DRAFT": draft, "FAIL_UPLOAD": "1" if fail_upload else "0", "CALL_LOG": str(log)},
            )
            return result, log.read_text()

    def test_published_release_is_not_overwritten(self):
        result, calls = self.run_publish(draft="false")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("upload", calls)
        self.assertNotIn("edit", calls)

    def test_upload_failure_keeps_release_draft(self):
        result, calls = self.run_publish(fail_upload=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("edit", calls)

    def test_create_and_retry_draft(self):
        for draft in ["missing", "true"]:
            with self.subTest(draft=draft):
                result, calls = self.run_publish(draft=draft)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual("create" in calls, draft == "missing")
                self.assertLess(calls.index("upload"), calls.index("edit"))


if __name__ == "__main__":
    unittest.main()
