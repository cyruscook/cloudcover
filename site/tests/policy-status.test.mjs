import assert from "node:assert/strict";
import { test } from "node:test";
import { parseApiCatalog } from "../src/lib/catalog.ts";
import { authorizationResolved, filterMappings, resourceMappings } from "../src/lib/mappings.ts";

function document() {
  return {
    schema_version: 2,
    iam_permissions: ["iam:PassRole", "s3:GetObject"],
    api_methods: [["cognito-identity", "SetIdentityPoolRoles", null], ["s3", "GetObject", [1]]],
    policy_status: [
      { state: "blocked", message: "Review the concrete requested role and service.", source: "https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles.html", evidence_permission_ids: [0] },
      { state: "unknown" },
    ],
  };
}

test("blocked operations retain independent evidence and disable mixed or filtered copying", () => {
  const catalog = parseApiCatalog(document());
  const mappings = resourceMappings([{ lifecycle: "create", apiMethodIds: [0, 1] }], catalog);
  assert.equal(catalog.apiMethods[0].policyStatus.state, "blocked");
  assert.equal(catalog.apiMethods[0].authorizationCoverage, "unresolved");
  assert.deepEqual(catalog.apiMethods[0].permissionIds, [0]);
  assert.equal(authorizationResolved(mappings, catalog), false);
  assert.equal(authorizationResolved(filterMappings(mappings, catalog, "PassRole"), catalog), false);
  assert.equal(authorizationResolved(filterMappings(mappings, catalog, "GetObject"), catalog), true);
  assert.equal(catalog.apiMethods[1].policyStatus.state, "unknown");
});

test("older clients see null and absent status remains unknown without asserting review", () => {
  const input = document();
  const legacy = structuredClone(input);
  delete legacy.policy_status;
  const catalog = parseApiCatalog(legacy);
  assert.equal(input.api_methods[0][2], null);
  assert.deepEqual(catalog.apiMethods[0].permissionIds, []);
  assert.deepEqual(catalog.apiMethods[0].policyStatus, { state: "unknown" });
  assert.deepEqual(catalog.apiMethods[1].policyStatus, { state: "unknown" });
  assert.equal(catalog.apiMethods[0].authorizationCoverage, "unresolved");
});

test("empty recorded evidence cannot turn a blocked operation into permissionless success", () => {
  const input = document();
  input.policy_status[0].evidence_permission_ids = [];
  const catalog = parseApiCatalog(input);
  assert.equal(catalog.apiMethods[0].authorizationCoverage, "unresolved");
  assert.deepEqual(catalog.apiMethods[0].permissionIds, []);
  assert.equal(authorizationResolved([{ apiMethodId: 0, permissionIds: [], lifecycles: [] }], catalog), false);
});

test("retained lifecycle warnings and documented empty evidence remain nonblocking references", () => {
  const input = document();
  input.api_methods[0][2] = [];
  input.policy_status[0] = { state: "unknown", message: "Deprecated API; exact historical mapping retained." };
  const catalog = parseApiCatalog(input);
  assert.equal(catalog.apiMethods[0].authorizationCoverage, "not_required");
  assert.equal(catalog.apiMethods[0].policyStatus.state, "unknown");
  assert.equal(authorizationResolved([{ apiMethodId: 0, permissionIds: [], lifecycles: [] }], catalog), true);
});

test("present unknown records cannot hide a null marker or carry blocked-only evidence", () => {
  for (const evidence of [null, [], [0], undefined]) {
    const input = document();
    input.policy_status[1].evidence_permission_ids = evidence;
    assert.throws(() => parseApiCatalog(input), /cannot carry blocked permission evidence/);
  }
  const input = document();
  input.api_methods[1][2] = null;
  assert.throws(() => parseApiCatalog(input), /contradicts its null authorization marker/);
  delete input.policy_status;
  const legacy = parseApiCatalog(input);
  assert.equal(legacy.apiMethods[1].policyStatus.state, "unknown");
  assert.equal(legacy.apiMethods[1].authorizationCoverage, "unresolved");
  assert.equal(authorizationResolved([{ apiMethodId: 1, permissionIds: [], lifecycles: [] }], legacy), false);
});

test("whitespace-only diagnostics cannot stand in for an actionable message", () => {
  for (const message of [" ", "\t\n", " \r\n "]) {
    const input = document();
    input.policy_status[0].message = message;
    assert.throws(() => parseApiCatalog(input), /missing a diagnostic/);
  }
  const input = document();
  input.policy_status[1].message = " \n ";
  assert.throws(() => parseApiCatalog(input), /missing a diagnostic/);
});

test("malformed status, evidence or citations cannot silently downgrade blockers", () => {
  for (const mutate of [
    v => { v.policy_status = []; },
    v => { v.policy_status = null; },
    v => { v.policy_status[0] = null; },
    v => { v.policy_status[0].state = "approved"; },
    v => { v.policy_status[0].message = ""; },
    v => { delete v.policy_status[0].message; },
    v => { v.api_methods[0][2] = [0]; },
    v => { delete v.policy_status[0].evidence_permission_ids; },
    v => { v.policy_status[0].evidence_permission_ids = [99]; },
    v => { v.policy_status[0].evidence_permission_ids = [0, 0]; },
    v => { v.policy_status[0].evidence_permission_ids = [1, 0]; },
    v => { v.policy_status[0].source = "javascript:alert(1)"; },
    v => { v.policy_status[0].source = "not a URL"; },
  ]) {
    const input = document(); mutate(input);
    assert.throws(() => parseApiCatalog(input));
  }
});
