import random
import unittest
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
