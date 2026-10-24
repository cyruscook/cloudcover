#!/usr/bin/env python3
"""Explicitly acquire a reviewable SAR snapshot; ordinary Cargo builds are offline."""
import concurrent.futures
import hashlib
import json
import pathlib
import subprocess
import urllib.request
import datetime
import os
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
URL = 'https://servicereference.us-east-1.amazonaws.com/'

def fetch(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()

def canonical(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()

def main():
    index_bytes = fetch(URL)
    index = json.loads(index_bytes)
    def document(entry):
        raw = fetch(entry['url'])
        return {'service': entry['service'], 'url': entry['url'],
                'modified': entry['modified'], 'source_sha256': hashlib.sha256(raw).hexdigest(),
                'document': json.loads(raw)}
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        documents = sorted(pool.map(document, index), key=lambda d: d['service'])
    if fetch(URL) != index_bytes:
        raise RuntimeError('SAR index changed during acquisition; retry for a consistent index')
    content = canonical({'schema_version': 1, 'services': documents})
    manifest = canonical({'schema_version': 1, 'source': URL,
        'retrieved_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'index_sha256': hashlib.sha256(index_bytes).hexdigest(),
        'catalog_sha256': hashlib.sha256(content).hexdigest(),
        'refresh_script_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        'extractor_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'service_count': len(documents)})
    # Both files are staged before publication. A partial replacement fails the build checksum.
    directory = ROOT / 'crates/clouds/cloudcover-aws/data/sar'
    directory.mkdir(parents=True, exist_ok=True)
    for name, data in [('catalog.json', content), ('manifest.json', manifest)]:
        with tempfile.NamedTemporaryFile(dir=directory, delete=False) as output:
            output.write(data)
            temporary = output.name
        os.replace(temporary, directory / name)
    print(f'pinned {len(documents)} services, {len(content)} bytes, sha256={hashlib.sha256(content).hexdigest()}')

if __name__ == '__main__':
    main()
