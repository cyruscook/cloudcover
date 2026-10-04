import assert from "node:assert/strict";
import { test } from "node:test";
import { inspectFiles } from "./mappings.mjs";

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
