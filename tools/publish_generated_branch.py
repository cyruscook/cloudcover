#!/usr/bin/env python3
"""Publish a generated commit without replacing identical pending PR content."""
import argparse
import subprocess


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


def publish(branch):
    ref = f'refs/heads/{branch}'
    git('check-ref-format', ref)
    listing = git('ls-remote', '--heads', 'origin', ref)
    remote_sha = listing.split()[0] if listing else ''
    if remote_sha:
        # Fetch only data; never check out or execute the pending PR's code.
        git('fetch', '--no-tags', 'origin', ref)
        fetched_sha = git('rev-parse', 'FETCH_HEAD')
        if fetched_sha != remote_sha:
            raise RuntimeError('Generated branch changed during fetch; retry from a fresh checkout')
        if git('rev-parse', 'HEAD^{tree}') == git('rev-parse', f'{remote_sha}^{{tree}}'):
            # The caller must still ensure a PR exists (a previous create may have failed).
            print(f'{branch}: identical tree; preserving existing commit {remote_sha}')
            return False
    # An explicit empty lease also protects first publication against a competing creator.
    # Compare full trees above so changes inherited from the base branch are not hidden.
    git('push', 'origin', f'HEAD:{ref}', f'--force-with-lease={ref}:{remote_sha}')
    print(f'{branch}: published generated changes')
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('branch')
    args = parser.parse_args()
    publish(args.branch)


if __name__ == '__main__':
    main()
