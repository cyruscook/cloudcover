"""Offline integration tests against a real bare remote; no GitHub writes."""
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

import publish_generated_branch as publisher


class PublicationTests(unittest.TestCase):
    branch = 'automation/test-data'
    ref = f'refs/heads/{branch}'

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = pathlib.Path(self.temporary.name)
        self.remote = root / 'remote.git'
        self.work = root / 'work'
        self.run_git(root, 'init', '--bare', str(self.remote))
        self.run_git(root, 'init', str(self.work))
        self.run_git(self.work, 'config', 'user.name', 'Test')
        self.run_git(self.work, 'config', 'user.email', 'test@example.invalid')
        self.run_git(self.work, 'remote', 'add', 'origin', str(self.remote))
        self.commit('base', 'base')
        self.base = self.run_git(self.work, 'rev-parse', 'HEAD')
        self.previous_cwd = pathlib.Path.cwd()
        os.chdir(self.work)
        self.addCleanup(os.chdir, self.previous_cwd)

    def run_git(self, cwd, *args, env=None):
        return subprocess.check_output(['git', *args], cwd=cwd, env=env,
                                       text=True, stderr=subprocess.PIPE).strip()

    def commit(self, name, content, timestamp='2026-01-01T00:00:00+00:00'):
        (self.work / name).write_text(content)
        self.run_git(self.work, 'add', name)
        env = {**os.environ, 'GIT_AUTHOR_DATE': timestamp, 'GIT_COMMITTER_DATE': timestamp}
        self.run_git(self.work, 'commit', '-m', 'Generated data', env=env)
        return self.run_git(self.work, 'rev-parse', 'HEAD')

    def remote_head(self):
        return self.run_git(self.remote, 'rev-parse', self.ref)

    def initial_publication(self):
        sha = self.commit('data', 'generated')
        self.run_git(self.work, 'push', 'origin', f'HEAD:{self.ref}')
        return sha

    def test_first_publication(self):
        candidate = self.commit('data', 'generated')
        self.assertTrue(publisher.publish(self.branch))
        self.assertEqual(self.remote_head(), candidate)

    def test_timestamp_only_change_preserves_existing_sha(self):
        original = self.initial_publication()
        self.run_git(self.work, 'reset', '--hard', self.base)
        candidate = self.commit('data', 'generated', '2026-01-02T00:00:00+00:00')
        self.assertNotEqual(candidate, original)
        self.assertFalse(publisher.publish(self.branch))
        self.assertEqual(self.remote_head(), original)

    def test_changed_data_is_published(self):
        self.initial_publication()
        self.run_git(self.work, 'reset', '--hard', self.base)
        candidate = self.commit('data', 'new generated data')
        self.assertTrue(publisher.publish(self.branch))
        self.assertEqual(self.remote_head(), candidate)

    def test_changed_base_tree_is_published_even_with_same_generated_data(self):
        self.initial_publication()
        self.run_git(self.work, 'reset', '--hard', self.base)
        self.commit('base', 'new base content')
        candidate = self.commit('data', 'generated')
        self.assertTrue(publisher.publish(self.branch))
        self.assertEqual(self.remote_head(), candidate)

    def race_after(self, operation, competing_sha):
        real_git = publisher.git

        def racing_git(*args):
            result = real_git(*args)
            if args[0] == operation:
                self.run_git(self.remote, 'update-ref', self.ref, competing_sha)
            return result
        return mock.patch.object(publisher, 'git', side_effect=racing_git)

    def test_branch_changed_between_listing_and_fetch_is_rejected(self):
        self.initial_publication()
        with self.race_after('ls-remote', self.base):
            with self.assertRaisesRegex(RuntimeError, 'changed during fetch'):
                publisher.publish(self.branch)
        self.assertEqual(self.remote_head(), self.base)

    def test_branch_changed_after_fetch_is_not_overwritten(self):
        self.initial_publication()
        self.commit('data', 'new generated data')
        with self.race_after('fetch', self.base):
            with self.assertRaises(subprocess.CalledProcessError):
                publisher.publish(self.branch)
        self.assertEqual(self.remote_head(), self.base)

    def test_competing_first_publication_is_not_overwritten(self):
        # Make the base object available remotely without creating the target branch.
        self.run_git(self.work, 'push', 'origin', f'{self.base}:refs/heads/main')
        self.commit('data', 'generated')
        with self.race_after('ls-remote', self.base):
            with self.assertRaises(subprocess.CalledProcessError):
                publisher.publish(self.branch)
        self.assertEqual(self.remote_head(), self.base)

    def test_failed_fetch_does_not_publish(self):
        original = self.initial_publication()
        self.commit('data', 'new generated data')
        real_git = publisher.git

        def failed_fetch(*args):
            if args[0] == 'fetch':
                raise subprocess.CalledProcessError(128, ['git', *args])
            return real_git(*args)

        with mock.patch.object(publisher, 'git', side_effect=failed_fetch):
            with self.assertRaises(subprocess.CalledProcessError):
                publisher.publish(self.branch)
        self.assertEqual(self.remote_head(), original)

    def test_identical_tree_after_base_history_only_change_keeps_existing_sha(self):
        original = self.initial_publication()
        self.run_git(self.work, 'reset', '--hard', self.base)
        self.run_git(self.work, 'commit', '--allow-empty', '-m', 'Base history only')
        candidate = self.commit('data', 'generated')
        self.assertNotEqual(candidate, original)
        self.assertFalse(publisher.publish(self.branch))
        self.assertEqual(self.remote_head(), original)

    def test_workflow_keeps_pr_recovery_after_noop_publication(self):
        workflow = (pathlib.Path(__file__).resolve().parents[1] / '.github/workflows'
                    / 'update-terraform-provider-aws-data.yml').read_text()
        publication = workflow.split('      - name: Push generated data branch\n', 1)[1]
        self.assertIn('python3 tools/publish_generated_branch.py "$BRANCH"', publication)
        recovery = publication.split('      - name: Create or update pull request\n', 1)[1]
        self.assertIn("if: steps.changes.outputs.changed == 'true'", recovery)
        self.assertIn('gh pr list', recovery)
        self.assertIn('gh pr create', recovery)
        self.assertNotIn('steps.publish', recovery)


if __name__ == '__main__':
    unittest.main()
