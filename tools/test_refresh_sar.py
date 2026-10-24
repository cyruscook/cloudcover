"""Offline refresh tests: catalog changes gate publication, never wall-clock time."""
import copy
import json
import pathlib
import tempfile
import unittest
from unittest import mock

import refresh_sar as sar


def catalog(actions=None, operations=None):
    document = {'Name': 'test', 'Actions': actions or [{'Name': 'Read'}],
                'Operations': operations or [{'Name': 'Read', 'AuthorizedActions': [{'Service': 'test', 'Name': 'Read'}]}]}
    return {'schema_version': 1, 'services': [{
        'service': 'test', 'url': sar.URL + 'test.json', 'modified': 123,
        'source_sha256': sar.sha256(sar.canonical(document)), 'document': document}]}


def write_snapshot(directory, value):
    directory.mkdir(parents=True, exist_ok=True)
    content = sar.canonical(value)
    manifest = {'schema_version': 1, 'source': sar.URL, 'retrieved_at': '2020-01-01T00:00:00+00:00',
                'index_sha256': 'a' * 64, 'catalog_sha256': sar.sha256(content),
                'refresh_script_sha256': 'b' * 64, 'extractor_revision': 'original-revision',
                'service_count': len(value['services'])}
    (directory / 'catalog.json').write_bytes(content)
    (directory / 'manifest.json').write_bytes(sar.canonical(manifest))


def fingerprint(directory):
    return {path.name: (path.read_bytes(), path.stat().st_mtime_ns) for path in directory.iterdir()}


class RefreshTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.current = self.root / 'current'
        self.original = catalog()
        write_snapshot(self.current, self.original)

    def test_unchanged_input_preserves_both_bytes_and_mtimes(self):
        before = fingerprint(self.current)
        report = self.root / 'body.md'
        with mock.patch.object(sar, 'collect_catalog', return_value=(b'new index bytes', self.original)), \
                mock.patch.object(sar.subprocess, 'check_output') as revision:
            self.assertFalse(sar.refresh(self.current, summary_path=report))
            revision.assert_not_called()
        self.assertEqual(fingerprint(self.current), before)
        self.assertFalse(report.exists())

    def test_identical_pending_pr_does_not_republish_daily(self):
        pending = self.root / 'pending'
        fetched = catalog(actions=[{'Name': 'NewAction'}])
        write_snapshot(pending, fetched)
        current_before, pending_before = fingerprint(self.current), fingerprint(pending)
        with mock.patch.object(sar, 'collect_catalog', return_value=(b'index', fetched)), \
                mock.patch.object(sar.subprocess, 'check_output') as revision:
            self.assertFalse(sar.refresh(self.current, previous_dir=pending))
            revision.assert_not_called()
        self.assertEqual(fingerprint(self.current), current_before)
        self.assertEqual(fingerprint(pending), pending_before)

    def test_main_already_matches_live_input_even_if_pending_pr_is_old(self):
        pending = self.root / 'pending'
        write_snapshot(pending, catalog(actions=[{'Name': 'SupersededAction'}]))
        before = fingerprint(self.current)
        with mock.patch.object(sar, 'collect_catalog', return_value=(b'index', self.original)):
            self.assertFalse(sar.refresh(self.current, previous_dir=pending))
        self.assertEqual(fingerprint(self.current), before)

    def test_changed_catalog_publishes_checksum_pair_and_semantic_summary(self):
        original = catalog(actions=[{'Name': 'Read'}, {'Name': 'Old'}], operations=[{'Name': 'Old'}])
        write_snapshot(self.current, original)
        changed = catalog(actions=[{'Name': 'Read', 'ActionConditionKeys': ['test:Context']}, {'Name': 'New'}],
                          operations=[{'Name': 'New'}])
        pending = self.root / 'pending'
        write_snapshot(pending, catalog(actions=[{'Name': 'OtherPendingChange'}]))
        report = self.root / 'body.md'
        with mock.patch.object(sar, 'collect_catalog', return_value=(b'new-index', changed)), \
                mock.patch.object(sar.subprocess, 'check_output', return_value='new-revision\n'):
            self.assertTrue(sar.refresh(self.current, previous_dir=pending, summary_path=report))
        content, loaded = sar.read_snapshot(self.current)
        self.assertEqual(content, sar.canonical(changed))
        self.assertEqual(loaded, changed)
        manifest = json.loads((self.current / 'manifest.json').read_bytes())
        self.assertEqual(manifest['catalog_sha256'], sar.sha256(content))
        self.assertEqual(manifest['index_sha256'], sar.sha256(b'new-index'))
        self.assertEqual(manifest['extractor_revision'], 'new-revision')
        body = report.read_text()
        self.assertIn('| IAM actions | 1 | 1 | 1 |', body)
        self.assertIn('| API operations | 1 | 1 | 0 |', body)
        self.assertIn(sar.sha256(sar.canonical(original)), body)
        self.assertIn(sar.sha256(content), body)
        self.assertIn('Approve workflows to run', body)
        published = fingerprint(self.current)
        with mock.patch.object(sar, 'collect_catalog', return_value=(b'index changed again', changed)):
            self.assertFalse(sar.refresh(self.current, previous_dir=pending))
        self.assertEqual(fingerprint(self.current), published)

    def test_corrupt_current_or_pending_manifest_fails_before_fetch(self):
        for pending in [False, True]:
            directory = self.root / 'pending' if pending else self.current
            write_snapshot(directory, self.original)
            manifest = json.loads((directory / 'manifest.json').read_bytes())
            manifest['catalog_sha256'] = '0' * 64
            (directory / 'manifest.json').write_bytes(sar.canonical(manifest))
            with mock.patch.object(sar, 'collect_catalog') as collect, self.assertRaises(ValueError):
                sar.refresh(self.current, previous_dir=directory if pending else None)
            collect.assert_not_called()
            write_snapshot(self.current, self.original)

    def test_unsupported_schemas_and_malformed_service_envelopes_fail(self):
        for mutate in [lambda value: value.update(schema_version=2),
                       lambda value: value['services'].append(copy.deepcopy(value['services'][0])),
                       lambda value: value['services'][0].update(service=None),
                       lambda value: value['services'][0].update(modified=True)]:
            value = copy.deepcopy(self.original)
            mutate(value)
            with self.assertRaises(ValueError):
                sar.validate_catalog(value)
        manifest = json.loads((self.current / 'manifest.json').read_bytes())
        manifest['schema_version'] = 2
        (self.current / 'manifest.json').write_bytes(sar.canonical(manifest))
        with self.assertRaises(ValueError):
            sar.read_snapshot(self.current)

    def test_fetch_is_deterministic_and_index_change_leaves_snapshot_untouched(self):
        index = [{'service': name, 'url': sar.URL + name + '.json', 'modified': 42} for name in ['z', 'a']]
        index_bytes = sar.canonical(index)
        replies = {sar.URL: index_bytes, **{entry['url']: sar.canonical({'Name': entry['service'], 'Actions': []}) for entry in index}}
        with mock.patch.object(sar, 'fetch', side_effect=replies.__getitem__):
            first = sar.collect_catalog()
            second = sar.collect_catalog()
        self.assertEqual(sar.canonical(first[1]), sar.canonical(second[1]))
        self.assertEqual([entry['service'] for entry in first[1]['services']], ['a', 'z'])
        before = fingerprint(self.current)
        calls = 0

        def fetch(url):
            nonlocal calls
            if url == sar.URL:
                calls += 1
                return index_bytes if calls == 1 else b'[]'
            return replies[url]

        with mock.patch.object(sar, 'fetch', side_effect=fetch), self.assertRaises(RuntimeError):
            sar.refresh(self.current)
        self.assertEqual(fingerprint(self.current), before)


if __name__ == '__main__':
    unittest.main()
