#!/usr/bin/env node
/**
 * Generate exact AWS SDK for JavaScript v3 operation mappings from version-matched
 * AWS SDK Git source, with npm tarballs as an explicit fallback for releases
 * unavailable in the repository history.
 */
import { gunzipSync } from "node:zlib";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { inspectFiles, compareMappings } from "./mappings.mjs";
import { openSourceRepository } from "./source-repository.mjs";

const REGISTRY = "https://registry.npmjs.org";

function usage() {
  return `Usage: node generate.mjs [--latest|--all] [--package NAME@VERSION ...] [--output FILE]\n\n` +
    `--latest generates the latest published version of every current @aws-sdk/client-* package using AWS SDK Git source.\n` +
    `--all generates every stable published version of every current @aws-sdk/client-* package using AWS SDK Git source.\n` +
    `--package accepts an exact package/version, for example @aws-sdk/client-s3@3.XXX.X.`;
}

function parseArguments(argv) {
  const packages = [];
  let latest = false;
  let all = false;
  let output = resolve(import.meta.dirname, "../data/mappings.json");
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--latest") {
      latest = true;
    } else if (argument === "--all") {
      all = true;
    } else if (argument === "--package") {
      const value = argv[++index];
      if (!value) throw new Error("--package requires NAME@VERSION");
      const separator = value.lastIndexOf("@");
      if (separator <= 0 || separator === value.length - 1) {
        throw new Error(`--package must be NAME@VERSION, got ${value}`);
      }
      packages.push({ package: value.slice(0, separator), version: value.slice(separator + 1) });
    } else if (argument === "--output") {
      output = resolve(argv[++index] ?? "");
      if (!output) throw new Error("--output requires a file path");
    } else if (argument === "--help" || argument === "-h") {
      process.stdout.write(`${usage()}\n`);
      process.exit(0);
    } else {
      throw new Error(`unknown argument ${argument}`);
    }
  }
  if (latest && all) throw new Error("--latest and --all cannot be combined");
  if (!latest && !all && packages.length === 0) {
    throw new Error("select --latest, --all, or at least one --package");
  }
  return { latest, all, output, packages };
}

async function json(url) {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`);
  return response.json();
}


async function selectedClientPackages(selection, sourceRepository) {
  const names = await sourceRepository.packageNames();
  const packages = [];
  for (let index = 0; index < names.length; index += 8) {
    const batch = names.slice(index, index + 8);
    packages.push(...(await Promise.all(batch.map(async (packageName) => {
      const metadata = await json(`${REGISTRY}/${encodeURIComponent(packageName)}`);
      const versions = metadata.versions ?? {};
      if (selection === "latest") {
        const version = metadata["dist-tags"]?.latest;
        const tarball = typeof version === "string" ? versions[version]?.dist?.tarball : undefined;
        return typeof version === "string" ? [{ package: packageName, version, tarball }] : [];
      }
      return Object.keys(versions)
        .filter(isStableVersion)
        .sort(compareVersions)
        .map((version) => ({ package: packageName, version, tarball: versions[version]?.dist?.tarball }));
    }))).flat());
  }
  return packages.sort(comparePackageVersions);
}

function isStableVersion(version) {
  return /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(version);
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1) {
    if (leftParts[index] !== rightParts[index]) return leftParts[index] - rightParts[index];
  }
  return 0;
}

function comparePackageVersions(left, right) {
  return left.package.localeCompare(right.package) || compareVersions(left.version, right.version);
}

function untar(gzip, prefix = "package/") {
  const bytes = gunzipSync(gzip);
  const files = new Map();
  for (let offset = 0; offset + 512 <= bytes.length;) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const name = readTarString(header, 0, 100);
    const headerPrefix = readTarString(header, 345, 155);
    const sizeText = readTarString(header, 124, 12).trim();
    const size = Number.parseInt(sizeText || "0", 8);
    if (!Number.isSafeInteger(size) || size < 0) throw new Error(`invalid tar entry size for ${name}`);
    const fullName = headerPrefix ? `${headerPrefix}/${name}` : name;
    const contentStart = offset + 512;
    const contentEnd = contentStart + size;
    if (contentEnd > bytes.length) throw new Error(`truncated tar entry ${fullName}`);
    if (header[156] === 48 && fullName.startsWith(prefix)) {
      files.set(fullName.slice(prefix.length), Buffer.from(bytes.subarray(contentStart, contentEnd)).toString("utf8"));
    }
    offset = contentStart + Math.ceil(size / 512) * 512;
  }
  return files;
}

function readTarString(bytes, offset, length) {
  const end = bytes.subarray(offset, offset + length).indexOf(0);
  return Buffer.from(bytes.subarray(offset, end < 0 ? offset + length : offset + end)).toString("utf8");
}

async function packageFiles(packageVersion) {
  let tarball = packageVersion.tarball;
  if (!tarball) {
    const metadata = await json(`${REGISTRY}/${encodeURIComponent(packageVersion.package)}/${encodeURIComponent(packageVersion.version)}`);
    if (metadata.version !== packageVersion.version) {
      throw new Error(`${packageVersion.package}@${packageVersion.version}: registry returned ${metadata.version}`);
    }
    tarball = metadata.dist?.tarball;
  }
  if (!tarball) throw new Error(`${packageVersion.package}@${packageVersion.version}: missing tarball`);
  const response = await fetch(tarball);
  if (!response.ok) throw new Error(`${packageVersion.package}@${packageVersion.version}: tarball ${response.status} ${response.statusText}`);
  return untar(Buffer.from(await response.arrayBuffer()));
}


async function inspectPackage(packageVersion, sourceRepository, mappingCache) {
  const source = sourceRepository ? await sourceRepository.snapshot(packageVersion) : null;
  if (source) {
    const cached = mappingCache.get(source.key);
    if (cached) return { ...packageVersion, mappings: cached };
    const snapshot = inspectFiles(packageVersion, await source.files());
    mappingCache.set(source.key, snapshot.mappings);
    return snapshot;
  }
  return inspectFiles(packageVersion, await packageFiles(packageVersion));
}
function mappingKey(mapping) {
  return JSON.stringify([mapping.receiver, mapping.method]);
}

function packageHistory(packageName, snapshots) {
  const state = new Map();
  const releases = [];
  for (const snapshot of snapshots.sort(comparePackageVersions)) {
    const next = new Map(snapshot.mappings.map((mapping) => [mappingKey(mapping), mapping]));
    const remove = [...state]
      .filter(([key]) => !next.has(key))
      .map(([, mapping]) => [mapping.receiver, mapping.method]);
    const upsert = [...next]
      .filter(([key, mapping]) => JSON.stringify(state.get(key)) !== JSON.stringify(mapping))
      .map(([, mapping]) => mapping);
    remove.sort((left, right) => String(left[0]).localeCompare(String(right[0])) || left[1].localeCompare(right[1]));
    upsert.sort(compareMappings);
    releases.push({ version: snapshot.version, remove, upsert });
    state.clear();
    for (const [key, mapping] of next) state.set(key, mapping);
  }
  return { package: packageName, releases };
}

async function run() {
  const options = parseArguments(process.argv.slice(2));
  const sourceRepository = options.latest || options.all || process.env.CLOUDCOVER_AWS_SDK_REPOSITORY
    ? await openSourceRepository({
      repository: process.env.CLOUDCOVER_AWS_SDK_REPOSITORY,
      cacheDir: process.env.CLOUDCOVER_AWS_SDK_CACHE ??
        resolve(process.env.HOME ?? import.meta.dirname, ".cache", "cloudcover"),
    })
    : null;
  try {
    const requested = new Map(options.packages.map((item) => [`${item.package}@${item.version}`, item]));
    if (options.latest || options.all) {
      const selected = await selectedClientPackages(options.all ? "all" : "latest", sourceRepository);
      for (const item of selected) requested.set(`${item.package}@${item.version}`, item);
      process.stderr.write(`selected ${requested.size} exact npm package releases\n`);
    }
    const packages = [...requested.values()].sort(comparePackageVersions);
    const snapshots = [];
    const mappingCache = new Map();
    const concurrency = 64;
    for (let index = 0; index < packages.length; index += concurrency) {
      const batch = packages.slice(index, index + concurrency);
      const outcomes = await Promise.allSettled(batch.map((item) => inspectPackage(item, sourceRepository, mappingCache)));
      for (const outcome of outcomes) {
        if (outcome.status === "fulfilled") {
          snapshots.push(outcome.value);
        } else if (options.latest || (options.all && /tarball 404\b/.test(String(outcome.reason?.message ?? outcome.reason)))) {
          process.stderr.write(`skipping unavailable package release: ${outcome.reason?.message ?? outcome.reason}\n`);
        } else {
          throw outcome.reason;
        }
      }
    }
    const byPackage = Map.groupBy(snapshots, (snapshot) => snapshot.package);
    const data = {
      schema_version: 2,
      provenance: {
        registry: REGISTRY,
        source: "AWS SDK Git release tags with npm tarball fallback; dist-es command builders, runtime config signingService, paginator, and waiter modules",
      },
      packages: [...byPackage].sort(([left], [right]) => left.localeCompare(right))
        .map(([packageName, releases]) => packageHistory(packageName, releases)),
    };
    await mkdir(dirname(options.output), { recursive: true });
    const temporary = `${options.output}.tmp`;
    await writeFile(temporary, `${JSON.stringify(data, null, 2)}\n`);
    await rename(temporary, options.output);
    process.stderr.write(`wrote ${snapshots.length} exact package releases across ${data.packages.length} packages\n`);
  } finally {
    await sourceRepository?.close();
  }
}

run().catch((error) => {
  process.stderr.write(`error: ${error.stack ?? error}\n`);
  process.exitCode = 1;
});
