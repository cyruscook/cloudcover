import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { openSourceRepository } from "./source-repository.mjs";

test("Git discovery includes lib-dynamodb and verifies its exact release manifest", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "cloudcover-sdk-source-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const git = (...args) => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" });
  git("init", "-b", "main");
  for (const [directory, name] of [["lib/lib-dynamodb", "@aws-sdk/lib-dynamodb"], ["clients/client-dynamodb", "@aws-sdk/client-dynamodb"]]) {
    mkdirSync(join(root, directory, "src/commands"), { recursive: true });
    writeFileSync(join(root, directory, "package.json"), JSON.stringify({ name, version: "3.8.0" }));
    writeFileSync(join(root, directory, "src/commands/GetCommand.ts"), "export class GetCommand {}\n");
  }
  git("add", ".");
  git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "SDK fixture");
  git("tag", "v3.8.0");
  git("tag", "v3.9.0");
  const repository = await openSourceRepository({ repository: root });
  try {
    for (const includeHistorical of [false, true]) {
      assert.deepEqual(await repository.packageNames({ includeHistorical }), ["@aws-sdk/client-dynamodb", "@aws-sdk/lib-dynamodb"]);
    }
    const snapshot = await repository.snapshot({ package: "@aws-sdk/lib-dynamodb", version: "3.8.0" });
    assert.ok(snapshot);
    assert.equal((await snapshot.files()).get("src/commands/GetCommand.ts"), "export class GetCommand {}\n");
    assert.equal(await repository.snapshot({ package: "@aws-sdk/lib-dynamodb", version: "3.9.0" }), null);
    assert.equal(await repository.snapshot({ package: "@aws-sdk/lib-unknown", version: "3.8.0" }), null);
  } finally {
    await repository.close();
  }
});
