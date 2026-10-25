"""Exercise Git's real CRLF checkout filters for every byte-addressed input."""
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class CheckoutBytes(unittest.TestCase):
    @unittest.skipUnless(shutil.which('git') and (ROOT / '.git').exists(), 'requires a Git checkout')
    def test_autocrlf_preserves_all_checksum_protected_bytes(self):
        inputs = set()
        for sidecar in (ROOT / 'crates/sdks').glob('*/metadata-manifest.json'):
            manifest = json.loads(sidecar.read_bytes())
            inputs.add(sidecar.relative_to(ROOT).as_posix())
            for field in ['sources', 'extractor_sources']:
                for relative, expected in manifest[field].items():
                    source = sidecar.parent / relative
                    self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), expected, source)
                    inputs.add(source.relative_to(ROOT).as_posix())
        inputs.update([
            'crates/clouds/cloudcover-aws/data/sar/catalog.json',
            'crates/clouds/cloudcover-aws/data/sar/manifest.json',
            'tools/refresh_sar.py',
        ])
        control = 'AGENTS.md'  # Unprotected text proves conversion really occurred.
        with tempfile.TemporaryDirectory(prefix='cloudcover-checkout-') as temporary:
            directory = pathlib.Path(temporary)
            environment = dict(os.environ, GIT_INDEX_FILE=str(directory / 'index'))
            subprocess.run(['git', 'read-tree', 'HEAD'], cwd=ROOT, env=environment, check=True)
            # Capture current edits only in the temporary index, leaving the real
            # index and working tree intact so this test also works before commit.
            subprocess.run(['git', 'add', '--pathspec-from-file=-', '--pathspec-file-nul'],
                           input=('\0'.join(sorted(inputs | {'.gitattributes'})) + '\0').encode('utf-8'),
                           cwd=ROOT, env=environment, check=True, capture_output=True)
            subprocess.run([
                'git', '-c', 'core.autocrlf=true', '-c', 'core.eol=crlf',
                'checkout-index', '--prefix=' + directory.as_posix() + '/checkout/', '-z', '--stdin',
            ], input=('\0'.join(sorted(inputs | {control})) + '\0').encode('utf-8'),
                cwd=ROOT, env=environment, check=True, capture_output=True)
            checkout = directory / 'checkout'
            for relative in sorted(inputs):
                with self.subTest(path=relative):
                    self.assertEqual(hashlib.sha256((checkout / relative).read_bytes()).digest(),
                                     hashlib.sha256((ROOT / relative).read_bytes()).digest())
            original = subprocess.check_output(['git', 'show', 'HEAD:' + control], cwd=ROOT)
            converted = (checkout / control).read_bytes()
            self.assertIn(b'\r\n', converted)
            self.assertEqual(converted, original.replace(b'\n', b'\r\n'))


if __name__ == '__main__':
    unittest.main()
