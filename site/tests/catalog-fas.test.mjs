import assert from "node:assert/strict";
import { test } from "node:test";
import { parseApiCatalog } from "../src/lib/catalog.ts";

function document() {
  return {
    schema_version: 2,
    iam_permissions: ["kms:Decrypt", "secretsmanager:GetSecretValue", "ssm:GetParameter"],
    api_methods: [["ssm", "GetParameter", [0, 1, 2]]],
    fas: [{
      coverage: "documented_dependencies",
      dependencies: [{
        rule: {
          id: "secrets-read", applicability: "Customer-managed secret key.",
          resource_role: "secret key", via_service: "secretsmanager.${Region}.amazonaws.com",
          sources: [{ url: "https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html", evidence: "GetSecretValue calls Decrypt." }],
        },
        path: ["ssm-secrets-reference", "secrets-read"], iam_permissions: ["kms:Decrypt"],
      }],
    }],
  };
}

test("catalog retains conditional FAS sources, key scope, and transitive provenance", () => {
  const dependency = parseApiCatalog(document()).apiMethods[0].fas.dependencies[0];
  assert.deepEqual(dependency.path, ["ssm-secrets-reference", "secrets-read"]);
  assert.equal(dependency.viaService, "secretsmanager.${Region}.amazonaws.com");
  assert.equal(dependency.resourceRole, "secret key");
  assert.equal(dependency.sources[0].evidence, "GetSecretValue calls Decrypt.");
});

test("unknown coverage remains distinct from reviewed absence", () => {
  const input = document();
  for (const coverage of ["unknown", "reviewed_no_dependencies"]) {
    input.fas[0] = { coverage, dependencies: [] };
    assert.equal(parseApiCatalog(input).apiMethods[0].fas.coverage, coverage);
  }
});

test("catalog rejects missing action IDs, dangling FAS entries, and indirect citations", () => {
  let input = document();
  input.api_methods[0][2] = [1, 2];
  assert.throws(() => parseApiCatalog(input), /omits a FAS permission/);
  input = document();
  input.fas = [];
  assert.throws(() => parseApiCatalog(input), /do not match/);
  input = document();
  input.fas[0].dependencies[0].rule.sources[0].url = "https://example.com/map.json";
  assert.throws(() => parseApiCatalog(input), /not AWS documentation/);
});

test("unresolved authorization stays distinct from documented empty permissions", () => {
  const input = document();
  input.api_methods[0][2] = null;
  let method = parseApiCatalog(input).apiMethods[0];
  assert.equal(method.authorizationCoverage, "unresolved");
  assert.deepEqual(method.permissionIds, []);
  assert.equal(method.fas.dependencies.length, 1);
  input.fas[0].dependencies[0].iam_permissions = ["kms:InventedPermission"];
  assert.throws(() => parseApiCatalog(input), /unknown FAS permission/);
  input.fas[0].dependencies[0].iam_permissions = ["kms:Decrypt"];
  input.fas[0] = null;
  method = parseApiCatalog(input).apiMethods[0];
  assert.equal(method.authorizationCoverage, "unresolved");
  assert.equal(method.fas, undefined);
  input.api_methods[0][2] = [];
  assert.throws(() => parseApiCatalog(input), /unresolved FAS authorization/);
  input.fas[0] = { coverage: "unknown", dependencies: [] };
  assert.equal(parseApiCatalog(input).apiMethods[0].authorizationCoverage, "not_required");
  input.schema_version = 1;
  assert.throws(() => parseApiCatalog(input), /unsupported schema/);
});
