#!/usr/bin/env node
/**
 * Generate exact AWS SDK for JavaScript v3 operation mappings from published npm
 * tarballs. The generator reads source text only; it never imports SDK code.
 */
import { gunzipSync } from "node:zlib";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const REGISTRY = "https://registry.npmjs.org";
const CLIENT_PACKAGE = /^@aws-sdk\/client-[a-z0-9-]+$/;

function usage() {
  return `Usage: node generate.mjs [--latest|--all] [--package NAME@VERSION ...] [--output FILE]\n\n` +
    `--latest and --all generate the latest published version of every @aws-sdk/client-* package.\n` +
    `--package accepts an exact package/version, for example @aws-sdk/client-s3@3.XXX.X.`;
}

function parseArguments(argv) {
  const packages = [];
  let latest = false;
  let output = resolve(import.meta.dirname, "../data/mappings.json");
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--latest" || argument === "--all") {
      latest = true;
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
  if (!latest && packages.length === 0) throw new Error("select --latest, --all, or at least one --package");
  return { latest, output, packages };
}

async function json(url) {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`);
  return response.json();
}

async function latestClientPackages() {
  const tree = await json("https://api.github.com/repos/aws/aws-sdk-js-v3/git/trees/main?recursive=1");
  const names = [...new Set(
    (tree.tree ?? [])
      .filter((entry) => entry.type === "blob")
      .map((entry) => entry.path.match(/^clients\/(client-[a-z0-9-]+)\/package\.json$/)?.[1])
      .filter(Boolean)
      .filter((name) => CLIENT_PACKAGE.test(`@aws-sdk/${name}`))
      .map((name) => `@aws-sdk/${name}`),
  )];
  const packages = [];
  for (let index = 0; index < names.length; index += 8) {
    const batch = names.slice(index, index + 8);
    packages.push(...await Promise.all(batch.map(async (packageName) => {
      const metadata = await json(`${REGISTRY}/${encodeURIComponent(packageName)}`);
      return { package: packageName, version: metadata["dist-tags"]?.latest };
    })));
  }
  return packages
    .filter((item) => typeof item.version === "string")
    .sort(comparePackageVersions);
}

function comparePackageVersions(left, right) {
  return left.package.localeCompare(right.package) || left.version.localeCompare(right.version);
}

function untar(gzip) {
  const bytes = gunzipSync(gzip);
  const files = new Map();
  for (let offset = 0; offset + 512 <= bytes.length;) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const name = readTarString(header, 0, 100);
    const prefix = readTarString(header, 345, 155);
    const sizeText = readTarString(header, 124, 12).trim();
    const size = Number.parseInt(sizeText || "0", 8);
    if (!Number.isSafeInteger(size) || size < 0) throw new Error(`invalid tar entry size for ${name}`);
    const fullName = prefix ? `${prefix}/${name}` : name;
    const contentStart = offset + 512;
    const contentEnd = contentStart + size;
    if (contentEnd > bytes.length) throw new Error(`truncated tar entry ${fullName}`);
    if (header[156] === 48 && fullName.startsWith("package/")) {
      files.set(fullName.slice("package/".length), Buffer.from(bytes.subarray(contentStart, contentEnd)).toString("utf8"));
    }
    offset = contentStart + Math.ceil(size / 512) * 512;
  }
  return files;
}

function readTarString(bytes, offset, length) {
  const end = bytes.subarray(offset, offset + length).indexOf(0);
  return Buffer.from(bytes.subarray(offset, end < 0 ? offset + length : offset + end)).toString("utf8");
}

async function packageFiles(packageName, version) {
  const metadata = await json(`${REGISTRY}/${encodeURIComponent(packageName)}/${encodeURIComponent(version)}`);
  if (metadata.version !== version) throw new Error(`${packageName}@${version}: registry returned ${metadata.version}`);
  if (!metadata.dist?.tarball) throw new Error(`${packageName}@${version}: missing tarball`);
  const response = await fetch(metadata.dist.tarball);
  if (!response.ok) throw new Error(`${packageName}@${version}: tarball ${response.status} ${response.statusText}`);
  return untar(Buffer.from(await response.arrayBuffer()));
}

function sourceFiles(files, prefix) {
  return [...files].filter(([name]) => name.startsWith(prefix) && name.endsWith(".js"));
}

function matchOne(text, pattern, description) {
  const matches = [...text.matchAll(pattern)];
  if (matches.length !== 1) throw new Error(`${description}: expected one match, got ${matches.length}`);
  return matches[0];
}

function canonicalSigningService(packageName, files) {
  const candidates = sourceFiles(files, "dist-es/").filter(([name]) =>
    /runtimeConfig\.shared\.js$/.test(name) || /auth\/httpAuthSchemeProvider\.js$/.test(name));
  const services = new Set();
  for (const [name, text] of candidates) {
    const constants = new Map([...text.matchAll(/\bconst\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*["']([^"']+)["']/g)].map((match) => [match[1], match[2]]));
    for (const match of text.matchAll(/\bsigningService\s*:\s*(?:config\.signingService\s*\?\?\s*)?([A-Za-z_$][A-Za-z0-9_$]*|["'][^"']+["'])/g)) {
      const value = match[1];
      const service = value.startsWith("\"") || value.startsWith("'") ? value.slice(1, -1) : constants.get(value);
      if (!service) throw new Error(`${packageName}:${name}: cannot resolve signingService ${value}`);
      services.add(service);
    }
    for (const match of text.matchAll(/\bsigningProperties\s*:\s*\{\s*name\s*:\s*["']([^"']+)["']/g)) services.add(match[1]);
  }
  if (services.size !== 1) {
    throw new Error(`${packageName}: expected one signing service in SDK metadata, found ${[...services].join(", ") || "none"}`);
  }
  return [...services][0];
}

function commandMappings(packageName, files, service) {
  const mappings = [];
  const commands = new Map();
  for (const [file, text] of sourceFiles(files, "dist-es/commands/")) {
    const classMatch = text.match(/export\s+class\s+([A-Za-z0-9_]+Command)\b/);
    if (!classMatch) continue;
    const operationMatch = text.match(/(?:\.s\(\s*["'][^"']+["']\s*,\s*["']([^"']+)["']|extends\s+command\(\s*[^,]+,\s*[^,]+,\s*["']([^"']+)["'])/);
    if (!operationMatch) throw new Error(`${packageName}:${file}: operation name not found`);
    const operation = operationMatch[1] ?? operationMatch[2];
    const command = classMatch[1];
    if (commands.has(command)) throw new Error(`${packageName}: duplicate command ${command}`);
    commands.set(command, operation);
    mappings.push({ package: packageName, receiver: null, method: command, api_methods: [{ service, name: operation }] });
  }
  if (commands.size === 0) throw new Error(`${packageName}: no exported commands found`);
  return { commands, mappings };
}
function clientMappings(packageName, files, service, commands) {
  const filesWithAggregatedClient = sourceFiles(files, "dist-es/").filter(([, text]) => /createAggregatedClient\(/.test(text));
  if (filesWithAggregatedClient.length !== 1) {
    throw new Error(`${packageName}: expected one aggregated client module, found ${filesWithAggregatedClient.length}`);
  }
  const [file, text] = filesWithAggregatedClient[0];
  const aggregatedMatch = matchOne(
    text,
    /createAggregatedClient\(\s*commands\s*,\s*([A-Za-z0-9_]+)\s*(?:,|\))/g,
    `${packageName}:${file}`,
  );
  const receiver = aggregatedMatch[1];
  if (!new RegExp(`export\\s+class\\s+${receiver}\\b`, "g").test(text)) {
    throw new Error(`${packageName}:${file}: aggregated client class not found`);
  }
  const commandBlock = matchOne(text, /const\s+commands\s*=\s*\{([\s\S]*?)\n\};/g, `${packageName}:${file}`)[1];
  const aggregatedCommands = new Set([...commandBlock.matchAll(/\b([A-Za-z0-9_]+Command)\b/g)].map((match) => match[1]));
  if (aggregatedCommands.size === 0) throw new Error(`${packageName}:${file}: no aggregated command symbols`);
  return [...aggregatedCommands].sort().map((command) => {
    const operation = commands.get(command);
    if (!operation) throw new Error(`${packageName}:${file}: aggregated client references unknown ${command}`);
    return {
      package: packageName,
      receiver,
      method: command.slice(0, -"Command".length).replace(/^./, (letter) => letter.toLowerCase()),
      api_methods: [{ service, name: operation }],
    };
  });
}

function helperMappings(packageName, files, service, commands) {
  const mappings = [];
  for (const [file, text] of sourceFiles(files, "dist-es/pagination/")) {
    for (const match of text.matchAll(/export\s+(?:const|function)\s+(paginate[A-Za-z0-9_]+)/g)) {
      const method = match[1];
      const command = `${method.slice("paginate".length)}Command`;
      const operation = commands.get(command);
      if (operation) {
        mappings.push({ package: packageName, receiver: null, method, api_methods: [{ service, name: operation }] });
      }
    }
  }
  for (const [file, text] of sourceFiles(files, "dist-es/waiters/")) {
    const names = [...text.matchAll(/export\s+(?:const|function)\s+(wait(?:For|Until)[A-Za-z0-9_]+)/g)].map((match) => match[1]);
    if (names.length === 0) continue;
    const commandMatch = text.match(/new\s+([A-Za-z0-9_]+Command)\s*\(/);
    if (!commandMatch) continue;
    const operation = commands.get(commandMatch[1]);
    if (!operation) continue;
    for (const method of names) mappings.push({ package: packageName, receiver: null, method, api_methods: [{ service, name: operation }] });
  }
  return mappings;
}

function compareMappings(left, right) {
  return left.package.localeCompare(right.package) ||
    String(left.receiver).localeCompare(String(right.receiver)) ||
    left.method.localeCompare(right.method);
}

async function inspectPackage(packageVersion) {
  const files = await packageFiles(packageVersion.package, packageVersion.version);
  const service = canonicalSigningService(packageVersion.package, files);
  const { commands, mappings } = commandMappings(packageVersion.package, files, service);
  mappings.push(...clientMappings(packageVersion.package, files, service, commands));
  mappings.push(...helperMappings(packageVersion.package, files, service, commands));
  mappings.sort(compareMappings);
  for (let index = 1; index < mappings.length; index += 1) {
    if (compareMappings(mappings[index - 1], mappings[index]) === 0) {
      throw new Error(`${packageVersion.package}: duplicate mapping ${mappings[index].receiver ?? ""}.${mappings[index].method}`);
    }
  }
  return { ...packageVersion, service, mappings };
}

async function run() {
  const options = parseArguments(process.argv.slice(2));
  const requested = new Map(options.packages.map((item) => [`${item.package}@${item.version}`, item]));
  if (options.latest) {
    for (const item of await latestClientPackages()) requested.set(`${item.package}@${item.version}`, item);
  }
  const packages = [...requested.values()].sort(comparePackageVersions);
  const result = [];
  const concurrency = 8;
  for (let index = 0; index < packages.length; index += concurrency) {
    const batch = packages.slice(index, index + concurrency);
    const outcomes = await Promise.allSettled(batch.map(inspectPackage));
    for (const outcome of outcomes) {
      if (outcome.status === "fulfilled") {
        result.push(outcome.value);
      } else if (options.latest) {
        process.stderr.write(`skipping unsupported package release: ${outcome.reason?.message ?? outcome.reason}\n`);
      } else {
        throw outcome.reason;
      }
    }
  }
  const data = {
    schema_version: 1,
    provenance: {
      registry: REGISTRY,
      source: "published npm package tarballs; dist-es command builders, runtime config signingService, paginator, and waiter modules",
    },
    packages: result,
  };
  await mkdir(dirname(options.output), { recursive: true });
  const temporary = `${options.output}.tmp`;
  await writeFile(temporary, `${JSON.stringify(data, null, 2)}\n`);
  await rename(temporary, options.output);
  process.stderr.write(`wrote ${result.length} exact npm package releases to ${options.output}\n`);
}

run().catch((error) => {
  process.stderr.write(`error: ${error.stack ?? error}\n`);
  process.exitCode = 1;
});
