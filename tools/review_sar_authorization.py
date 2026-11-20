#!/usr/bin/env python3
"""Read-only gate for loss of previously mapped direct SAR authorization.

Both inputs must be checksum-paired catalog/manifest directories. Report every
raw transition, including reviewed compiler bridges; reject any without an exact
reviewed bridge. This is a semantic gate, not a claim of complete AWS coverage.
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


def review(baseline, candidate, supplement):
    before, after = operations(baseline), operations(candidate)
    bridges = {(row['service'], row['operation']):
               {(action['service'], action['name']) for action in row['authorized_actions']}
               for row in supplement['mappings']}
    actions = set(definitions(candidate, 'Actions'))
    return [{'service': key[0], 'operation': key[1],
             'previous_actions': [f'{s}:{n}' for s, n in sorted(old)],
             'status': 'reviewed_compiler_bridge' if old == bridges.get(key) and old <= actions
                       and key in after else 'unresolved',
             'candidate_operation_present': key in after}
            for key, old in sorted(before.items()) if old and not after.get(key)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline_dir', type=Path)
    parser.add_argument('candidate_dir', type=Path)
    args = parser.parse_args()
    baseline_bytes, baseline = read_snapshot(args.baseline_dir)
    candidate_bytes, candidate = read_snapshot(args.candidate_dir)
    supplement = json.loads((ROOT / 'crates/clouds/cloudcover-aws/data/authorization.json').read_text())
    rows = review(baseline, candidate, supplement)
    print(json.dumps({'baseline_sha256': hashlib.sha256(baseline_bytes).hexdigest(),
                      'candidate_sha256': hashlib.sha256(candidate_bytes).hexdigest(),
                      'mapped_to_empty_or_missing': rows}, indent=2))
    return 1 if any(row['status'] == 'unresolved' for row in rows) else 0


if __name__ == '__main__':
    raise SystemExit(main())
