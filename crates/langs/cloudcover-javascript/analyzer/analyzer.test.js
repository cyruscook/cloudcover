'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const test = require('node:test');

const analyzer = fs.readFileSync(path.join(__dirname, 'analyzer.js'), 'utf8');
const typescript = path.resolve(__dirname, '../../../../site/node_modules/typescript');

function fixture(files, { sdk = true } = {}) {
  assert.ok(fs.existsSync(typescript), `project TypeScript dependency is missing at ${typescript}`);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cloudcover-javascript-'));
  fs.mkdirSync(path.join(root, 'node_modules'), { recursive: true });
  fs.symlinkSync(typescript, path.join(root, 'node_modules', 'typescript'), 'dir');

  if (sdk) {
    const packageRoot = path.join(root, 'node_modules', '@aws-sdk', 'client-test');
    fs.mkdirSync(packageRoot, { recursive: true });
    fs.writeFileSync(path.join(packageRoot, 'package.json'), JSON.stringify({
      name: '@aws-sdk/client-test',
      version: '3.0.0',
      types: 'index.d.ts',
      main: 'index.js',
    }));
    fs.writeFileSync(path.join(packageRoot, 'index.js'), '');
    fs.writeFileSync(path.join(packageRoot, 'index.d.ts'), `
      export declare class GetObjectCommand { constructor(input: unknown); }
      export declare class PutObjectCommand { constructor(input: unknown); }
      export declare class S3Client {
        constructor(config: unknown);
        send(command: unknown): Promise<unknown>;
        destroy(): void;
      }
      export interface S3 {
        getObject(input: unknown): Promise<unknown>;
        putObject(input: unknown): Promise<unknown>;
      }
      export declare class S3 extends S3Client implements S3 {}
      export declare const paginateListObjects: (config: unknown, input: unknown) => Promise<unknown>;
      export declare const waitForBucketExists: (config: unknown, input: unknown) => Promise<unknown>;
      export declare const waitUntilBucketExists: (config: unknown, input: unknown) => Promise<unknown>;
      export interface TypeOnly {}
    `);
  }

  for (const [name, contents] of Object.entries(files)) {
    const target = path.join(root, name);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, contents);
  }
  return root;
}

function analyze(root) {
  const result = spawnSync(process.execPath, ['-e', analyzer, root], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}

function methodNames(result) {
  return result.methods.map(({ receiver, name }) => receiver ? `${receiver}.${name}` : name);
}

test('analyzes JavaScript included by an empty jsconfig', (t) => {
  const root = fixture({
    'jsconfig.json': '{}',
    'main.js': 'import { GetObjectCommand } from "@aws-sdk/client-test"; new GetObjectCommand({});',
  });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  assert.deepEqual(methodNames(analyze(root)), ['GetObjectCommand']);
});

test('follows imported project sources without scanning excluded files', (t) => {
  const root = fixture({
    'tsconfig.json': JSON.stringify({ compilerOptions: { module: 'NodeNext' }, files: ['main.ts'] }),
    'main.ts': 'import "./worker.js";',
    'worker.ts': 'import { GetObjectCommand } from "@aws-sdk/client-test"; new GetObjectCommand({});',
    'excluded.ts': 'import { PutObjectCommand } from "@aws-sdk/client-test"; new PutObjectCommand({});',
  });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  assert.deepEqual(methodNames(analyze(root)), ['GetObjectCommand']);
});

test('recognizes legacy waitFor helpers from SDK declarations', (t) => {
  const root = fixture({
    'main.ts': 'import { waitForBucketExists } from "@aws-sdk/client-test"; waitForBucketExists({}, {});',
  });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  assert.deepEqual(methodNames(analyze(root)), ['waitForBucketExists']);
});

test('does not report client lifecycle methods as aggregate operations', (t) => {
  const root = fixture({
    'main.ts': `
      import { S3Client, GetObjectCommand } from "@aws-sdk/client-test";
      const client = new S3Client({});
      client.send(new GetObjectCommand({}));
      client.destroy();
    `,
  });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  assert.deepEqual(methodNames(analyze(root)), ['GetObjectCommand']);
});

test('fails closed for unresolved runtime AWS modules and missing runtime exports', (t) => {
  const unresolved = fixture({
    'main.ts': 'import { MissingCommand } from "@aws-sdk/client-missing"; new MissingCommand({});',
  });
  const missingExport = fixture({
    'main.ts': 'import { MissingCommand } from "@aws-sdk/client-test"; new MissingCommand({});',
  });
  t.after(() => fs.rmSync(unresolved, { recursive: true, force: true }));
  t.after(() => fs.rmSync(missingExport, { recursive: true, force: true }));

  const unresolvedResult = analyze(unresolved);
  assert.equal(typeof unresolvedResult.error, 'string');
  assert.match(unresolvedResult.error, /@aws-sdk\/client-missing/);
  assert.match(unresolvedResult.error, /main\.ts/);

  const missingExportResult = analyze(missingExport);
  assert.equal(typeof missingExportResult.error, 'string');
  assert.match(missingExportResult.error, /MissingCommand/);
  assert.match(missingExportResult.error, /main\.ts/);
});

test('preserves named, namespace, aliased, and CommonJS SDK operations', (t) => {
  const root = fixture({
    'named.ts': `
      import { S3 as NamedS3, GetObjectCommand as AliasedCommand } from "@aws-sdk/client-test";
      import * as sdk from "@aws-sdk/client-test";
      new NamedS3({}).getObject({});
      new AliasedCommand({});
      sdk.waitUntilBucketExists({}, {});
      sdk.paginateListObjects({}, {});
    `,
    'common.cjs': `
      const { S3: CommonS3 } = require("@aws-sdk/client-test");
      new CommonS3({}).putObject({});
    `,
  });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  assert.deepEqual(new Set(methodNames(analyze(root))), new Set([
    'GetObjectCommand',
    'paginateListObjects',
    'S3.getObject',
    'S3.putObject',
    'waitUntilBucketExists',
  ]));
});

test('ignores unresolved type-only AWS imports', (t) => {
  const root = fixture({
    'main.ts': 'import type { Missing } from "@aws-sdk/client-missing"; const value: Missing | undefined = undefined;',
  });
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  assert.deepEqual(analyze(root), { methods: [], modules: [] });
});
