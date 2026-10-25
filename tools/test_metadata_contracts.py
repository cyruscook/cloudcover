import contextlib
import io
import json
import pathlib
import random
import sys
import tempfile
import unittest
from unittest import mock
import metadata_contracts
from metadata_contracts import apply, version, digest

class Contracts(unittest.TestCase):
    def test_exact_versions(self):
        for raw in ['1.2', '01.2.3', '1.2.3+build', 'v1.2.3', '1.2.3-beta']:
            with self.assertRaises(ValueError): version(raw)
        self.assertEqual(version('0.1.0'), (0, 1, 0))

    def test_arrays_duplicates_conflicts_and_missing_removals(self):
        for delta in [dict(remove=None, upsert=[]), dict(remove=[['p','C','M']], upsert=[]),
                      dict(remove=[], upsert=[['p','C','M', [['s','A']]], ['p','C','M', [['s','B']]]])]:
            with self.assertRaises((ValueError, TypeError)): apply(delta, {})
        state = {('p','C','M'): (('s','A'),)}
        with self.assertRaises(ValueError):
            apply(dict(remove=[['p','C','M']], upsert=[['p','C','M', [['s','B']]]]), state)
        with self.assertRaises(ValueError): apply(dict(remove=[], upsert=[['p','C','M', [['','A']]]]), {})

    def test_known_no_calls_is_sdk_specific(self):
        delta = dict(remove=[], upsert=[['resource','aws_test','read', []]])
        with self.assertRaises(ValueError): apply(delta, {})
        state = {}; apply(delta, state, allow_empty=True)
        self.assertEqual(state[('resource','aws_test','read')], ())

    def test_randomized_diff_round_trips(self):
        rng = random.Random(5938)
        for _ in range(100):
            before = {('p','C',str(i)): (('s',str(rng.randrange(3))),) for i in range(20) if rng.randrange(2)}
            after = {('p','C',str(i)): (('s',str(rng.randrange(3))),) for i in range(20) if rng.randrange(2)}
            delta = dict(remove=[list(k) for k in before if k not in after],
                         upsert=[[*k, list(map(list,v))] for k,v in after.items() if before.get(k) != v])
            actual = dict(before); apply(delta, actual)
            self.assertEqual(actual, after)
            self.assertEqual(digest(sorted(actual.items())), digest(sorted(after.items())))

class Shapes(unittest.TestCase):
    def test_go_and_terraform_tuple_shapes(self):
        rows = [
            [None, 'C', 'M', [['s','A']]],
            ['p', None, 'M', [['s','A']]],
            ['p', 'C', None, [['s','A']]],
            ['p', 'C', 'M', ['sA']],
            ['p', 'C', 'M', {'s':'A'}],
            ['p', 'C', 'M', [('s','A')]],
            ['p', 'C', 'M', [['s','A','extra']]],
            ['p', 'C', 'M', [[None,'A']]],
            ['p', 'C', 'M', [['s',1]]],
            ['p', 'C', 'M', [['s','A']], 'extra'],
            ['p', 'C', 'M'],
            'pCMx',
        ]
        for allow_empty in (False, True):
            for row in rows:
                with self.subTest(row=row, terraform=allow_empty), self.assertRaises(ValueError):
                    apply(dict(remove=[], upsert=[row]), {}, allow_empty=allow_empty)
        for removal in [['p','C'], ['p','C','M','extra'], [None,'C','M'], 'pCM']:
            with self.assertRaises(ValueError):
                apply(dict(remove=[removal], upsert=[]), {('p','C','M'): (('s','A'),)})

    def test_js_null_is_only_valid_for_receiver(self):
        valid = dict(receiver=None, method='M', api_methods=[dict(service='s', name='A')])
        state = {}; apply(dict(remove=[], upsert=[valid]), state, javascript=True)
        self.assertEqual(state, {(None,'M'): (('s','A'),)})
        invalid = [dict(valid, method=None), dict(valid, receiver=1), dict(valid, receiver=''),
                   dict(valid, api_methods=['sA']), dict(valid, api_methods=[['s','A']]),
                   dict(valid, api_methods=[dict(service=None, name='A')]),
                   dict(valid, api_methods=[dict(service='s', name='A', extra=True)]),
                   dict(receiver=None, method='M'), ['p','C','M',[]]]
        for row in invalid:
            with self.subTest(row=row), self.assertRaises(ValueError):
                apply(dict(remove=[], upsert=[row]), {}, javascript=True)
        for removal in [[None,None], [None,'M','extra'], 'MM']:
            with self.assertRaises(ValueError):
                apply(dict(remove=[removal], upsert=[]), state, javascript=True)


class ManifestPortability(unittest.TestCase):
    def test_portable_paths_deterministic_writes_and_strict_line_endings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            go = root / 'cloudcover-aws-sdk-go-v1'
            terraform = root / 'cloudcover-terraform-provider-aws'
            for directory in [go, terraform]:
                (directory / 'data').mkdir(parents=True)
            (go / 'data/café.json').write_bytes(json.dumps({
                'module_path': 'example/sdk',
                'releases': [{'module_version': '1.0.0', 'remove': [], 'upsert': []}],
            }).encode('utf-8'))
            (terraform / 'data/1.0.0.json').write_bytes(json.dumps({
                'provider_version': '1.0.0', 'remove': [], 'upsert': [],
            }).encode('utf-8'))
            (go / 'generator').mkdir()
            (go / 'generator/extractor.go').write_bytes('// café\n'.encode('utf-8'))
            (go / 'src/bin').mkdir(parents=True)
            (go / 'src/bin/generate.rs').write_bytes(b'// extractor\n')
            (go / 'src/data.rs').write_bytes(b'// reader\n')

            def run(*arguments):
                with mock.patch.object(metadata_contracts, 'SDK_ROOT', root), \
                        mock.patch.object(sys, 'argv', ['metadata_contracts.py', *arguments]), \
                        contextlib.redirect_stdout(io.StringIO()):
                    metadata_contracts.main()

            run('--write')
            paths = [go / 'metadata-manifest.json', terraform / 'metadata-manifest.json']
            original = [path.read_bytes() for path in paths]
            for content in original:
                self.assertTrue(content.endswith(b'\n'))
                self.assertNotIn(b'\r', content)
            manifest = json.loads(original[0])
            self.assertEqual(set(manifest['sources']), {'data/café.json'})
            self.assertEqual(set(manifest['extractor_sources']), {
                'generator/extractor.go', 'src/bin/generate.rs', 'src/data.rs',
            })
            run()
            run('--write')
            self.assertEqual([path.read_bytes() for path in paths], original)
            paths[0].write_bytes(original[0].replace(b'\n', b'\r\n'))
            with self.assertRaisesRegex(SystemExit, 'manifest is stale'):
                run()
