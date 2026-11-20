import copy
import json
from pathlib import Path
import unittest

from review_sar_authorization import review


def catalog(links, present=True, action_present=True):
    return {'services': [{'service': 'test', 'document': {
        'Actions': [{'Name': 'Run'}] if action_present else [],
        'Operations': [{'Name': 'Run', 'AuthorizedActions': [
            {'Service': s, 'Name': n} for s, n in links]}] if present else []}}]}


class AuthorizationDriftTests(unittest.TestCase):
    def disposition_fixture(self):
        dispositions = json.loads((Path(__file__).resolve().parents[1] /
            'crates/clouds/cloudcover-aws/data/authorization-dispositions.json').read_text())
        services = {}

        def service(name):
            return services.setdefault(name, {'service': name, 'document': {
                'Operations': [], 'Actions': [], 'Resources': []}})['document']

        for row in dispositions['entries']:
            service(row['service'])['Operations'].append({
                'Name': row['operation'], 'AuthorizedActions': copy.deepcopy(row['authorized_actions'])})
        for category, field in [('Actions', 'action_scopes'), ('Resources', 'resource_scopes')]:
            for key, shape in dispositions[field].items():
                prefix, name = key.split(':', 1)
                service(prefix)[category].append({'Name': name, **copy.deepcopy(shape)})
        before = {'services': list(services.values())}
        after = copy.deepcopy(before)
        for entry in after['services']:
            removed = {row['operation'] for row in dispositions['entries']
                       if row['service'] == entry['service'] and row['candidate_state'] == 'removed'}
            entry['document']['Operations'] = [row for row in entry['document']['Operations']
                                                if row['Name'] not in removed]
            for row in entry['document']['Operations']:
                row['AuthorizedActions'] = []
        return before, after, dispositions

    def test_exact_dispositions_report_every_loss_without_claiming_a_grant(self):
        before, after, dispositions = self.disposition_fixture()
        rows = review(before, after, {'mappings': []}, dispositions)
        self.assertEqual(len(rows), 22)
        self.assertEqual([row['status'] for row in rows].count('reviewed_deprecated_sdk_api'), 20)
        self.assertEqual([row['status'] for row in rows].count('reviewed_retired'), 1)
        self.assertEqual([row['status'] for row in rows].count('reviewed_manual_review'), 1)

    def test_new_loss_is_not_acknowledged_by_a_service_wide_exception(self):
        before, after, dispositions = self.disposition_fixture()
        old = next(s for s in before['services'] if s['service'] == 'network-firewall')
        old['document']['Operations'].append({'Name': 'NewlyAffectedProxyOperation',
            'AuthorizedActions': [{'Service': 'network-firewall', 'Name': 'NewlyAffectedProxyOperation'}]})
        row = next(row for row in review(before, after, {'mappings': []}, dispositions)
                   if row['operation'] == 'NewlyAffectedProxyOperation')
        self.assertEqual(row['status'], 'unresolved')

    def test_dispositions_require_exact_transition_context_and_retained_scopes(self):
        before, after, dispositions = self.disposition_fixture()
        nf = next(s for s in after['services'] if s['service'] == 'network-firewall')
        nf['document']['Operations'].append({'Name': 'CreateProxy', 'AuthorizedActions': []})
        rows = review(before, after, {'mappings': []}, dispositions)
        self.assertEqual(next(r for r in rows if r['operation'] == 'CreateProxy')['status'], 'unresolved')
        before, after, dispositions = self.disposition_fixture()
        old = next(s for s in before['services'] if s['service'] == 'cloudwatch')
        old['document']['Operations'][0]['AuthorizedActions'][0]['Context'] = None
        rows = review(before, after, {'mappings': []}, dispositions)
        self.assertEqual(next(r for r in rows if r['operation'] == 'CreateAlert')['status'], 'unresolved')
        for category, field, value in [('Resources', 'ARNFormats', ['*']),
                                       ('Actions', 'Resources', [{'Name': 'unreviewed'}]),
                                       ('Actions', 'ActionConditionKeys', ['new:Condition'])]:
            before, after, dispositions = self.disposition_fixture()
            for service in after['services']:
                if service['document'][category]:
                    service['document'][category][0][field] = value
                    break
            rows = review(before, after, {'mappings': []}, dispositions)
            self.assertTrue(all(row['status'] == 'unresolved' for row in rows))

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
