import assert from "node:assert/strict";
import { test } from "node:test";
import { parseIamService, filterContextKeys } from "../src/lib/iam.ts";

const document = {
  schema_version: 1,
  permissions: [
    {
      permission: "ec2:RunInstances",
      resources: [
        {
          name: "instance",
          arn_formats: [
            "arn:${Partition}:ec2:${Region}:${Account}:instance/${InstanceId}",
          ],
        },
      ],
      context_keys: [
        {
          name: "ec2:Region",
          types: ["String"],
          permission: true,
          permission_resources: [],
          resource_types: [],
        },
        {
          name: "ec2:InstanceType",
          types: ["String"],
          permission: false,
          permission_resources: ["instance"],
          resource_types: [],
        },
        {
          name: "aws:ResourceTag/${TagKey}",
          types: ["String"],
          permission: false,
          permission_resources: [],
          resource_types: ["instance"],
        },
      ],
    },
  ],
};
const permission = parseIamService(document, "ec2").get("ec2:RunInstances");

test("SAR parsing preserves ARN placeholders and all three context key scopes", () => {
  assert.equal(
    permission.resources[0].arnFormats[0],
    document.permissions[0].resources[0].arn_formats[0],
  );
  assert.deepEqual(
    permission.contextKeys.map((key) => [
      key.permission,
      key.permissionResources,
      key.resourceTypes,
    ]),
    [
      [true, [], []],
      [false, ["instance"], []],
      [false, [], ["instance"]],
    ],
  );
});

test("resource filtering includes permission keys and only the selected resource's keys", () => {
  const keys = [
    ...permission.contextKeys,
    {
      name: "ec2:Subnet",
      types: ["ARN"],
      permission: false,
      permissionResources: ["subnet"],
      resourceTypes: [],
    },
  ];
  assert.deepEqual(
    filterContextKeys(keys, "", "instance"),
    permission.contextKeys,
  );
  assert.deepEqual(
    filterContextKeys(keys, "string INSTANCE", "instance"),
    permission.contextKeys.slice(1),
  );
  assert.deepEqual(filterContextKeys(keys, "region", "instance"), [keys[0]]);
  assert.deepEqual(filterContextKeys(keys, "does not exist"), []);
  assert.equal(keys.length, 4);
});

test("a wildcard permission can still have context keys and unknown value types", () => {
  const parsed = parseIamService(
    {
      schema_version: 1,
      permissions: [
        {
          permission: "ec2:DescribeInstances",
          resources: [],
          context_keys: [
            document.permissions[0].context_keys[0],
            {
              name: "ec2:Unknown",
              types: [],
              permission: true,
              permission_resources: [],
              resource_types: [],
            },
          ],
        },
      ],
    },
    "ec2",
  );
  assert.equal(parsed.get("ec2:DescribeInstances").resources.length, 0);
  assert.deepEqual(
    parsed.get("ec2:DescribeInstances").contextKeys[1].types,
    [],
  );
});

test("invalid SAR data is rejected rather than rendered with misleading applicability", () => {
  const invalid = [
    { ...document, schema_version: 2 },
    {
      ...document,
      permissions: [...document.permissions, ...document.permissions],
    },
    {
      ...document,
      permissions: [{ ...document.permissions[0], permission: "s3:GetObject" }],
    },
    {
      ...document,
      permissions: [
        {
          ...document.permissions[0],
          resources: [
            ...document.permissions[0].resources,
            ...document.permissions[0].resources,
          ],
        },
      ],
    },
    {
      ...document,
      permissions: [
        {
          ...document.permissions[0],
          context_keys: [
            {
              ...document.permissions[0].context_keys[1],
              permission_resources: ["missing"],
            },
          ],
        },
      ],
    },
    {
      ...document,
      permissions: [
        {
          ...document.permissions[0],
          context_keys: [
            { ...document.permissions[0].context_keys[0], permission: "false" },
          ],
        },
      ],
    },
    {
      ...document,
      permissions: [
        {
          ...document.permissions[0],
          context_keys: [
            { ...document.permissions[0].context_keys[0], permission: false },
          ],
        },
      ],
    },
  ];
  for (const value of invalid)
    assert.throws(() => parseIamService(value, "ec2"));
});
