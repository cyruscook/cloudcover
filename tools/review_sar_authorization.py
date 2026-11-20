#!/usr/bin/env python3
"""Read-only gate for loss of previously mapped direct SAR authorization.

Both inputs must be checksum-paired catalog/manifest directories. Report every
raw transition, including reviewed compiler bridges and blocking dispositions;
reject any without an exact reviewed disposition. Acknowledgment never grants
permissions or establishes complete AWS coverage.
"""
import argparse
import hashlib
import json
from pathlib import Path

from refresh_sar import definitions, read_snapshot

ROOT = Path(__file__).resolve().parents[1]


def operations(catalog):
    result = {}
    for key, op in definitions(catalog, 'Operations').items():
        links = op.get('AuthorizedActions', [])
        if not isinstance(links, list):
            raise ValueError(f'AuthorizedActions must be an array: {key}')
        actions = []
        for action in links:
            if (not isinstance(action, dict)
                    or any(not isinstance(action.get(field), str) or not action[field]
                           for field in ['Service', 'Name'])):
                raise ValueError(f'invalid authorized action: {key}')
            actions.append((action['Service'], action['Name']))
        # SAR may repeat an action with different Context values. This gate
        # compares direct action identity only, just like the compiler index.
        result[key] = set(actions)
    return result


def context_shape(links, service_field, name_field, context_field):
    return sorted((action[service_field], action[name_field],
                   json.dumps(action[context_field], sort_keys=True, separators=(',', ':'))
                   if context_field in action else '<absent>') for action in links)


def scopes_match(catalog, dispositions):
    for category, field, expected in [
            ('Actions', 'action_scopes', ('Resources', 'ActionConditionKeys')),
            ('Resources', 'resource_scopes', ('ARNFormats', 'ConditionKeys'))]:
        rows = definitions(catalog, category)
        for key, shape in dispositions.get(field, {}).items():
            row = rows.get(tuple(key.split(':', 1)))
            if row is None or {name: row.get(name, []) for name in expected} != shape:
                return False
    return True


def bridge_scope_matches(catalog, mapping):
    scope = mapping.get('required_scope')
    if scope is None:
        return True
    service = mapping['service']
    action = definitions(catalog, 'Actions').get((service, mapping['operation']))
    resource = definitions(catalog, 'Resources').get((service, scope['resource_type']))
    return (action is not None and resource is not None
            and action.get('Resources') == [{'Name': scope['resource_type']}]
            and action.get('ActionConditionKeys', []) == []
            and resource.get('ARNFormats') == [scope['arn_format']]
            and resource.get('ConditionKeys') == ['aws:ResourceTag/${TagKey}'])


def review(baseline, candidate, supplement, dispositions=None):
    before, after = operations(baseline), operations(candidate)
    baseline_rows = definitions(baseline, 'Operations')
    bridges = {(row['service'], row['operation']):
               {(action['service'], action['name']) for action in row['authorized_actions']}
               for row in supplement['mappings']}
    bridge_shapes = {(row['service'], row['operation']):
                     context_shape(row['authorized_actions'], 'service', 'name', 'context')
                     for row in supplement['mappings']}
    actions = set(definitions(candidate, 'Actions'))
    dispositions = dispositions or {}
    exceptions = {(row['service'], row['operation']): row
                  for row in dispositions.get('entries', [])}
    if len(exceptions) != len(dispositions.get('entries', [])):
        raise ValueError('duplicate authorization disposition')
    for row in exceptions.values():
        if (row['kind'], row['candidate_state']) not in {
                ('manual_review', 'empty'), ('retired', 'empty'),
                ('deprecated_sdk_api', 'removed')}:
            raise ValueError('invalid authorization disposition transition')
        for link in row['authorized_actions']:
            if f"{link['Service']}:{link['Name']}" not in dispositions.get('action_scopes', {}):
                raise ValueError('missing reviewed action scope')
    scope_ok = scopes_match(baseline, dispositions) and scopes_match(candidate, dispositions)
    mapping_rows = {(row['service'], row['operation']): row for row in supplement['mappings']}
    rows = []
    for key, old in sorted(before.items()):
        if not old or after.get(key):
            continue
        status = 'unresolved'
        if (old == bridges.get(key) and old <= actions and key in after
                and context_shape(baseline_rows[key]['AuthorizedActions'], 'Service', 'Name', 'Context')
                == bridge_shapes.get(key)
                and bridge_scope_matches(baseline, mapping_rows[key])
                and bridge_scope_matches(candidate, mapping_rows[key])):
            status = 'reviewed_compiler_bridge'
        exception = exceptions.get(key)
        if (exception and scope_ok
                and context_shape(baseline_rows[key]['AuthorizedActions'], 'Service', 'Name', 'Context')
                == context_shape(exception['authorized_actions'], 'Service', 'Name', 'Context')
                and ((exception['candidate_state'] == 'empty' and key in after)
                     or (exception['candidate_state'] == 'removed' and key not in after))):
            status = 'reviewed_' + exception['kind']
        rows.append({'service': key[0], 'operation': key[1],
             'previous_actions': [f'{s}:{n}' for s, n in sorted(old)],
             'status': status, 'candidate_operation_present': key in after})
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline_dir', type=Path)
    parser.add_argument('candidate_dir', type=Path)
    args = parser.parse_args()
    baseline_bytes, baseline = read_snapshot(args.baseline_dir)
    candidate_bytes, candidate = read_snapshot(args.candidate_dir)
    supplement = json.loads((ROOT / 'crates/clouds/cloudcover-aws/data/authorization.json').read_text())
    dispositions = json.loads((ROOT / 'crates/clouds/cloudcover-aws/data/authorization-dispositions.json').read_text())
    rows = review(baseline, candidate, supplement, dispositions)
    print(json.dumps({'baseline_sha256': hashlib.sha256(baseline_bytes).hexdigest(),
                      'candidate_sha256': hashlib.sha256(candidate_bytes).hexdigest(),
                      'mapped_to_empty_or_missing': rows}, indent=2))
    return 1 if any(row['status'] == 'unresolved' for row in rows) else 0


if __name__ == '__main__':
    raise SystemExit(main())
