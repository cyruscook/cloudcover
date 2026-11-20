import unittest

from review_sar_authorization import review


def catalog(links, present=True, action_present=True):
    return {'services': [{'service': 'test', 'document': {
        'Actions': [{'Name': 'Run'}] if action_present else [],
        'Operations': [{'Name': 'Run', 'AuthorizedActions': [
            {'Service': s, 'Name': n} for s, n in links]}] if present else []}}]}


class AuthorizationDriftTests(unittest.TestCase):
    def test_bridge_requires_the_reviewed_context_shape(self):
        before = catalog([('test', 'Run')])
        before['services'][0]['document']['Operations'][0]['AuthorizedActions'][0]['Context'] = {'test:Scope': ['narrow']}
        action = {'service': 'test', 'name': 'Run'}
        supplement = {'mappings': [{'service': 'test', 'operation': 'Run', 'authorized_actions': [action]}]}
        self.assertEqual(review(before, catalog([]), supplement)[0]['status'], 'unresolved')
        action['context'] = {'test:Scope': ['narrow']}
        self.assertEqual(review(before, catalog([]), supplement)[0]['status'], 'reviewed_compiler_bridge')
        action['context'] = {'test:Scope': ['other']}
        self.assertEqual(review(before, catalog([]), supplement)[0]['status'], 'unresolved')

    def test_malformed_and_duplicate_rows_fail_instead_of_hiding_drift(self):
        for links in [None, [{'Service': 'test', 'Name': ''}]]:
            invalid = catalog([])
            invalid['services'][0]['document']['Operations'][0]['AuthorizedActions'] = links
            with self.assertRaises(ValueError):
                review(invalid, catalog([]), {'mappings': []})
        invalid = catalog([])
        invalid['services'][0]['document']['Operations'] *= 2
        with self.assertRaises(ValueError):
            review(invalid, catalog([]), {'mappings': []})

    def test_context_variants_of_same_action_do_not_hide_loss(self):
        before = catalog([('test', 'Run'), ('test', 'Run')])
        rows = review(before, catalog([]), {'mappings': []})
        self.assertEqual(rows[0]['previous_actions'], ['test:Run'])
        self.assertEqual(rows[0]['status'], 'unresolved')

    def test_raw_loss_is_reported_even_with_exact_reviewed_bridge(self):
        before = catalog([('test', 'Run')])
        supplement = {'mappings': [{'service': 'test', 'operation': 'Run',
                       'authorized_actions': [{'service': 'test', 'name': 'Run'}]}]}
        for present, action_present, expected in [
                (True, True, 'reviewed_compiler_bridge'), (False, True, 'unresolved'),
                (True, False, 'unresolved')]:
            rows = review(before, catalog([], present, action_present), supplement)
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]['status'], expected)
            self.assertEqual(rows[0]['previous_actions'], ['test:Run'])

    def test_no_bridge_or_different_bridge_cannot_hide_loss(self):
        for mappings in [[], [{'service': 'test', 'operation': 'Run',
                              'authorized_actions': [{'service': 'test', 'name': 'Other'}]}]]:
            rows = review(catalog([('test', 'Run')]), catalog([]), {'mappings': mappings})
            self.assertEqual(rows[0]['status'], 'unresolved')

    def test_unchanged_and_previously_unmapped_are_not_regressions(self):
        for before in [catalog([]), catalog([('test', 'Run')])]:
            self.assertEqual(review(before, before, {'mappings': []}), [])
