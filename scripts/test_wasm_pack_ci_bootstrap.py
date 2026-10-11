"""Regression checks for the verified wasm-pack CI bootstrap."""

import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts/install-wasm-pack-ci.sh"
PINNED_DIGESTS = {
    "x86_64-unknown-linux-musl":
        "278a8d668085821f4d1a637bd864f1713f872b0ae3a118c77562a308c0abfe8d",
    "aarch64-unknown-linux-musl":
        "5941c7b05060440ff37ee50fe9009a408e63fa5ba607a3b0736f5a887ec5f2ca",
}


class VerifiedWasmPackCITests(unittest.TestCase):
    def test_install_script_has_valid_bash_syntax(self):
        subprocess.run(["bash", "-n", str(INSTALLER)], check=True)

    def test_release_identity_is_pinned_for_both_linux_targets(self):
        script = INSTALLER.read_text()
        self.assertIn("version=0.14.0", script)
        self.assertIn("sha256sum --check --status", script)
        for target, digest in PINNED_DIGESTS.items():
            self.assertIn("target=" + target, script)
            self.assertIn("digest=" + digest, script)
        self.assertEqual(len(re.findall(r"^\s+digest=[a-f0-9]{64}$", script, re.M)), 2)
        self.assertIn('>> "$GITHUB_PATH"', script)

    def test_bad_download_is_rejected_without_installing(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            fake_bin = directory / "fake-bin"
            fake_bin.mkdir()
            fake_curl = fake_bin / "curl"
            fake_curl.write_text(
                '#!/usr/bin/env bash\n'
                'set -euo pipefail\n'
                'while [[ "$#" -gt 0 ]]; do\n'
                '  if [[ "$1" == "--output" ]]; then\n'
                '    printf "invalid archive bytes" > "$2"\n'
                '    exit 0\n'
                '  fi\n'
                '  shift\n'
                'done\n'
                'exit 2\n'
            )
            fake_curl.chmod(0o755)
            github_path = directory / "github-path"
            github_path.write_text("")
            env = dict(os.environ)
            env["PATH"] = str(fake_bin) + os.pathsep + os.environ["PATH"]
            env["RUNNER_TEMP"] = str(directory)
            env["GITHUB_PATH"] = str(github_path)
            result = subprocess.run(
                ["bash", str(INSTALLER)],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum did not match", result.stderr)
            self.assertFalse((directory / "wasm-pack-ci/bin/wasm-pack").exists())
            self.assertEqual(github_path.read_text(), "")

    def test_all_artifact_producers_use_verified_bootstrap(self):
        for name in ("validate.yml", "pages.yml", "release.yml"):
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                self.assertIn("bash scripts/install-wasm-pack-ci.sh", workflow)
                self.assertNotIn("cargo install wasm-pack", workflow)
                self.assertIn("reuse_across_runs: true", workflow)
                self.assertIn("consume-build-artifact@", workflow)


if __name__ == "__main__":
    unittest.main()
