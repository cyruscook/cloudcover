export const SCHEMA_VERSION = 1;
export const API_SCHEMA_VERSION = 2;

export type ApiMethod = {
  service: string;
  name: string;
  canonical: string;
  permissionIds: number[];
  authorizationCoverage: "mapped" | "not_required" | "unresolved";
  fas?: FasInfo;
  policyStatus: PolicyStatus;
};

export type PolicyStatus = {
  state: "blocked" | "unknown";
  message?: string;
  source?: string;
};

export type FasDependency = {
  ruleId: string;
  applicability: string;
  resourceRole: string;
  viaService?: string;
  sources: { url: string; evidence: string }[];
  path: string[];
  permissions: string[];
};

export type FasInfo = {
  coverage: "documented_dependencies" | "reviewed_no_dependencies" | "unknown";
  dependencies: FasDependency[];
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
  schemaVersion(document, "API catalog", API_SCHEMA_VERSION);
  const iamPermissions = strings(document.iam_permissions, "API IAM permissions");
  const iamPermissionNames = new Set(iamPermissions);
  const apiMethodsValue = array(document.api_methods, "API methods");
  const fasEntries = document.fas === undefined ? undefined : array(document.fas, "FAS entries");
  const policyEntries = document.policy_status === undefined ? undefined : array(document.policy_status, "Policy status entries");
  if (fasEntries && fasEntries.length !== apiMethodsValue.length) {
    throw new CatalogError("FAS entries do not match the API method index");
  }
  if (policyEntries && policyEntries.length !== apiMethodsValue.length) {
    throw new CatalogError("Policy status entries do not match the API method index");
  }
  const apiMethods = apiMethodsValue.map((row, index) => {
    const values = tuple(row, 3, `API method ${index}`);
    const service = string(values[0], `API method ${index} service`);
    const name = string(values[1], `API method ${index} operation`);
    const unresolved = values[2] === null;
    const policyEntry = policyEntries ? record(policyEntries[index], `API method ${index} policy status`) : undefined;
    const policyStatus = parsePolicyStatus(policyEntry);
    if (policyStatus.state === "blocked" && !unresolved) {
      throw new CatalogError(`Blocked API method ${index} must have a null authorization marker`);
    }
    if (policyEntry && policyStatus.state === "unknown" && unresolved) {
      throw new CatalogError(`Unknown policy status for API method ${index} contradicts its null authorization marker`);
    }
    const evidence = policyStatus.state === "blocked" ? policyEntry?.evidence_permission_ids : undefined;
    if (policyStatus.state === "blocked" && evidence === undefined) {
      throw new CatalogError(`Blocked API method ${index} is missing its evidence marker`);
    }
    const permissionIds = unresolved && (evidence === undefined || evidence === null) ? [] : integerArray(
      unresolved ? evidence : values[2],
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
    if (fasEntries?.[index] === null && !unresolved) {
      throw new CatalogError(`API method ${index} has unresolved FAS authorization without an unresolved permission marker`);
    }
    const fas = fasEntries && fasEntries[index] !== null ? parseFasInfo(fasEntries[index]) : undefined;
    if (fas && fas.dependencies.some(dependency => dependency.permissions.some(
      permission => !iamPermissionNames.has(permission),
    ))) throw new CatalogError(`API method ${index} references an unknown FAS permission`);
    if (!unresolved && fas && fas.dependencies.some(dependency => dependency.permissions.some(
      permission => !permissionIds.some(id => iamPermissions[id] === permission),
    ))) throw new CatalogError(`API method ${index} omits a FAS permission`);
    const authorizationCoverage: ApiMethod["authorizationCoverage"] = unresolved
      ? "unresolved" : permissionIds.length ? "mapped" : "not_required";
    return { service, name, canonical: `${service}.${name}`, permissionIds, authorizationCoverage, fas, policyStatus };
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

function parsePolicyStatus(entry: Record<string, unknown> | undefined): PolicyStatus {
  if (!entry) return { state: "unknown" };
  const state = string(entry.state, "Policy status state");
  if (state !== "blocked" && state !== "unknown") throw new CatalogError("Unknown policy status state");
  if (state === "unknown" && Object.hasOwn(entry, "evidence_permission_ids")) {
    throw new CatalogError("Unknown policy status cannot carry blocked permission evidence");
  }
  const message = entry.message === undefined ? undefined : string(entry.message, "Policy status message");
  if ((state === "blocked" && message === undefined) || (message !== undefined && !message.trim())) {
    throw new CatalogError("Policy status is missing a diagnostic");
  }
  const source = entry.source === undefined ? undefined : string(entry.source, "Policy status source");
  if (source !== undefined) {
    let url: URL;
    try { url = new URL(source); } catch { throw new CatalogError("Invalid policy status source URL"); }
    if (url.protocol !== "https:") throw new CatalogError("Policy status source must use HTTPS");
  }
  return { state, message, source };
}

function parseFasInfo(value: unknown): FasInfo {
  const entry = record(value, "FAS entry");
  const coverage = string(entry.coverage, "FAS coverage");
  if (!["documented_dependencies", "reviewed_no_dependencies", "unknown"].includes(coverage)) {
    throw new CatalogError("Unknown FAS coverage status");
  }
  const dependencies = array(entry.dependencies, "FAS dependencies").map(value => {
    const dependency = record(value, "FAS dependency");
    const rule = record(dependency.rule, "FAS rule");
    const sources = array(rule.sources, "FAS sources").map(value => {
      const source = record(value, "FAS source");
      const url = string(source.url, "FAS source URL");
      if (!url.startsWith("https://docs.aws.amazon.com/")) throw new CatalogError("FAS source is not AWS documentation");
      return { url, evidence: string(source.evidence, "FAS evidence") };
    });
    const viaService = rule.via_service === null ? undefined : string(rule.via_service, "KMS ViaService");
    return {
      ruleId: string(rule.id, "FAS rule ID"),
      applicability: string(rule.applicability, "FAS applicability"),
      resourceRole: string(rule.resource_role, "FAS resource role"),
      viaService, sources,
      path: strings(dependency.path, "FAS path"),
      permissions: strings(dependency.iam_permissions, "FAS permissions"),
    };
  });
  return { coverage: coverage as FasInfo["coverage"], dependencies };
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

function schemaVersion(document: Record<string, unknown>, context: string, expected = SCHEMA_VERSION): void {
  if (document.schema_version !== expected) {
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
