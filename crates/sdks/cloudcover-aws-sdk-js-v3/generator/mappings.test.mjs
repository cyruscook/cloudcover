import assert from "node:assert/strict";
import { test } from "node:test";
import { inspectFiles } from "./mappings.mjs";
import { readFileSync } from "node:fs";

function sourceFiles(description, service = "example") {
  return new Map([
    ["src/runtimeConfig.shared.ts", `export const getRuntimeConfig = (config: any) => ({ signingService: config.signingService ?? "${service}" });`],
    ["src/commands/GetWidgetCommand.ts", `/** ${description} */\nexport class GetWidgetCommand {}`],
    ["src/Example.ts", `
      import { GetWidgetCommand } from "./commands/GetWidgetCommand";
      import { ExampleClient } from "./ExampleClient";
      /** ${description} */
      export class Example extends ExampleClient {
        public getWidget(args: any, options?: any): Promise<any>;
        public getWidget(args: any, options?: any): Promise<any> {
          return this.send(new GetWidgetCommand(args), options);
        }
      }
    `],
    ["src/pagination/GetWidgetPaginator.ts", `export async function* paginateGetWidget() { yield {}; }`],
    ["src/waiters/waitForWidgetExists.ts", `
      const checkState = (client: any) => client.send(new GetWidgetCommand({}));
      export const waitForWidgetExists = async () => checkState({});
      export const waitUntilWidgetExists = async () => checkState({});
    `],
  ]);
}

test("documentation changes preserve command, legacy client, paginator, and waiter mappings", () => {
  const first = inspectFiles({ package: "@aws-sdk/client-example", version: "3.0.0" }, sourceFiles("Old documentation"));
  const next = inspectFiles({ package: "@aws-sdk/client-example", version: "3.1.0" }, sourceFiles("New documentation"));
  assert.deepEqual(first.mappings, next.mappings);
  assert.equal(first.mappings.length, 5);
  assert.deepEqual(new Set(first.mappings.map((mapping) => mapping.method)), new Set([
    "GetWidgetCommand", "getWidget", "paginateGetWidget", "waitForWidgetExists", "waitUntilWidgetExists",
  ]));
  for (const mapping of first.mappings) {
    assert.deepEqual(mapping.api_methods, [{ service: "example", name: "GetWidget" }]);
  }
});

test("cached modules retain release-specific signing metadata", () => {
  inspectFiles({ package: "@aws-sdk/client-example", version: "3.0.0" }, sourceFiles("Documentation", "before"));
  const next = inspectFiles({ package: "@aws-sdk/client-example", version: "3.1.0" }, sourceFiles("Documentation", "after"));
  for (const mapping of next.mappings) {
    assert.equal(mapping.api_methods[0].service, "after");
  }
});

test("invalid TypeScript still fails generation", () => {
  const files = sourceFiles("Documentation");
  files.set("src/commands/GetWidgetCommand.ts", "export class GetWidgetCommand {");
  assert.throws(() => inspectFiles({ package: "@aws-sdk/client-example", version: "3.0.0" }, files), /TypeScript transpilation failed/);
});

const documentOperations = {
  BatchExecuteStatement: "BatchExecuteStatement", BatchGet: "BatchGetItem", BatchWrite: "BatchWriteItem",
  Delete: "DeleteItem", ExecuteStatement: "ExecuteStatement", ExecuteTransaction: "ExecuteTransaction",
  Get: "GetItem", Put: "PutItem", Query: "Query", Scan: "Scan", TransactGet: "TransactGetItems",
  TransactWrite: "TransactWriteItems", Update: "UpdateItem",
};

function documentFixture(version) {
  const fixture = JSON.parse(readFileSync(new URL(`./fixtures/lib-dynamodb-${version}.json`, import.meta.url)));
  return { ...fixture, files: new Map(Object.entries(fixture.files)) };
}

for (const version of ["3.8.0", "3.1170.0"]) {
  test(`published lib-dynamodb ${version} wraps exact low-level operations`, () => {
    const fixture = documentFixture(version);
    const { mappings } = inspectFiles(fixture, fixture.files);
    for (const [wrapper, operation] of Object.entries(documentOperations)) {
      for (const [receiver, method] of [[null, `${wrapper}Command`],
        ["DynamoDBDocument", wrapper.replace(/^./, (letter) => letter.toLowerCase())]]) {
        assert.deepEqual(mappings.find((row) => row.receiver === receiver && row.method === method)?.api_methods,
          [{ service: "dynamodb", name: operation }], `${receiver}.${method}`);
      }
    }
    assert.ok(!mappings.some((row) => ["from", "destroy", "constructor", "resolveMiddleware"].includes(row.method)));
    if (version === "3.1170.0") {
      for (const [method, operation] of [["paginateQuery", "Query"], ["paginateScan", "Scan"], ["paginatedBatchGet", "BatchGetItem"]]) {
        assert.deepEqual(mappings.find((row) => row.receiver === null && row.method === method)?.api_methods,
          [{ service: "dynamodb", name: operation }]);
      }
      assert.deepEqual(mappings.find((row) => row.receiver === "DynamoDBDocument" && row.method === "paginatedBatchGet")?.api_methods,
        [{ service: "dynamodb", name: "BatchGetItem" }]);
    }
  });
}

test("document wrapper extraction rejects missing and ambiguous low-level operations", () => {
  for (const body of ["", "new OtherCommand({});"]) {
    const fixture = documentFixture("3.1170.0");
    fixture.files.set("dist-es/commands/GetCommand.js", `
      import { PutItemCommand as OtherCommand } from "@aws-sdk/client-dynamodb";
      export class GetCommand { constructor() { ${body} } }
    `);
    if (!body) assert.throws(() => inspectFiles(fixture, fixture.files), /expected one wrapped DynamoDB operation/);
    else {
      // A renamed wrapper still follows source evidence, never its own name.
      assert.deepEqual(inspectFiles(fixture, fixture.files).mappings.find((row) => row.method === "GetCommand").api_methods,
        [{ service: "dynamodb", name: "PutItem" }]);
      fixture.files.set("dist-es/commands/GetCommand.js", fixture.files.get("dist-es/commands/GetCommand.js") + `
        import { GetItemCommand } from "@aws-sdk/client-dynamodb"; new GetItemCommand({});
      `);
      assert.throws(() => inspectFiles(fixture, fixture.files), /expected one wrapped DynamoDB operation/);
    }
  }
});

test("unknown document methods fail generation without borrowing the next method's command", () => {
  const fixture = documentFixture("3.1170.0");
  fixture.files.set("dist-es/DynamoDBDocument.js", `
    export class DynamoDBDocument extends DynamoDBDocumentClient {
      unknown() { return unknownHelper(); }
      put(args) { return this.send(new PutCommand(args)); }
    }
  `);
  assert.throws(() => inspectFiles(fixture, fixture.files), /cannot resolve document method unknown/);
});
