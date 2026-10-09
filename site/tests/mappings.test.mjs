import assert from "node:assert/strict";
import { test } from "node:test";
import { filterMappings, mappingPermissionIds, resourceMappings } from "../src/lib/mappings.ts";

const catalog = {
  iamPermissions: ["iam:PassRole", "s3:GetObject", "s3:PutObject"],
  apiMethods: [
    { canonical: "s3.GetObject", permissionIds: [1] },
    { canonical: "s3.HeadBucket", permissionIds: [] },
    { canonical: "s3.PutObject", permissionIds: [0, 2] },
  ],
};
const rows = [
  { lifecycle: "create", apiMethodIds: [0, 2] },
  { lifecycle: "read", apiMethodIds: [0, 1] },
  { lifecycle: "read", apiMethodIds: [0] },
  { lifecycle: "update", apiMethodIds: [2] },
  { lifecycle: "delete", apiMethodIds: [] },
];
const mappings = resourceMappings(rows, catalog);

test("shared actions retain every lifecycle and permission without duplicating actions", () => {
  assert.deepEqual(mappings, [
    { apiMethodId: 0, permissionIds: [1], lifecycles: ["create", "read"] },
    { apiMethodId: 1, permissionIds: [], lifecycles: ["read"] },
    { apiMethodId: 2, permissionIds: [0, 2], lifecycles: ["create", "update"] },
  ]);
  assert.deepEqual(rows[1].apiMethodIds, [0, 1]);
});

test("a permission search keeps only its actual API relationships", () => {
  const filtered = filterMappings(mappings, catalog, "PASSROLE");
  assert.deepEqual(filtered, [
    { apiMethodId: 2, permissionIds: [0], lifecycles: ["create", "update"] },
  ]);
  assert.deepEqual(mappingPermissionIds(filtered), [0]);
  assert.deepEqual(mappings[2].permissionIds, [0, 2]);
});

test("action and permission terms must match the same relationship", () => {
  assert.deepEqual(filterMappings(mappings, catalog, "s3.PutObject iam:PassRole"), [
    { apiMethodId: 2, permissionIds: [0], lifecycles: ["create", "update"] },
  ]);
  assert.deepEqual(filterMappings(mappings, catalog, "iam:PassRole s3:PutObject"), []);
});

test("lifecycle and text filters combine without losing actions with unknown permissions", () => {
  assert.deepEqual(filterMappings(mappings, catalog, "   ", "read"), mappings.slice(0, 2));
  assert.deepEqual(filterMappings(mappings, catalog, "headbucket", "read"), [mappings[1]]);
  assert.deepEqual(filterMappings(mappings, catalog, "putobject", "read"), []);
  assert.deepEqual(filterMappings(mappings, catalog, "", "delete"), []);
});

test("copying uses the unique permission union of the filtered relationships", () => {
  assert.deepEqual(mappingPermissionIds([...mappings, mappings[0]]), [0, 1, 2]);
  assert.deepEqual(mappingPermissionIds(filterMappings(mappings, catalog, "", "read")), [1]);
  assert.deepEqual(mappingPermissionIds([mappings[1]]), []);
});
