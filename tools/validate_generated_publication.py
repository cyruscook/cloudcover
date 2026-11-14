#!/usr/bin/env python3
"""Fail closed unless an updater publishes only its own regular JSON data.

This is an accidental-change guard, not a sandbox for hostile generator code.
GitHub's environment branch policy and rulesets remain the security boundary.
"""
import argparse
import json
import re
import subprocess

SDK_BRANCHES = {
    'automation/aws-sdk-go-v1-data': 'cloudcover-aws-sdk-go-v1',
    'automation/aws-sdk-go-v2-data': 'cloudcover-aws-sdk-go-v2',
    'automation/aws-sdk-js-v3-data': 'cloudcover-aws-sdk-js-v3',
    'automation/terraform-provider-aws-data': 'cloudcover-terraform-provider-aws',
}
SAR_BRANCH = 'automation/aws-sar-catalog'
MANIFESTS = {f'crates/sdks/{crate}/metadata-manifest.json' for crate in SDK_BRANCHES.values()}


def git(*args):
    return subprocess.check_output(['git', *args])


def allowed(branch, path):
    if branch == SAR_BRANCH:
        return path in {
            'crates/clouds/cloudcover-aws/data/sar/catalog.json',
            'crates/clouds/cloudcover-aws/data/sar/manifest.json',
        }
    if branch not in SDK_BRANCHES:
        raise ValueError('Unknown updater branch')
    prefix = f'crates/sdks/{SDK_BRANCHES[branch]}/data/'
    # Current generated layouts contain direct JSON children, not arbitrary trees.
    return path in MANIFESTS or (
        path.startswith(prefix)
        and re.fullmatch(r'[a-zA-Z0-9_.-]+\.json', path[len(prefix):]) is not None
    )


def validate(branch, base, worktree=False):
    if branch not in SDK_BRANCHES and branch != SAR_BRANCH:
        raise ValueError('Unknown updater branch')
    if not re.fullmatch(r'[0-9a-f]{40}', base):
        raise ValueError('Expected a full immutable source SHA')
    git('merge-base', '--is-ancestor', base, 'HEAD')
    if worktree:
        names = git('diff', '--name-only', '-z', base).split(b'\0')
        names += git('ls-files', '--others', '--exclude-standard', '-z').split(b'\0')
    else:
        if git('status', '--porcelain', '--untracked-files=normal').strip():
            raise ValueError('Publication requires a clean worktree')
        names = git('diff', '--name-only', '-z', base, 'HEAD').split(b'\0')
    paths = sorted({name.decode('utf-8') for name in names if name})
    if not paths:
        raise ValueError('No publication changes')
    for path in paths:
        if not allowed(branch, path):
            raise ValueError(f'Unexpected generated path: {path}')
        if worktree:
            from pathlib import Path
            file = Path(path)
            if file.is_symlink() or not file.is_file() or file.stat().st_mode & 0o111:
                raise ValueError(f'Not a regular non-executable file: {path}')
            data = file.read_bytes()
        else:
            entry = git('ls-tree', '-z', 'HEAD', '--', path).split(b'\t', 1)[0]
            if not entry.startswith(b'100644 blob '):
                raise ValueError(f'Not a regular non-executable Git blob: {path}')
            data = git('show', f'HEAD:{path}')
        json.loads(data)
    print(f'Validated {len(paths)} generated JSON paths for {branch}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('branch')
    parser.add_argument('base')
    parser.add_argument('--worktree', action='store_true')
    args = parser.parse_args()
    validate(args.branch, args.base, args.worktree)


if __name__ == '__main__':
    main()
