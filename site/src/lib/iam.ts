import { CatalogError, SCHEMA_VERSION } from "./catalog.ts";

export type IamResource = { name: string; arnFormats: string[] };
export type ContextKey = {
  name: string;
  types: string[];
  permission: boolean;
  permissionResources: string[];
  resourceTypes: string[];
};
export type IamPermission = {
  permission: string;
  resources: IamResource[];
  contextKeys: ContextKey[];
};

export function parseIamService(
  value: unknown,
  service: string,
): Map<string, IamPermission> {
  const document = record(value);
  if (
    document.schema_version !== SCHEMA_VERSION ||
    !Array.isArray(document.permissions)
  ) {
    throw new CatalogError("Unsupported IAM reference document");
  }
  const permissions = new Map<string, IamPermission>();
  for (const entry of document.permissions) {
    const row = record(entry);
    const permission = string(row.permission);
    if (!permission.startsWith(`${service}:`) || permissions.has(permission)) {
      throw new CatalogError("Invalid or duplicate IAM reference permission");
    }
    const resources = array(row.resources).map((entry) => {
      const resource = record(entry);
      return {
        name: string(resource.name),
        arnFormats: strings(resource.arn_formats),
      };
    });
    const names = new Set(resources.map((resource) => resource.name));
    if (names.size !== resources.length)
      throw new CatalogError("Duplicate IAM resource type");
    const contextKeys = array(row.context_keys).map((entry) => {
      const key = record(entry);
      if (typeof key.permission !== "boolean")
        throw new CatalogError("Invalid context key applicability");
      const permissionResources = strings(key.permission_resources);
      const resourceTypes = strings(key.resource_types);
      if (
        [...permissionResources, ...resourceTypes].some(
          (name) => !names.has(name),
        )
      ) {
        throw new CatalogError(
          "Context key references an unknown resource type",
        );
      }
      if (
        !key.permission &&
        !permissionResources.length &&
        !resourceTypes.length
      ) {
        throw new CatalogError("Context key has no applicability");
      }
      return {
        name: string(key.name),
        types: strings(key.types),
        permission: key.permission,
        permissionResources,
        resourceTypes,
      };
    });
    if (
      new Set(contextKeys.map((key) => key.name)).size !== contextKeys.length
    ) {
      throw new CatalogError("Duplicate IAM context key");
    }
    permissions.set(permission, { permission, resources, contextKeys });
  }
  return permissions;
}

export function filterContextKeys(
  keys: ContextKey[],
  query: string,
  resource = "",
): ContextKey[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return keys.filter((key) => {
    const scopes = [...key.permissionResources, ...key.resourceTypes];
    const text =
      `${key.name} ${key.types.join(" ")} ${key.permission ? "permission" : ""} ${scopes.join(" ")}`.toLowerCase();
    return (
      (!resource || key.permission || scopes.includes(resource)) &&
      terms.every((term) => text.includes(term))
    );
  });
}

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new CatalogError("Invalid IAM reference record");
  return value as Record<string, unknown>;
}
function array(value: unknown): unknown[] {
  if (!Array.isArray(value))
    throw new CatalogError("Invalid IAM reference array");
  return value;
}
function string(value: unknown): string {
  if (typeof value !== "string" || !value)
    throw new CatalogError("Invalid IAM reference string");
  return value;
}
function strings(value: unknown): string[] {
  return array(value).map(string);
}
