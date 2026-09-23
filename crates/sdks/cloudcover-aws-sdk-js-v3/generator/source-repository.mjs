import { createHash } from "node:crypto";
import { execFile, spawn } from "node:child_process";
import { mkdir, rename, rm, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const DEFAULT_REPOSITORY = "https://github.com/aws/aws-sdk-js-v3.git";
const CACHE_NAME = "aws-sdk-js-v3.git";
const GIT_ENV = { ...process.env, GIT_TERMINAL_PROMPT: "0" };
const MAX_GIT_OUTPUT = 256 * 1024 * 1024;
const OBJECT_CACHE_BYTES = 64 * 1024 * 1024;
const SNAPSHOT_CACHE_ENTRIES = 256;
const SOURCE_DIRECTORIES = new Set(["commands", "pagination", "waiters", "auth"]);
const OMITTED_SEGMENTS = new Set([
  "__tests__",
  "test",
  "tests",
  "models",
  "protocols",
  "endpoints",
]);

async function runGit(repository, args) {
  try {
    const { stdout } = await execFileAsync("git", ["-C", repository, ...args], {
      encoding: "utf8",
      env: GIT_ENV,
      maxBuffer: MAX_GIT_OUTPUT,
    });
    return stdout;
  } catch (error) {
    error.message = `git ${args.join(" ")} failed in ${repository}: ${error.message}`;
    throw error;
  }
}

async function pathExists(path) {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function prepareRepository(repository, cacheDir) {
  if (repository !== undefined) {
    const localRepository = resolve(repository);
    await runGit(localRepository, ["rev-parse", "--git-dir"]);
    return localRepository;
  }

  if (cacheDir === undefined) {
    throw new TypeError("cacheDir is required when repository is not provided");
  }

  const directory = resolve(cacheDir);
  const cachedRepository = resolve(directory, CACHE_NAME);
  await mkdir(directory, { recursive: true });

  if (await pathExists(cachedRepository)) {
    await runGit(cachedRepository, [
      "fetch",
      "--force",
      "--prune-tags",
      "origin",
      "+refs/heads/main:refs/heads/main",
      "+refs/tags/*:refs/tags/*",
    ]);
    await runGit(cachedRepository, ["symbolic-ref", "HEAD", "refs/heads/main"]);
    return cachedRepository;
  }

  const temporaryRepository = `${cachedRepository}.tmp-${process.pid}-${Date.now()}`;
  try {
    await execFileAsync("git", ["clone", "--bare", DEFAULT_REPOSITORY, temporaryRepository], {
      env: GIT_ENV,
      maxBuffer: MAX_GIT_OUTPUT,
    });
    try {
      await rename(temporaryRepository, cachedRepository);
    } catch (error) {
      if ((error.code !== "EEXIST" && error.code !== "ENOTEMPTY") || !(await pathExists(cachedRepository))) {
        throw error;
      }
      await rm(temporaryRepository, { recursive: true, force: true });
    }
  } catch (error) {
    await rm(temporaryRepository, { recursive: true, force: true });
    error.message = `unable to create cached AWS SDK source repository: ${error.message}`;
    throw error;
  }
  await runGit(cachedRepository, ["symbolic-ref", "HEAD", "refs/heads/main"]);

  return cachedRepository;
}

class StreamReader {
  constructor(stream, errorDescription) {
    this.chunks = [];
    this.offset = 0;
    this.waiter = undefined;
    this.failure = undefined;
    this.errorDescription = errorDescription;

    stream.on("data", (chunk) => {
      this.chunks.push(chunk);
      this.#wake();
    });
    stream.on("error", (error) => {
      this.failure = error;
      this.#wake();
    });
    stream.on("end", () => {
      this.failure ??= new Error(`unexpected end of ${this.errorDescription}`);
      this.#wake();
    });
  }

  #wake() {
    const waiter = this.waiter;
    this.waiter = undefined;
    waiter?.();
  }

  async #ready() {
    while (this.chunks.length === 0) {
      if (this.failure) throw this.failure;
      await new Promise((resolveWaiter) => {
        this.waiter = resolveWaiter;
      });
    }
  }

  async readLine() {
    const pieces = [];
    let length = 0;
    for (;;) {
      await this.#ready();
      const chunk = this.chunks[0];
      const newline = chunk.indexOf(10, this.offset);
      const end = newline === -1 ? chunk.length : newline;
      const piece = chunk.subarray(this.offset, end);
      pieces.push(piece);
      length += piece.length;
      if (length > 1024 * 1024) throw new Error(`invalid overlong line from ${this.errorDescription}`);

      if (newline !== -1) {
        this.offset = newline + 1;
        if (this.offset === chunk.length) {
          this.chunks.shift();
          this.offset = 0;
        }
        return Buffer.concat(pieces, length).toString("utf8");
      }

      this.chunks.shift();
      this.offset = 0;
    }
  }

  async readExactly(length) {
    const output = Buffer.allocUnsafe(length);
    let written = 0;
    while (written < length) {
      await this.#ready();
      const chunk = this.chunks[0];
      const available = chunk.length - this.offset;
      const count = Math.min(available, length - written);
      chunk.copy(output, written, this.offset, this.offset + count);
      written += count;
      this.offset += count;
      if (this.offset === chunk.length) {
        this.chunks.shift();
        this.offset = 0;
      }
    }
    return output;
  }
}

class CatFileBatch {
  constructor(repository) {
    this.child = spawn("git", ["-C", repository, "cat-file", "--batch"], {
      env: GIT_ENV,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.reader = new StreamReader(this.child.stdout, "git cat-file output");
    this.stderr = "";
    this.closed = false;
    this.failure = undefined;
    this.tail = Promise.resolve();

    this.child.stderr.setEncoding("utf8");
    this.child.stderr.on("data", (text) => {
      if (this.stderr.length < 64 * 1024) this.stderr += text.slice(0, 64 * 1024 - this.stderr.length);
    });
    this.exit = new Promise((resolveExit) => {
      this.child.once("error", (error) => {
        this.failure = error;
        resolveExit({ error });
      });
      this.child.once("close", (code, signal) => {
        if (code !== 0 && !this.failure) {
          this.failure = new Error(
            `git cat-file exited with ${signal ? `signal ${signal}` : `code ${code}`}${this.stderr ? `: ${this.stderr.trim()}` : ""}`,
          );
        }
        resolveExit({ code, signal });
      });
    });
  }

  #enqueue(operation) {
    if (this.closed) return Promise.reject(new Error("source repository is closed"));
    const result = this.tail.then(operation);
    this.tail = result.catch(() => {});
    return result;
  }

  async #write(input) {
    if (this.failure) throw this.failure;
    await new Promise((resolveWrite, rejectWrite) => {
      this.child.stdin.write(input, (error) => (error ? rejectWrite(error) : resolveWrite()));
    });
  }

  async #readObject(expectedOid) {
    const header = await this.reader.readLine();
    if (header === `${expectedOid} missing`) {
      throw new Error(`Git object ${expectedOid} is missing`);
    }
    const match = /^([0-9a-f]+) ([a-z]+) ([0-9]+)$/.exec(header);
    if (!match) throw new Error(`invalid git cat-file response: ${header}`);
    const size = Number(match[3]);
    if (!Number.isSafeInteger(size)) throw new Error(`invalid Git object size: ${match[3]}`);
    const body = await this.reader.readExactly(size);
    const terminator = await this.reader.readExactly(1);
    if (terminator[0] !== 10) throw new Error("invalid git cat-file object terminator");
    return { oid: match[1], type: match[2], body };
  }

  read(oid) {
    return this.#enqueue(async () => {
      await this.#write(`${oid}\n`);
      return this.#readObject(oid);
    });
  }

  readMany(oids) {
    return this.#enqueue(async () => {
      if (oids.length === 0) return [];
      const write = this.#write(`${oids.join("\n")}\n`);
      write.catch(() => {});
      const objects = [];
      for (const oid of oids) objects.push(await this.#readObject(oid));
      await write;
      return objects;
    });
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    await this.tail;
    if (!this.child.stdin.destroyed) this.child.stdin.end();
    await this.exit;
    if (this.failure) throw this.failure;
  }
}

class ByteLruCache {
  constructor(maxBytes) {
    this.maxBytes = maxBytes;
    this.bytes = 0;
    this.values = new Map();
  }

  get(key) {
    const value = this.values.get(key);
    if (value === undefined) return undefined;
    this.values.delete(key);
    this.values.set(key, value);
    return value.object;
  }

  set(key, object) {
    const size = object.body.length;
    if (size > this.maxBytes) return;
    const previous = this.values.get(key);
    if (previous) this.bytes -= previous.size;
    this.values.delete(key);
    this.values.set(key, { object, size });
    this.bytes += size;
    while (this.bytes > this.maxBytes) {
      const oldest = this.values.entries().next().value;
      this.values.delete(oldest[0]);
      this.bytes -= oldest[1].size;
    }
  }
}

class EntryLruCache {
  constructor(maxEntries) {
    this.maxEntries = maxEntries;
    this.values = new Map();
  }

  get(key) {
    if (!this.values.has(key)) return undefined;
    const value = this.values.get(key);
    this.values.delete(key);
    this.values.set(key, value);
    return value;
  }

  set(key, value) {
    this.values.delete(key);
    this.values.set(key, value);
    while (this.values.size > this.maxEntries) this.values.delete(this.values.keys().next().value);
  }
}

function parseTree(body, oidBytes) {
  const entries = [];
  let offset = 0;
  while (offset < body.length) {
    const space = body.indexOf(32, offset);
    const nul = body.indexOf(0, space + 1);
    if (space === -1 || nul === -1 || nul + 1 + oidBytes > body.length) {
      throw new Error("invalid Git tree object");
    }
    const mode = body.toString("ascii", offset, space);
    const name = body.toString("utf8", space + 1, nul);
    const oid = body.subarray(nul + 1, nul + 1 + oidBytes).toString("hex");
    entries.push({ name, oid, tree: mode === "40000" });
    offset = nul + 1 + oidBytes;
  }
  return entries;
}

function isSourceFile(path) {
  const segments = path.split("/");
  const file = segments.at(-1);
  if (!file.endsWith(".ts") || file.endsWith(".d.ts")) return false;
  if (/(?:^|\.)(?:spec|test)\.ts$/.test(file)) return false;
  return !segments.some((segment) => OMITTED_SEGMENTS.has(segment));
}

function packageDirectory(packageName) {
  if (typeof packageName !== "string") return undefined;
  const name = packageName.includes("/") ? packageName.slice(packageName.lastIndexOf("/") + 1) : packageName;
  return /^client-[a-z0-9][a-z0-9-]*$/.test(name) ? name : undefined;
}

/**
 * Opens AWS SDK JS v3 Git history. `snapshot` returns null only when the exact
 * vVERSION tag does not contain a matching client package/version. Call
 * `files()` before `close()`; it reads all selected blobs through one batch
 * Git process and preserves their repository-relative package paths.
 */
export async function openSourceRepository({ repository, cacheDir } = {}) {
  const repositoryPath = await prepareRepository(repository, cacheDir);
  const [objectFormatText, refsText, headOidText] = await Promise.all([
    runGit(repositoryPath, ["rev-parse", "--show-object-format"]),
    runGit(repositoryPath, ["for-each-ref", "--format=%(refname:strip=2)%00%(objectname)", "refs/tags/v*"]),
    runGit(repositoryPath, ["rev-parse", "HEAD"]),
  ]);
  const objectFormat = objectFormatText.trim();
  const oidBytes = objectFormat === "sha1" ? 20 : objectFormat === "sha256" ? 32 : undefined;
  if (oidBytes === undefined) throw new Error(`unsupported Git object format: ${objectFormat}`);

  const tags = new Map();
  for (const line of refsText.split("\n")) {
    if (!line) continue;
    const separator = line.indexOf("\0");
    if (separator === -1) throw new Error(`invalid Git tag ref record: ${line}`);
    tags.set(line.slice(0, separator), line.slice(separator + 1));
  }
  const headOid = headOidText.trim();

  const batch = new CatFileBatch(repositoryPath);
  const objects = new ByteLruCache(OBJECT_CACHE_BYTES);
  const snapshots = new EntryLruCache(SNAPSHOT_CACHE_ENTRIES);
  let closed = false;

  async function object(oid, cache = true) {
    const cached = objects.get(oid);
    if (cached) return cached;
    const value = await batch.read(oid);
    if (cache) objects.set(oid, value);
    return value;
  }

  async function commitTree(refOid) {
    let oid = refOid;
    for (let depth = 0; depth < 16; depth += 1) {
      const value = await object(oid);
      if (value.type === "commit") {
        const match = /^tree ([0-9a-f]+)$/m.exec(value.body.toString("utf8"));
        if (!match) throw new Error(`Git commit ${oid} has no tree`);
        return match[1];
      }
      if (value.type !== "tag") throw new Error(`Git tag resolved to unexpected ${value.type} object ${oid}`);
      const match = /^object ([0-9a-f]+)$/m.exec(value.body.toString("utf8"));
      if (!match) throw new Error(`Git tag object ${oid} has no target`);
      oid = match[1];
    }
    throw new Error(`Git tag chain from ${refOid} is too deep`);
  }

  async function treeEntries(oid) {
    const value = await object(oid);
    if (value.type !== "tree") throw new Error(`expected Git tree ${oid}, found ${value.type}`);
    return parseTree(value.body, oidBytes);
  }

  async function childTree(treeOid, name) {
    const entry = (await treeEntries(treeOid)).find((candidate) => candidate.name === name);
    return entry?.tree ? entry.oid : undefined;
  }

  async function collectDirectory(treeOid, prefix, output) {
    for (const entry of await treeEntries(treeOid)) {
      const path = `${prefix}${entry.name}`;
      if (entry.tree) {
        if (!OMITTED_SEGMENTS.has(entry.name)) await collectDirectory(entry.oid, `${path}/`, output);
      } else if (isSourceFile(path)) {
        output.push({ path, oid: entry.oid });
      }
    }
  }

  async function inspectClientTree(clientTreeOid, packageName) {
    const cacheKey = `${packageName}\0${clientTreeOid}`;
    const cached = snapshots.get(cacheKey);
    if (cached !== undefined) return cached;

    const rootEntries = await treeEntries(clientTreeOid);
    const packageJsonEntry = rootEntries.find((entry) => entry.name === "package.json" && !entry.tree);
    if (!packageJsonEntry) {
      snapshots.set(cacheKey, null);
      return null;
    }

    const packageObject = await object(packageJsonEntry.oid);
    if (packageObject.type !== "blob") {
      throw new Error(`package.json ${packageJsonEntry.oid} is a ${packageObject.type}, not a blob`);
    }
    let manifest;
    try {
      manifest = JSON.parse(packageObject.body.toString("utf8"));
    } catch (error) {
      error.message = `invalid package.json in ${packageName} tree ${clientTreeOid}: ${error.message}`;
      throw error;
    }

    const srcEntry = rootEntries.find((entry) => entry.name === "src" && entry.tree);
    const sourceTreeOid = srcEntry?.oid ?? clientTreeOid;
    const prefix = srcEntry ? "src/" : "";
    const sourceEntries = srcEntry ? await treeEntries(sourceTreeOid) : rootEntries;
    const files = [];
    for (const entry of sourceEntries) {
      const path = `${prefix}${entry.name}`;
      if (!entry.tree && isSourceFile(path)) {
        files.push({ path, oid: entry.oid });
      } else if (entry.tree && SOURCE_DIRECTORIES.has(entry.name)) {
        await collectDirectory(entry.oid, `${path}/`, files);
      }
    }
    files.sort((left, right) => left.path.localeCompare(right.path));

    const result = { name: manifest.name, version: manifest.version, files };
    snapshots.set(cacheKey, result);
    return result;
  }

  return {
    async snapshot({ package: packageName, version }) {
      if (closed) throw new Error("source repository is closed");
      const clientDirectory = packageDirectory(packageName);
      if (!clientDirectory || typeof version !== "string") return null;
      const refOid = tags.get(`v${version}`);
      if (refOid === undefined) return null;

      const rootTreeOid = await commitTree(refOid);
      const clientsTreeOid = await childTree(rootTreeOid, "clients");
      if (clientsTreeOid === undefined) return null;
      const clientTreeOid = await childTree(clientsTreeOid, clientDirectory);
      if (clientTreeOid === undefined) return null;
      const inspected = await inspectClientTree(clientTreeOid, packageName);
      if (inspected === null || inspected.name !== packageName || inspected.version !== version) return null;

      const digest = createHash("sha256");
      digest.update(packageName);
      digest.update("\0");
      for (const file of inspected.files) {
        digest.update(file.path);
        digest.update("\0");
        digest.update(file.oid);
        digest.update("\0");
      }
      const selectedFiles = inspected.files;
      let loadedFiles;

      return {
        key: `git:${digest.digest("hex")}`,
        async files() {
          if (loadedFiles) return loadedFiles;
          if (closed) throw new Error("source repository is closed");
          loadedFiles = (async () => {
            const contents = new Map();
            const chunkSize = 1024;
            for (let offset = 0; offset < selectedFiles.length; offset += chunkSize) {
              const selectedChunk = selectedFiles.slice(offset, offset + chunkSize);
              const values = await batch.readMany(selectedChunk.map((file) => file.oid));
              for (let index = 0; index < values.length; index += 1) {
                if (values[index].type !== "blob") {
                  throw new Error(`source ${selectedChunk[index].path} is a ${values[index].type}, not a blob`);
                }
                contents.set(selectedChunk[index].path, values[index].body.toString("utf8"));
              }
            }
            return contents;
          })();
          return loadedFiles;
        },
      };
    },

    async packageNames() {
      if (closed) throw new Error("source repository is closed");
      const rootTreeOid = await commitTree(headOid);
      const clientsTreeOid = await childTree(rootTreeOid, "clients");
      if (clientsTreeOid === undefined) throw new Error("Git HEAD has no clients directory");

      const clientEntries = (await treeEntries(clientsTreeOid)).filter(
        (entry) => entry.tree && /^client-[a-z0-9][a-z0-9-]*$/.test(entry.name),
      );
      const names = [];
      const chunkSize = 1024;
      for (let offset = 0; offset < clientEntries.length; offset += chunkSize) {
        const entryChunk = clientEntries.slice(offset, offset + chunkSize);
        const values = await batch.readMany(entryChunk.map((entry) => entry.oid));
        for (let index = 0; index < values.length; index += 1) {
          if (values[index].type !== "tree") {
            throw new Error(`client ${entryChunk[index].name} is a ${values[index].type}, not a tree`);
          }
          const hasManifest = parseTree(values[index].body, oidBytes).some(
            (entry) => entry.name === "package.json" && !entry.tree,
          );
          if (hasManifest) names.push(`@aws-sdk/${entryChunk[index].name}`);
        }
      }
      names.sort();
      return names;
    },

    async close() {
      if (closed) return;
      closed = true;
      await batch.close();
    },
  };
}
