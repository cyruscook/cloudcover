#!/usr/bin/env python3
"""Validate SDK history contracts and reproduce auditable sidecar manifests."""
import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
SDK_ROOT = ROOT / 'crates/sdks'

def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()

def version(raw):
    if not isinstance(raw, str) or not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', raw):
        raise ValueError(f'invalid exact stable version: {raw!r}')
    return tuple(map(int, raw.split('.')))

def mapping_key(raw, javascript):
    arity = 2 if javascript else 3
    if not isinstance(raw, list) or len(raw) != arity:
        raise ValueError(f'mapping key must be an array of length {arity}')
    for position, value in enumerate(raw):
        if javascript and position == 0 and value is None:
            continue
        if not isinstance(value, str) or not value:
            raise ValueError('mapping key fields must be nonempty strings; only JS receiver may be null')
    return tuple(raw)

def api_pairs(raw, javascript):
    if not isinstance(raw, list):
        raise ValueError('API methods must be an array')
    pairs = []
    for api in raw:
        if javascript:
            if not isinstance(api, dict) or set(api) != {'service', 'name'}:
                raise ValueError('JS API methods must have service and name fields')
            api = [api['service'], api['name']]
        if not isinstance(api, list) or len(api) != 2 or any(not isinstance(x, str) or not x for x in api):
            raise ValueError('API tuple must be an array of two nonempty strings')
        pairs.append(tuple(api))
    return tuple(sorted(set(pairs)))

def apply(release, state, javascript=False, allow_empty=False):
    removes, upserts = release['remove'], release['upsert']
    if not isinstance(removes, list) or not isinstance(upserts, list):
        raise ValueError('remove and upsert must be arrays')
    keys = set()
    for raw in removes:
        key = mapping_key(raw, javascript)
        if key in keys or key not in state:
            raise ValueError(f'duplicate or missing removal: {key}')
        keys.add(key)
    additions = {}
    for row in upserts:
        if javascript:
            if not isinstance(row, dict) or set(row) != {'receiver', 'method', 'api_methods'}:
                raise ValueError('JS mapping row must have receiver, method and api_methods fields')
            key = mapping_key([row['receiver'], row['method']], True)
            apis = api_pairs(row['api_methods'], True)
        else:
            if not isinstance(row, list) or len(row) != 4:
                raise ValueError('mapping row must be an array of length four')
            key = mapping_key(row[:3], False)
            apis = api_pairs(row[3], False)
        if key in keys or key in additions:
            raise ValueError(f'duplicate or conflicting key: {key}')
        if not apis and not allow_empty:
            raise ValueError(f'empty API list: {key}')
        if state.get(key) == apis:
            raise ValueError(f'redundant upsert: {key}')
        additions[key] = apis
    for key in keys: del state[key]
    state.update(additions)

def manifest(directory):
    javascript = directory.name.endswith('js-v3')
    terraform = 'terraform' in directory.name
    files = sorted((directory / 'data').glob('*.json'))
    docs = [json.loads(path.read_bytes()) for path in files]
    if javascript:
        if docs[0].get('schema_version', 1) not in (1, 2): raise ValueError('unsupported JS schema')
        groups = [(g['package'], g['releases']) for g in docs[0]['packages']]
    elif terraform:
        groups = [('terraform-provider-aws', sorted(docs, key=lambda d: version(d['provider_version'])))]
    else:
        groups = [(d['module_path'], d['releases']) for d in docs]
    identities = set(); release_count = 0; snapshots = set(); stream = hashlib.sha256(); no_calls = 0; unique_rows = set(); api_lists = set()
    for identity, releases in groups:
        if not identity or identity in identities or not releases: raise ValueError('duplicate/empty SDK identity or history')
        identities.add(identity); previous = None; state = {}; snapshot_hash = digest([])
        for release in releases:
            raw = release.get('version', release.get('module_version', release.get('provider_version')))
            current = version(raw)
            if previous is not None and previous >= current: raise ValueError('versions must increase strictly')
            previous = current; release_count += 1
            unsupported = release.get('unsupported_reason')
            if unsupported:
                if unsupported != 'aws_sdk_go_v1' or release['remove'] or release['upsert']: raise ValueError('invalid unsupported release')
                status = 'unsupported'
            else:
                apply(release, state, javascript, terraform)
                if release['remove'] or release['upsert']:
                    snapshot_hash = digest(sorted(((key, apis) for key, apis in state.items()), key=lambda item: tuple((x is not None, x or "") for x in item[0])))
                if release['remove'] or release['upsert']:
                    unique_rows.update((identity, key, apis) for key, apis in state.items())
                    api_lists.update(state.values())
                status = 'legacy_unverified'  # Historical extraction completeness was not recorded.
                no_calls += sum(not apis for apis in state.values()) if terraform else 0
            snapshots.add(snapshot_hash)
            stream.update((json.dumps([identity, raw, status, snapshot_hash], separators=(',', ':'))+'\n').encode())
    sources = {str(p.relative_to(directory)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
    extractor_files = sorted([p for p in (directory/'generator').rglob('*') if p.is_file() and 'node_modules' not in p.parts] + list((directory/'src/bin').glob('*.rs')) + ([directory/'src/data.rs'] if (directory/'src/data.rs').exists() else []))
    extractors = {str(p.relative_to(directory)): hashlib.sha256(p.read_bytes()).hexdigest() for p in extractor_files}
    return {'schema_version': 1, 'sdk_identity': directory.name, 'sources': sources,
        'extractor_sources': extractors, 'source_identity': 'legacy: exact upstream release names retained in histories',
        'source_content_digest': None, 'extraction_options': None,
        'coverage_status': 'legacy_unverified',
        'diagnostics': ['Historical upstream commit/content digests and extraction options are unavailable; no completeness claim is inferred.'],
        'release_count': release_count, 'unique_mapping_rows': len(unique_rows), 'unique_api_lists': len(api_lists), 'distinct_snapshot_hashes': sorted(snapshots),
        'release_snapshot_index_sha256': stream.hexdigest(),
        'known_no_call_memberships': no_calls,
        'dependency_artifact_digests': {}}

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--write', action='store_true'); args = parser.parse_args()
    results = {d.name: manifest(d) for d in sorted(SDK_ROOT.iterdir()) if (d/'data').is_dir()}
    tf = results['cloudcover-terraform-provider-aws']
    tf['dependency_artifact_digests'] = {key: digest(value['sources']) for key, value in results.items() if 'go-v' in key}
    tf['diagnostics'].append('Dependency digests identify the migrated SDK inventory; historical per-release dependency selection was not recorded.')
    for name, value in results.items():
        path = SDK_ROOT/name/'metadata-manifest.json'
        output = json.dumps(value, sort_keys=True, indent=2)+'\n'
        if args.write: path.write_text(output)
        elif not path.exists() or path.read_text() != output: raise SystemExit(f'{path}: manifest is stale; run python3 tools/metadata_contracts.py --write')
        print(f'{name}: {value["release_count"]} releases verified; snapshot index {value["release_snapshot_index_sha256"]}')

if __name__ == '__main__': main()
