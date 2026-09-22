export const SCHEMA_VERSION = 1;

export type ApiMethod = {
  service: string;
  name: string;
  canonical: string;
  permissionIds: number[];
};

export type ApiCatalog = {
  iamPermissions: string[];
  apiMethods: ApiMethod[];
};

export type TerraformIndex = {
  latest: string;
  versions: string[];
};

export type TerraformRow = {
  resource: string;
  lifecycle: string;
  apiMethodIds: number[];
};

export type TerraformSnapshot = {
  version: string;
  rowIds: number[];
};

export class CatalogError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CatalogError";
  }
}

export function parseApiCatalog(value: unknown): ApiCatalog {
  const document = record(value, "API catalog");
  schemaVersion(document, "API catalog");
  const iamPermissions = strings(document.iam_permissions, "API IAM permissions");
  const apiMethodsValue = array(document.api_methods, "API methods");
  const apiMethods = apiMethodsValue.map((row, index) => {
    const values = tuple(row, 3, `API method ${index}`);
    const service = string(values[0], `API method ${index} service`);
    const name = string(values[1], `API method ${index} operation`);
    const permissionIds = integerArray(
      values[2],
      `API method ${index} IAM permission IDs`,
    );
    for (const permissionId of permissionIds) {
      if (permissionId >= iamPermissions.length) {
        throw new CatalogError(
          `API method ${index} references missing IAM permission ${permissionId}`,
        );
      }
    }
    if (!strictlyIncreasing(permissionIds)) {
      throw new CatalogError(`API method ${index} has unsorted IAM permission IDs`);
    }
    return { service, name, canonical: `${service}.${name}`, permissionIds };
  });

  if (
    !strictlyIncreasingBy(
      apiMethods,
      (method) => `${method.service}\u0000${method.name}`,
    ) ||
    !strictlyIncreasingStrings(iamPermissions)
  ) {
    throw new CatalogError("API catalog indexes are not sorted and unique");
  }

  return { iamPermissions, apiMethods };
}

export function parseTerraformIndex(value: unknown): TerraformIndex {
  const document = record(value, "Terraform index");
  schemaVersion(document, "Terraform index");
  const versions = strings(document.versions, "Terraform versions");
  const latest = string(document.latest, "Terraform latest version");
  if (versions.length === 0 || versions[versions.length - 1] !== latest) {
    throw new CatalogError("Terraform latest version is not the last indexed version");
  }
  if (new Set(versions).size !== versions.length) {
    throw new CatalogError("Terraform versions are not unique");
  }
  return { latest, versions };
}

export function parseTerraformRows(
  value: unknown,
  apiMethodCount: number,
): TerraformRow[] {
  const document = record(value, "Terraform rows");
  schemaVersion(document, "Terraform rows");
  const rows = array(document.rows, "Terraform rows").map((row, index) => {
    const values = tuple(row, 3, `Terraform row ${index}`);
    const resource = string(values[0], `Terraform row ${index} resource`);
    const lifecycle = string(values[1], `Terraform row ${index} lifecycle`);
    const apiMethodIds = integerArray(
      values[2],
      `Terraform row ${index} API method IDs`,
    );
    for (const apiMethodId of apiMethodIds) {
      if (apiMethodId >= apiMethodCount) {
        throw new CatalogError(
          `Terraform row ${index} references missing API method ${apiMethodId}`,
        );
      }
    }
    if (!strictlyIncreasing(apiMethodIds)) {
      throw new CatalogError(`Terraform row ${index} has unsorted API method IDs`);
    }
    return { resource, lifecycle, apiMethodIds };
  });
  if (!strictlyIncreasingTerraformRows(rows)) {
    throw new CatalogError("Terraform rows are not sorted and unique");
  }
  return rows;
}

export function parseTerraformSnapshot(
  value: unknown,
  expectedVersion: string,
  rowCount: number,
): TerraformSnapshot {
  const document = record(value, `Terraform ${expectedVersion} snapshot`);
  schemaVersion(document, `Terraform ${expectedVersion} snapshot`);
  const version = string(document.version, "Terraform snapshot version");
  if (version !== expectedVersion) {
    throw new CatalogError(
      `Terraform snapshot version ${version} does not match ${expectedVersion}`,
    );
  }
  const rowIds = integerArray(document.row_ids, "Terraform snapshot row IDs");
  for (const rowId of rowIds) {
    if (rowId >= rowCount) {
      throw new CatalogError(`Terraform snapshot references missing row ${rowId}`);
    }
  }
  if (!strictlyIncreasing(rowIds)) {
    throw new CatalogError("Terraform snapshot row IDs are not sorted and unique");
  }
  return { version, rowIds };
}

export function dataUrl(baseUrl: string, path: string): string {
  const normalizedBase = baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`;
  return `${normalizedBase}${path}`;
}

function record(value: unknown, context: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new CatalogError(`${context} is not an object`);
  }
  return value as Record<string, unknown>;
}

function schemaVersion(document: Record<string, unknown>, context: string): void {
  if (document.schema_version !== SCHEMA_VERSION) {
    throw new CatalogError(`${context} has unsupported schema version`);
  }
}

function array(value: unknown, context: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new CatalogError(`${context} is not an array`);
  }
  return value;
}

function tuple(value: unknown, length: number, context: string): unknown[] {
  const values = array(value, context);
  if (values.length !== length) {
    throw new CatalogError(`${context} has the wrong shape`);
  }
  return values;
}

function string(value: unknown, context: string): string {
  if (typeof value !== "string") {
    throw new CatalogError(`${context} is not a string`);
  }
  return value;
}

function strings(value: unknown, context: string): string[] {
  return array(value, context).map((entry, index) =>
    string(entry, `${context} ${index}`),
  );
}

function integerArray(value: unknown, context: string): number[] {
  return array(value, context).map((entry, index) => {
    if (
      typeof entry !== "number" ||
      !Number.isSafeInteger(entry) ||
      entry < 0
    ) {
      throw new CatalogError(`${context} ${index} is not a non-negative integer`);
    }
    return entry;
  });
}

function strictlyIncreasing(values: number[]): boolean {
  return values.every((value, index) => index === 0 || values[index - 1] < value);
}

function strictlyIncreasingStrings(values: string[]): boolean {
  return values.every((value, index) => index === 0 || values[index - 1] < value);
}

function strictlyIncreasingTerraformRows(rows: TerraformRow[]): boolean {
  return rows.every(
    (row, index) =>
      index === 0 || terraformRowPrecedes(rows[index - 1], row),
  );
}

function terraformRowPrecedes(
  left: TerraformRow,
  right: TerraformRow,
): boolean {
  if (left.resource !== right.resource) {
    return left.resource < right.resource;
  }
  if (left.lifecycle !== right.lifecycle) {
    return left.lifecycle < right.lifecycle;
  }
  const length = Math.min(left.apiMethodIds.length, right.apiMethodIds.length);
  for (let index = 0; index < length; index += 1) {
    if (left.apiMethodIds[index] !== right.apiMethodIds[index]) {
      return left.apiMethodIds[index] < right.apiMethodIds[index];
    }
  }
  return left.apiMethodIds.length < right.apiMethodIds.length;
}

function strictlyIncreasingBy<T>(values: T[], key: (value: T) => string): boolean {
  return values.every(
    (value, index) => index === 0 || key(values[index - 1]) < key(value),
  );
}
