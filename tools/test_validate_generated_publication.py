import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from validate_generated_publication import SDK_BRANCHES, allowed, validate


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.old = os.getcwd()
        os.chdir(self.tmp.name)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.invalid')
        Path('README.md').write_text('base\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'base')
        self.base = self.git('rev-parse', 'HEAD').strip()
        self.branch = 'automation/aws-sdk-go-v1-data'
        self.path = Path('crates/sdks/cloudcover-aws-sdk-go-v1/data/1.2.3.json')
        self.path.parent.mkdir(parents=True)
        self.path.write_text('{}')

    def tearDown(self):
        os.chdir(self.old)
        self.tmp.cleanup()

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True)

    def commit(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'generated')

    def test_allowed_worktree_and_commit(self):
        validate(self.branch, self.base, True)
        self.commit()
        validate(self.branch, self.base)

    def test_other_sdk_is_blocked(self):
        self.commit()
        with self.assertRaises(ValueError):
            validate('automation/aws-sdk-go-v2-data', self.base)

    def test_code_change_blocked_even_if_unstaged(self):
        Path('README.md').write_text('changed')
        with self.assertRaises(ValueError):
            validate(self.branch, self.base, True)

    def test_untracked_code_is_blocked(self):
        Path('evil.py').write_text('pass')
        with self.assertRaises(ValueError):
            validate(self.branch, self.base, True)

    def test_workflow_change_blocked(self):
        p = Path('.github/workflows/evil.yml')
        p.parent.mkdir(parents=True)
        p.write_text('name: evil')
        self.commit()
        with self.assertRaises(ValueError):
            validate(self.branch, self.base)

    def test_symlink_blocked(self):
        self.path.unlink()
        self.path.symlink_to('../../../../README.md')
        with self.assertRaises(ValueError):
            validate(self.branch, self.base, True)
        self.commit()
        with self.assertRaises(ValueError):
            validate(self.branch, self.base)

    def test_executable_blocked(self):
        # Windows chmod cannot set POSIX execute bits. The updater runs on
        # Ubuntu; exercise its worktree guard where those bits are supported.
        if os.name != 'nt':
            self.path.chmod(0o755)
            with self.assertRaises(ValueError):
                validate(self.branch, self.base, True)
        # Git tree modes are portable, even when the host filesystem is not.
        self.git('add', '.')
        self.git('update-index', '--chmod=+x', self.path.as_posix())
        self.git('commit', '-qm', 'executable generated file')
        with self.assertRaisesRegex(ValueError, 'regular non-executable'):
            validate(self.branch, self.base)

    def test_invalid_json_blocked(self):
        self.path.write_text('not json')
        with self.assertRaises(json.JSONDecodeError):
            validate(self.branch, self.base, True)

    def test_unknown_branch_and_nonimmutable_base_blocked(self):
        with self.assertRaises(ValueError):
            validate('main', self.base, True)
        with self.assertRaises(ValueError):
            validate(self.branch, 'main', True)

    def test_dirty_committed_publication_blocked(self):
        self.commit()
        self.path.write_text('{"uncommitted":true}')
        with self.assertRaises(ValueError):
            validate(self.branch, self.base)

    def test_deletion_blocked(self):
        self.commit()
        base = self.git('rev-parse', 'HEAD').strip()
        self.path.unlink()
        self.commit()
        with self.assertRaises(ValueError):
            validate(self.branch, base)

    def test_all_branch_allowlists(self):
        for branch, crate in SDK_BRANCHES.items():
            self.assertTrue(allowed(branch, f'crates/sdks/{crate}/data/mappings.json'))
            self.assertTrue(allowed(branch, f'crates/sdks/{crate}/metadata-manifest.json'))
            self.assertFalse(allowed(branch, f'crates/sdks/{crate}/data/nested/code.json'))
            self.assertFalse(allowed(branch, f'crates/sdks/{crate}/data/../evil.json'))
        self.assertTrue(allowed('automation/aws-sar-catalog', 'crates/clouds/cloudcover-aws/data/sar/catalog.json'))
        self.assertFalse(allowed('automation/aws-sar-catalog', 'crates/clouds/cloudcover-aws/src/lib.rs'))

class WorkflowContractTests(unittest.TestCase):
    def test_credentials_and_publication_guards(self):
        root = Path(__file__).resolve().parent.parent
        files = sorted((root / '.github/workflows').glob('update-*.yml'))
        self.assertEqual(len(files), 5)
        for path in files:
            with self.subTest(path=path.name):
                text = path.read_text()
                self.assertIn("if: github.repository == 'cyruscook/cloudcover' && github.ref == 'refs/heads/main'", text)
                self.assertIn('environment: cloudcover-updater', text)
                self.assertIn('persist-credentials: false', text)
                self.assertNotIn('\n  contents: write', text)
                self.assertNotIn('\n      contents: write', text)
                self.assertEqual(text.count('secrets.CLOUDCOVER_UPDATER_APP_PRIVATE_KEY'), 1)
                self.assertIn('repositories: cloudcover', text)
                self.assertIn('permission-contents: write', text)
                self.assertIn('permission-pull-requests: write', text)
                self.assertNotIn('skip-token-revoke:', text)
                self.assertLess(text.index('Validate committed publication'), text.index('id: updater-token'))
                self.assertLess(text.index('id: updater-token'), text.index('      - name: Push '))
                publication = text[text.index('      - name: Push '):]
                self.assertEqual(publication.count('GH_TOKEN: ${{ steps.updater-token.outputs.token }}'), 2)
                self.assertNotIn('GH_TOKEN: ${{ github.token }}', publication)
                self.assertIn('credential.helper', publication)
                self.assertTrue('--force-with-lease' in publication or 'publish_generated_branch.py' in publication)


if __name__ == '__main__':
    unittest.main()
