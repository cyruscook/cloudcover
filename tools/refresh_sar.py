#!/usr/bin/env python3
"""Explicitly acquire a reviewable SAR snapshot; ordinary Cargo builds are offline."""
import argparse
import concurrent.futures
import datetime
import hashlib
import json
import os
import pathlib
import re
import subprocess
import tempfile
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
URL = 'https://servicereference.us-east-1.amazonaws.com/'
DATA_DIR = ROOT / 'crates/clouds/cloudcover-aws/data/sar'


def fetch(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()


def canonical(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()


def sha256(content):
    return hashlib.sha256(content).hexdigest()


def validate_catalog(catalog):
    if (not isinstance(catalog, dict) or set(catalog) != {'schema_version', 'services'}
            or type(catalog['schema_version']) is not int or catalog['schema_version'] != 1
            or not isinstance(catalog['services'], list) or not catalog['services']):
        raise ValueError('invalid SAR catalog schema or empty service list')
    previous = None
    for entry in catalog['services']:
        if (not isinstance(entry, dict)
                or set(entry) != {'service', 'url', 'modified', 'source_sha256', 'document'}
                or not isinstance(entry['service'], str) or not entry['service']
                or (previous is not None and previous >= entry['service'])
                or not isinstance(entry['url'], str) or not entry['url']
                or type(entry['modified']) is not int or entry['modified'] < 0
                or not isinstance(entry['source_sha256'], str)
                or not re.fullmatch(r'[0-9a-fA-F]{64}', entry['source_sha256'])
                or not isinstance(entry['document'], dict)
                or entry['document'].get('Name') != entry['service']):
            raise ValueError('invalid, duplicate or unsorted SAR service')
        previous = entry['service']


def read_snapshot(directory):
    catalog_path = directory / 'catalog.json'
    manifest_path = directory / 'manifest.json'
    if not catalog_path.exists() and not manifest_path.exists():
        return None
    content = catalog_path.read_bytes()
    manifest = json.loads(manifest_path.read_bytes())
    catalog = json.loads(content)
    validate_catalog(catalog)
    fields = {'schema_version', 'source', 'retrieved_at', 'index_sha256',
              'catalog_sha256', 'refresh_script_sha256', 'extractor_revision', 'service_count'}
    if (not isinstance(manifest, dict) or set(manifest) != fields
            or type(manifest['schema_version']) is not int or manifest['schema_version'] != 1
            or manifest['source'] != URL or manifest['catalog_sha256'] != sha256(content)
            or type(manifest['service_count']) is not int
            or manifest['service_count'] != len(catalog['services'])
            or any(not isinstance(manifest[key], str) or not manifest[key]
                   for key in ['retrieved_at', 'extractor_revision'])
            or any(not isinstance(manifest[key], str)
                   or not re.fullmatch(r'[0-9a-fA-F]{64}', manifest[key])
                   for key in ['index_sha256', 'refresh_script_sha256'])):
        raise ValueError('invalid SAR manifest schema, provenance or checksum')
    return content, catalog


def collect_catalog():
    index_bytes = fetch(URL)
    index = json.loads(index_bytes)

    def document(entry):
        raw = fetch(entry['url'])
        return {'service': entry['service'], 'url': entry['url'],
                'modified': entry['modified'], 'source_sha256': sha256(raw),
                'document': json.loads(raw)}

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        documents = sorted(pool.map(document, index), key=lambda entry: entry['service'])
    if fetch(URL) != index_bytes:
        raise RuntimeError('SAR index changed during acquisition; retry for a consistent index')
    catalog = {'schema_version': 1, 'services': documents}
    validate_catalog(catalog)
    return index_bytes, catalog


def definitions(catalog, field):
    result = {}
    if catalog:
        for entry in catalog['services']:
            rows = entry['document'].get(field, [])
            if not isinstance(rows, list):
                raise ValueError(f'{field} must be an array')
            for row in rows:
                if not isinstance(row, dict) or not isinstance(row.get('Name'), str) or not row['Name']:
                    raise ValueError(f'invalid {field} definition')
                key = entry['service'], row['Name']
                if key in result:
                    raise ValueError(f'duplicate {field} identity: {key}')
                result[key] = row
    return result


def summary(old, new):
    old_content, old_catalog = old if old else (b'', None)
    old_services = {entry['service']: entry['document'] for entry in old_catalog['services']} if old_catalog else {}
    new_services = {entry['service']: entry['document'] for entry in new['services']}
    lines = ['Refresh the pinned AWS Service Authorization Reference.', '',
             f'Catalog SHA-256: `{sha256(old_content) if old else "none"}` → `{sha256(canonical(new))}`.', '',
             '| Definition | Added | Removed | Changed |', '| --- | ---: | ---: | ---: |']
    for label, before, after in [('Services', old_services, new_services)] + [
            (label, definitions(old_catalog, field), definitions(new, field))
            for label, field in [('IAM actions', 'Actions'), ('API operations', 'Operations'),
                                 ('Resource types', 'Resources'), ('Condition keys', 'ConditionKeys')]]:
        added, removed = after.keys() - before.keys(), before.keys() - after.keys()
        changed = sum(before[key] != after[key] for key in before.keys() & after.keys())
        lines.append(f'| {label} | {len(added)} | {len(removed)} | {changed} |')
    changed_services = sorted(key for key in new_services.keys() | old_services.keys()
                              if new_services.get(key) != old_services.get(key))
    names = ', '.join('`' + name.replace('`', '\\`') + '`' for name in changed_services[:12])
    lines.extend(['', f'Affected services: {names or "none (catalog source/provenance bytes changed)"}'
                  + (f' and {len(changed_services) - 12} more.' if len(changed_services) > 12 else '.'), '',
                  'Review policy/FAS tests and these input changes before merging. Normal builds remain pinned and offline.', '',
                  'This update uses GITHUB_TOKEN. GitHub may require a repository writer to select **Approve workflows to run** for the resulting PR workflows; no additional credentials or automatic merge are configured.', ''])
    return '\n'.join(lines)


def refresh(directory=DATA_DIR, previous_dir=None, summary_path=None):
    current = read_snapshot(directory)
    previous = read_snapshot(previous_dir) if previous_dir else None
    if previous_dir and previous is None:
        raise ValueError('previous SAR snapshot is missing')
    index_bytes, catalog = collect_catalog()
    content = canonical(catalog)
    # Keep provenance for unchanged inputs, including an identical pending PR.
    # Do not churn retrieved_at, extractor_revision or refresh_script_sha256 daily.
    if any(snapshot and snapshot[0] == content for snapshot in [current, previous]):
        print(f'SAR catalog unchanged: sha256={sha256(content)}; preserved existing provenance')
        return False
    report = summary(current, catalog)
    manifest = canonical({'schema_version': 1, 'source': URL,
        'retrieved_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'index_sha256': sha256(index_bytes), 'catalog_sha256': sha256(content),
        'refresh_script_sha256': sha256(pathlib.Path(__file__).read_bytes()),
        'extractor_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'service_count': len(catalog['services'])})
    directory.mkdir(parents=True, exist_ok=True)
    # Stage both files. A partial replacement is rejected by the build checksum.
    for name, data in [('catalog.json', content), ('manifest.json', manifest)]:
        with tempfile.NamedTemporaryFile(dir=directory, delete=False) as output:
            output.write(data)
            temporary = output.name
        os.replace(temporary, directory / name)
    if summary_path:
        summary_path.write_text(report)
    print(f'pinned {len(catalog["services"])} services, {len(content)} bytes, sha256={sha256(content)}')
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--previous-dir', type=pathlib.Path,
                        help='Validated snapshot from the currently open automation PR (data only)')
    parser.add_argument('--summary', type=pathlib.Path, help='Write a concise update PR body when inputs change')
    args = parser.parse_args()
    refresh(previous_dir=args.previous_dir, summary_path=args.summary)


if __name__ == '__main__':
    main()
