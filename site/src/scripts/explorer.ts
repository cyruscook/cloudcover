import {
  type ApiCatalog, type TerraformIndex, type TerraformRow, type TerraformSnapshot,
  CatalogError, dataUrl, parseApiCatalog, parseTerraformIndex,
  parseTerraformRows, parseTerraformSnapshot,
} from "../lib/catalog";

type Kind = "api" | "iam" | "terraform";
type Entry = { kind: Kind; label: string; normalized: string; id: number | string };
type ViewRoute = { kind: Kind; id: string; version?: string };
const labels: Record<Kind, string> = { api: "API action", iam: "IAM permission", terraform: "Terraform" };
function element<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new CatalogError(`Missing explorer element ${id}`);
  return node as T;
}
const root = element("explorer");
const input = element<HTMLInputElement>("search");
const suggestions = element("suggestions");
const feedback = element("search-feedback");
const status = element("load-status");
const versionSelect = element<HTMLSelectElement>("terraform-version");
const detail = element("detail");
const welcome = element("welcome");
const clear = element<HTMLButtonElement>("clear-search");
let catalog: ApiCatalog;
let index: TerraformIndex;
let rows: TerraformRow[];
let snapshot: TerraformSnapshot | undefined;
let entries: Entry[] = [];
let matches: Entry[] = [];
let selected: Entry | undefined;
let active = -1;
let versionToken = 0;
const snapshots = new Map<string, TerraformSnapshot>();
let routeToken = 0;
const methodsByPermission = new Map<number, number[]>();
const rowsByResource = new Map<string, TerraformRow[]>();

async function fetchDocument(path: string): Promise<unknown> {
  const response = await fetch(dataUrl(root.dataset.baseUrl ?? "./", path));
  if (!response.ok) throw new CatalogError(`Could not load ${path} (${response.status})`);
  return response.json();
}
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text: string, className = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  result.className = className;
  return result;
}
function setStatus(message: string, error = false): void {
  status.replaceChildren(document.createTextNode(message));
  status.classList.toggle("error", error);
}
function retry(message: string, action: () => void): void {
  setStatus(message, true);
  const button = node("button", "Retry", "retry-button");
  button.type = "button";
  button.addEventListener("click", action);
  status.append(button);
}
function entry(kind: Kind, label: string, id: number | string): Entry {
  return { kind, label, id, normalized: label.toLowerCase() };
}
function readRoute(): ViewRoute | undefined {
  const parameters = new URLSearchParams(window.location.search);
  const kind = parameters.get("view");
  const id = parameters.get("id");
  if ((kind !== "api" && kind !== "iam" && kind !== "terraform") || !id) return undefined;
  const version = parameters.get("version") || undefined;
  return { kind, id, version };
}
function routeUrl(item: Entry): URL {
  const url = new URL(window.location.href);
  url.searchParams.set("view", item.kind);
  url.searchParams.set("id", item.label);
  if (item.kind === "terraform") url.searchParams.set("version", versionSelect.value);
  else url.searchParams.delete("version");
  return url;
}
function writeRoute(item: Entry): void {
  const url = routeUrl(item);
  if (url.href !== window.location.href) window.history.pushState(null, "", url);
}
function goHome(): void {
  const url = new URL(window.location.href);
  url.searchParams.delete("view");
  url.searchParams.delete("id");
  url.searchParams.delete("version");
  if (url.href !== window.location.href) window.history.pushState(null, "", url);
  showWelcome();
}
function dismiss(): void {
  matches = [];
  suggestions.hidden = true;
  input.setAttribute("aria-expanded", "false");
  input.removeAttribute("aria-activedescendant");
  active = -1;
}
function updateActive(): void {
  suggestions.querySelectorAll(".suggestion").forEach((child, i) => child.setAttribute("aria-selected", String(i === active)));
  if (active < 0) input.removeAttribute("aria-activedescendant");
  else {
    input.setAttribute("aria-activedescendant", `suggestion-${active}`);
    document.getElementById(`suggestion-${active}`)?.scrollIntoView({ block: "nearest" });
  }
}
function search(): void {
  const query = input.value.trim().toLowerCase();
  clear.hidden = input.value.length === 0;
  if (!query || input.disabled) { dismiss(); return; }
  const terms = query.split(/\s+/);
  const groups = (["api", "iam", "terraform"] as const).map(kind => entries
    .filter(item => item.kind === kind && terms.every(term => item.normalized.includes(term)))
    .sort((a, b) => Number(b.normalized === query) - Number(a.normalized === query)
      || Number(b.normalized.startsWith(query)) - Number(a.normalized.startsWith(query))
      || a.label.localeCompare(b.label)));
  const total = groups.reduce((sum, group) => sum + group.length, 0);
  // Reserve space for every matching type so broad queries do not hide Terraform.
  matches = [];
  for (let rank = 0; rank < 8; rank++) {
    for (const group of groups) if (group[rank]) matches.push(group[rank]);
  }
  active = -1;
  suggestions.replaceChildren();
  for (const [i, item] of matches.entries()) {
    const option = node("li", "", "suggestion");
    option.id = `suggestion-${i}`;
    option.setAttribute("role", "option");
    option.setAttribute("aria-selected", "false");
    const text = node("span", "", "suggestion-label");
    const start = item.normalized.indexOf(query);
    if (start >= 0) text.append(item.label.slice(0, start), node("mark", item.label.slice(start, start + query.length)), item.label.slice(start + query.length));
    else text.textContent = item.label;
    const badge = node("span", labels[item.kind], "type-badge");
    badge.dataset.kind = item.kind;
    option.append(text, badge);
    option.addEventListener("pointerdown", event => event.preventDefault());
    option.addEventListener("click", () => select(item));
    option.addEventListener("pointermove", () => { active = i; updateActive(); });
    suggestions.append(option);
  }
  if (!matches.length) {
    const empty = node("li", "No matches. Try a service name, action, or resource.", "suggestion-empty");
    empty.setAttribute("role", "option");
    empty.setAttribute("aria-disabled", "true");
    suggestions.append(empty);
  }
  feedback.textContent = total > matches.length
    ? `${total.toLocaleString()} matches. Showing up to 8 per type; refine your search for more.`
    : `${total} matches. Use arrow keys to choose and Enter to open.`;
  if (total > matches.length) {
    const hint = node("li", "Showing up to 8 per type. Keep typing to narrow your search.", "suggestion-empty");
    hint.setAttribute("role", "presentation");
    suggestions.append(hint);
  }
  suggestions.hidden = false;
  input.setAttribute("aria-expanded", "true");
  input.removeAttribute("aria-activedescendant");
}
function mappingCard(title: string, items: Entry[], empty: string): HTMLElement {
  const section = node("section", "", "mapping-card");
  const heading = node("h2", title);
  heading.append(node("span", String(items.length), "count"));
  section.append(heading);
  if (!items.length) section.append(node("p", empty, "empty-mapping"));
  else {
    const list = node("ul", "", "result-list");
    for (const item of items) {
      const li = node("li", "");
      const button = node("button", item.label, "mapping-link");
      button.type = "button";
      button.addEventListener("click", () => select(item));
      li.append(button);
      list.append(li);
    }
    section.append(list);
  }
  return section;
}
function apiEntry(id: number): Entry { return entry("api", catalog.apiMethods[id].canonical, id); }
function iamEntry(id: number): Entry { return entry("iam", catalog.iamPermissions[id], id); }
function renderDetail(item: Entry, focus: boolean): void {
  welcome.hidden = true;
  detail.hidden = false;
  detail.replaceChildren();
  const header = node("div", "", "detail-header");
  header.append(node("p", labels[item.kind], "eyebrow"), node("h1", item.label));
  const description = item.kind === "api" ? "IAM permissions associated with this API action."
    : item.kind === "iam" ? "API actions associated with this IAM permission."
    : `Known API actions by resource lifecycle · AWS provider ${snapshot?.version ?? versionSelect.value}`;
  header.append(node("p", description, "detail-description"));
  detail.append(header);
  const grid = node("div", "", "detail-grid");
  if (item.kind === "api") {
    grid.append(mappingCard("IAM permissions", catalog.apiMethods[Number(item.id)].permissionIds.map(iamEntry), "No IAM permissions mapped in the catalog."));
  } else if (item.kind === "iam") {
    grid.append(mappingCard("API actions", (methodsByPermission.get(Number(item.id)) ?? []).map(apiEntry), "No API actions mapped in the catalog."));
  } else {
    const resourceRows = rowsByResource.get(item.label) ?? [];
    if (!resourceRows.length) {
      grid.append(node("p", "This resource has no known mappings in this provider version.", "empty-mapping"));
    } else {
      const permissions = new Set<number>();
      for (const lifecycle of ["create", "read", "update", "delete"]) {
        const apiIds = new Set(resourceRows.filter(row => row.lifecycle === lifecycle).flatMap(row => row.apiMethodIds));
        for (const id of apiIds) for (const permission of catalog.apiMethods[id].permissionIds) permissions.add(permission);
        grid.append(mappingCard(`${lifecycle[0].toUpperCase()}${lifecycle.slice(1)} API actions`, [...apiIds].sort((a, b) => a - b).map(apiEntry), "No known API actions for this lifecycle step."));
      }
      grid.append(mappingCard("Combined IAM permissions", [...permissions].sort((a, b) => a - b).map(iamEntry), "No IAM permissions reached by the known API actions."));
    }
  }
  detail.append(grid, node("p", "Mappings reflect CloudCover’s indexed data, not a complete permissions policy. Required permissions can depend on request parameters and resource configuration.", "detail-description"));
  if (focus) { detail.focus({ preventScroll: true }); detail.scrollIntoView({ block: "start" }); }
}
function showWelcome(): void {
  selected = undefined;
  input.value = "";
  clear.hidden = true;
  dismiss();
  detail.hidden = true;
  detail.replaceChildren();
  welcome.hidden = false;
}
function showMissing(route: ViewRoute): void {
  selected = undefined;
  welcome.hidden = true;
  detail.hidden = false;
  input.value = route.id;
  clear.hidden = false;
  detail.replaceChildren(
    node("p", labels[route.kind], "eyebrow"),
    node("h1", route.id),
    node("p", `This ${labels[route.kind].toLowerCase()} is not available in the current catalog.`, "empty-mapping"),
  );
}
function select(item: Entry, focus = true, updateHistory = true): void {
  selected = item;
  input.value = item.label;
  clear.hidden = false;
  dismiss();
  if (updateHistory) writeRoute(item);
  renderDetail(item, focus);
  feedback.textContent = `${labels[item.kind]} ${item.label} selected.`;
}
async function restoreRoute(focus: boolean): Promise<void> {
  const token = ++routeToken;
  const route = readRoute();
  if (!route) { showWelcome(); return; }
  if (route.kind === "terraform" && route.version) {
    if (!index.versions.includes(route.version)) { showMissing(route); return; }
    if (versionSelect.value !== route.version) {
      versionSelect.value = route.version;
      await loadVersion(route.version);
      if (token !== routeToken) return;
    }
  }
  const item = entries.find(candidate => candidate.kind === route.kind && candidate.label === route.id);
  if (item) select(item, focus, false);
  else showMissing(route);
}
async function loadVersion(version: string): Promise<void> {
  const token = ++versionToken;
  snapshot = undefined;
  rowsByResource.clear();
  entries = entries.filter(item => item.kind !== "terraform");
  dismiss();
  if (selected?.kind === "terraform") {
    detail.replaceChildren(node("p", `Loading AWS provider ${version}…`, "empty-mapping"));
  }
  setStatus(`Loading Terraform ${version}… API and IAM search is available.`);
  versionSelect.disabled = true;
  try {
    const loadedRows = rows ?? parseTerraformRows(await fetchDocument("data/terraform/rows.json"), catalog.apiMethods.length);
    const loadedSnapshot = snapshots.get(version) ?? parseTerraformSnapshot(await fetchDocument(`data/terraform/versions/${version}.json`), version, loadedRows.length);
    if (token !== versionToken) return;
    rows = loadedRows;
    snapshot = loadedSnapshot;
    snapshots.set(version, snapshot);
    for (const id of snapshot.rowIds) {
      const row = rows[id];
      const group = rowsByResource.get(row.resource);
      if (group) group.push(row); else rowsByResource.set(row.resource, [row]);
    }
    entries.push(...[...rowsByResource.keys()].sort().map(resource => entry("terraform", resource, resource)));
    setStatus(`${catalog.apiMethods.length.toLocaleString()} API actions · ${catalog.iamPermissions.length.toLocaleString()} IAM permissions · ${rowsByResource.size.toLocaleString()} Terraform resources`);
    if (selected?.kind === "terraform") renderDetail(selected, false);
    if (document.activeElement === input) search();
  } catch (error) {
    if (token !== versionToken) return;
    retry(`Terraform data unavailable. API and IAM search is available. ${error instanceof Error ? error.message : ""}`, () => void loadVersion(version));
    if (selected?.kind === "terraform") detail.replaceChildren(node("p", "Terraform mappings could not be loaded. Use Retry above.", "empty-mapping"));
  } finally {
    if (token === versionToken) versionSelect.disabled = false;
  }
}
async function load(): Promise<void> {
  input.disabled = true;
  setStatus("Loading catalogs…");
  try {
    const [apiDocument, indexDocument] = await Promise.all([fetchDocument("data/api.json"), fetchDocument("data/terraform/index.json")]);
    catalog = parseApiCatalog(apiDocument);
    index = parseTerraformIndex(indexDocument);
    entries = [...catalog.apiMethods.map((_, id) => apiEntry(id)), ...catalog.iamPermissions.map((_, id) => iamEntry(id))];
    methodsByPermission.clear();
    catalog.apiMethods.forEach((method, id) => method.permissionIds.forEach(permission => {
      const methods = methodsByPermission.get(permission);
      if (methods) methods.push(id); else methodsByPermission.set(permission, [id]);
    }));
    versionSelect.replaceChildren(...index.versions.map(version => {
      const option = node("option", version);
      option.value = version;
      return option;
    }));
    const route = readRoute();
    const initialVersion = route?.kind === "terraform" && route.version && index.versions.includes(route.version)
      ? route.version : index.latest;
    versionSelect.value = initialVersion;
    input.disabled = false;
    await loadVersion(initialVersion);
    await restoreRoute(false);
  } catch (error) {
    retry(`Could not load catalogs. ${error instanceof Error ? error.message : ""}`, () => void load());
  }
}
input.addEventListener("input", search);
input.addEventListener("focus", search);
input.addEventListener("blur", dismiss);
input.addEventListener("keydown", event => {
  if (event.key === "Escape") { dismiss(); return; }
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (suggestions.hidden) search();
    if (!matches.length) return;
    active = active < 0 ? (event.key === "ArrowDown" ? 0 : matches.length - 1)
      : (active + (event.key === "ArrowDown" ? 1 : -1) + matches.length) % matches.length;
    updateActive();
  } else if (event.key === "Enter" && !suggestions.hidden && matches.length) {
    event.preventDefault();
    select(matches[active < 0 ? 0 : active]);
  }
});
clear.addEventListener("click", () => { goHome(); input.focus(); });
versionSelect.addEventListener("change", () => {
  void loadVersion(versionSelect.value).then(() => {
    if (selected?.kind === "terraform") writeRoute(selected);
  });
});
for (const button of document.querySelectorAll<HTMLButtonElement>("[data-query]")) {
  button.addEventListener("click", () => { input.value = button.dataset.query ?? ""; input.focus(); search(); });
}
window.addEventListener("popstate", () => void restoreRoute(true));
void load();
