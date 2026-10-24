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
