import type { ApiCatalog, TerraformRow } from "./catalog";

export type Kind = "api" | "iam" | "terraform";
export type Entry = { kind: Kind; label: string; normalized: string; id: number | string };
export const LIFECYCLES = ["create", "read", "update", "delete"] as const;
export type Lifecycle = typeof LIFECYCLES[number];
export type Mapping = {
  apiMethodId: number;
  permissionIds: number[];
  lifecycles: Lifecycle[];
};

// An action shared by several lifecycle steps should retain all its relationships
// without repeating the same action and permissions in separate lists.
export function resourceMappings(rows: TerraformRow[], catalog: ApiCatalog): Mapping[] {
  const actions = new Map<number, Set<Lifecycle>>();
  for (const lifecycle of LIFECYCLES) {
    for (const row of rows.filter(row => row.lifecycle === lifecycle)) {
      for (const id of row.apiMethodIds) {
        const steps = actions.get(id) ?? new Set<Lifecycle>();
        steps.add(lifecycle);
        actions.set(id, steps);
      }
    }
  }
  return [...actions].sort(([left], [right]) => left - right).map(([id, steps]) => ({
    apiMethodId: id,
    permissionIds: catalog.apiMethods[id].permissionIds,
    lifecycles: LIFECYCLES.filter(step => steps.has(step)),
  }));
}

export function filterMappings(
  mappings: Mapping[], catalog: ApiCatalog, query: string, lifecycle?: Lifecycle,
): Mapping[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const result: Mapping[] = [];
  for (const mapping of mappings) {
    if (lifecycle && !mapping.lifecycles.includes(lifecycle)) continue;
    const action = catalog.apiMethods[mapping.apiMethodId].canonical.toLowerCase();
    const matches = (permission = "") => terms.every(term => `${action} ${permission}`.includes(term));
    if (!mapping.permissionIds.length) {
      if (matches()) result.push(mapping);
      continue;
    }
    const permissionIds = mapping.permissionIds.filter(id => matches(catalog.iamPermissions[id].toLowerCase()));
    if (permissionIds.length) result.push({ ...mapping, permissionIds });
  }
  return result;
}

export function mappingPermissionIds(mappings: Mapping[]): number[] {
  return [...new Set(mappings.flatMap(mapping => mapping.permissionIds))].sort((left, right) => left - right);
}
