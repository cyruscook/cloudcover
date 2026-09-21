import {
  type ApiCatalog,
  type ApiMethod,
  type TerraformIndex,
  type TerraformRow,
  type TerraformSnapshot,
  CatalogError,
  dataUrl,
  parseApiCatalog,
  parseTerraformIndex,
  parseTerraformRows,
  parseTerraformSnapshot,
} from "../lib/catalog";

type Mode = "api" | "iam" | "terraform";

type SearchOption<T> = {
  display: string;
  value: T;
};

type ComboboxSetup<T> = {
  input: HTMLInputElement;
  listbox: HTMLElement;
  options: (query: string) => SearchOption<T>[];
  onSelect: (option: SearchOption<T>) => void;
};

class Combobox<T> {
  private readonly input: HTMLInputElement;
  private readonly listbox: HTMLElement;
  private readonly getOptions: (query: string) => SearchOption<T>[];
  private readonly onSelect: (option: SearchOption<T>) => void;
  private matches: SearchOption<T>[] = [];
  private activeIndex = -1;

  constructor(setup: ComboboxSetup<T>) {
    this.input = setup.input;
    this.listbox = setup.listbox;
    this.getOptions = setup.options;
    this.onSelect = setup.onSelect;
    this.input.addEventListener("input", () => {
      this.activeIndex = -1;
      this.render();
    });
    this.input.addEventListener("keydown", (event) => this.handleKeydown(event));
    this.input.addEventListener("blur", () => {
      window.setTimeout(() => this.dismiss(), 120);
    });
  }

  clear(): void {
    this.input.value = "";
    this.activeIndex = -1;
    this.dismiss();
  }

  setDisabled(disabled: boolean): void {
    this.input.disabled = disabled;
    if (disabled) {
      this.dismiss();
    }
  }

  private handleKeydown(event: KeyboardEvent): void {
    if (this.input.disabled) {
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (this.matches.length === 0) {
        return;
      }
      const direction = event.key === "ArrowDown" ? 1 : -1;
      this.activeIndex =
        (this.activeIndex + direction + this.matches.length) % this.matches.length;
      this.renderMatches();
      return;
    }
    if (event.key === "Enter") {
      if (this.activeIndex >= 0 && this.matches[this.activeIndex]) {
        event.preventDefault();
        this.select(this.matches[this.activeIndex]);
      }
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      this.dismiss();
    }
  }

  private render(): void {
    const query = this.input.value.trim();
    if (query.length === 0) {
      this.dismiss();
      return;
    }
    this.matches = this.getOptions(query).slice(0, 50);
    this.renderMatches();
  }

  private renderMatches(): void {
    this.listbox.replaceChildren();
    this.listbox.hidden = false;
    this.input.setAttribute("aria-expanded", "true");
    if (this.matches.length === 0) {
      const empty = document.createElement("li");
      empty.className = "suggestion-empty";
      empty.setAttribute("role", "option");
      empty.setAttribute("aria-disabled", "true");
      empty.textContent = "No matches.";
      this.listbox.append(empty);
      this.input.setAttribute("aria-activedescendant", "");
      return;
    }

    this.matches.forEach((match, index) => {
      const option = document.createElement("li");
      option.id = `${this.listbox.id}-option-${index}`;
      option.className = "suggestion";
      option.setAttribute("role", "option");
      option.setAttribute("aria-selected", String(index === this.activeIndex));
      option.textContent = match.display;
      option.addEventListener("mouseenter", () => {
        this.activeIndex = index;
        this.updateActiveOption();
      });
      option.addEventListener("mousedown", (event) => event.preventDefault());
      option.addEventListener("click", () => this.select(match));
      this.listbox.append(option);
    });
    this.input.setAttribute(
      "aria-activedescendant",
      this.activeIndex >= 0
        ? `${this.listbox.id}-option-${this.activeIndex}`
        : "",
    );
  }

  private updateActiveOption(): void {
    const options = this.listbox.querySelectorAll<HTMLElement>(".suggestion");
    options.forEach((option, index) => {
      option.setAttribute("aria-selected", String(index === this.activeIndex));
    });
    this.input.setAttribute(
      "aria-activedescendant",
      this.activeIndex >= 0
        ? `${this.listbox.id}-option-${this.activeIndex}`
        : "",
    );
  }

  private select(option: SearchOption<T>): void {
    this.input.value = option.display;
    this.dismiss();
    this.onSelect(option);
  }

  private dismiss(): void {
    this.matches = [];
    this.activeIndex = -1;
    this.listbox.replaceChildren();
    this.listbox.hidden = true;
    this.input.setAttribute("aria-expanded", "false");
    this.input.setAttribute("aria-activedescendant", "");
  }
}

const root = document.getElementById("explorer");
if (!root) {
  throw new CatalogError("Explorer root is missing");
}

const baseUrl = root.dataset.baseUrl ?? "./";
const apiInput = element<HTMLInputElement>("search-api");
const iamInput = element<HTMLInputElement>("search-iam");
const terraformInput = element<HTMLInputElement>("search-terraform");
const versionSelect = element<HTMLSelectElement>("terraform-version");
const loadStatus = element<HTMLElement>("load-status");
const apiResult = element<HTMLElement>("result-api");
const iamResult = element<HTMLElement>("result-iam");
const terraformResult = element<HTMLElement>("result-terraform");
const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
const panels = Array.from(document.querySelectorAll<HTMLElement>('[role="tabpanel"]'));

let apiCatalog: ApiCatalog | undefined;
let terraformIndex: TerraformIndex | undefined;
let terraformRows: TerraformRow[] | undefined;
let activeVersion = "";
let terraformLoadToken = 0;
const terraformSnapshots = new Map<string, TerraformSnapshot>();
let apiCombobox: Combobox<ApiMethod> | undefined;
let iamCombobox: Combobox<string> | undefined;
let terraformCombobox: Combobox<string> | undefined;

function element<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) {
    throw new CatalogError(`Missing explorer element ${id}`);
  }
  return value as T;
}

function setInteractive(interactive: boolean): void {
  for (const tab of tabs) {
    tab.disabled = !interactive;
  }
  apiCombobox?.setDisabled(!interactive);
  iamCombobox?.setDisabled(!interactive);
  versionSelect.disabled = !interactive;
  terraformCombobox?.setDisabled(!interactive || terraformRows === undefined);
}

function setStatus(message: string, error = false): void {
  loadStatus.classList.toggle("error", error);
  loadStatus.replaceChildren(document.createTextNode(message));
}

function showRetry(target: HTMLElement, message: string, retry: () => void): void {
  target.replaceChildren();
  const text = document.createElement("p");
  text.className = "error-message";
  text.append(document.createTextNode(message));
  const button = document.createElement("button");
  button.className = "retry-button";
  button.type = "button";
  button.textContent = "Retry";
  button.addEventListener("click", retry);
  text.append(button);
  target.append(text);
}

async function fetchDocument(path: string): Promise<unknown> {
  const response = await fetch(dataUrl(baseUrl, path));
  if (!response.ok) {
    throw new CatalogError(`Could not load ${path} (${response.status})`);
  }
  return response.json();
}

function filteredOptions<T>(items: SearchOption<T>[], query: string): SearchOption<T>[] {
  const normalized = query.toLocaleLowerCase();
  return items
    .filter((item) => item.display.toLocaleLowerCase().includes(normalized))
    .sort((left, right) => {
      const leftValue = left.display.toLocaleLowerCase();
      const rightValue = right.display.toLocaleLowerCase();
      const leftPrefix = leftValue.startsWith(normalized);
      const rightPrefix = rightValue.startsWith(normalized);
      if (leftPrefix !== rightPrefix) {
        return leftPrefix ? -1 : 1;
      }
      return left.display < right.display ? -1 : left.display > right.display ? 1 : 0;
    });
}

function renderList(values: string[]): HTMLUListElement {
  const list = document.createElement("ul");
  list.className = "result-list";
  for (const value of values) {
    const item = document.createElement("li");
    item.textContent = value;
    list.append(item);
  }
  return list;
}

function renderHeading(target: HTMLElement, heading: string): void {
  target.replaceChildren();
  const title = document.createElement("h3");
  title.textContent = heading;
  target.append(title);
}

function renderApiResult(method: ApiMethod): void {
  if (!apiCatalog) {
    return;
  }
  renderHeading(apiResult, method.canonical);
  const permissions = method.permissionIds.map((id) => apiCatalog?.iamPermissions[id] ?? "");
  if (permissions.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "No authorized IAM actions in CloudCover data.";
    apiResult.append(empty);
    setStatus(`${method.canonical} selected; no authorized IAM actions.`);
    return;
  }
  apiResult.append(renderList(permissions));
  setStatus(`${method.canonical} selected; ${permissions.length} IAM actions shown.`);
}

function renderIamResult(permission: string): void {
  if (!apiCatalog) {
    return;
  }
  const permissionId = apiCatalog.iamPermissions.indexOf(permission);
  const methods = apiCatalog.apiMethods.filter((method) =>
    method.permissionIds.includes(permissionId),
  );
  renderHeading(iamResult, permission);
  if (methods.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "No API operations use this IAM action in CloudCover data.";
    iamResult.append(empty);
    setStatus(`${permission} selected; no API operations found.`);
    return;
  }
  iamResult.append(renderList(methods.map((method) => method.canonical)));
  setStatus(`${permission} selected; ${methods.length} API operations shown.`);
}

function resourcesForSnapshot(snapshot: TerraformSnapshot): SearchOption<string>[] {
  if (!terraformRows) {
    return [];
  }
  const resources = new Set<string>();
  for (const rowId of snapshot.rowIds) {
    resources.add(terraformRows[rowId].resource);
  }
  return [...resources].sort().map((resource) => ({ display: resource, value: resource }));
}

function renderTerraformResult(resource: string): void {
  if (!apiCatalog || !terraformRows) {
    return;
  }
  const snapshot = terraformSnapshots.get(activeVersion);
  if (!snapshot) {
    return;
  }
  const rows = snapshot.rowIds
    .map((rowId) => terraformRows?.[rowId])
    .filter((row): row is TerraformRow => row !== undefined && row.resource === resource);
  renderHeading(terraformResult, `${resource} · ${activeVersion}`);
  const reachedPermissions = new Set<number>();
  for (const lifecycle of ["create", "read", "update", "delete"]) {
    const lifecycleRows = rows.filter((row) => row.lifecycle === lifecycle);
    const apiIds = new Set<number>();
    for (const row of lifecycleRows) {
      for (const apiId of row.apiMethodIds) {
        apiIds.add(apiId);
        for (const permissionId of apiCatalog.apiMethods[apiId].permissionIds) {
          reachedPermissions.add(permissionId);
        }
      }
    }
    const section = document.createElement("section");
    const heading = document.createElement("h4");
    heading.textContent = lifecycle;
    section.append(heading);
    const methods = [...apiIds]
      .sort((left, right) => left - right)
      .map((apiId) => apiCatalog?.apiMethods[apiId].canonical ?? "");
    if (methods.length === 0) {
      const empty = document.createElement("p");
      empty.className = "empty-mapping";
      empty.textContent = "No known API operation for this lifecycle action.";
      section.append(empty);
    } else {
      section.append(renderList(methods));
    }
    terraformResult.append(section);
  }
  const permissionHeading = document.createElement("h4");
  permissionHeading.textContent = "IAM action union";
  terraformResult.append(permissionHeading);
  const permissions = [...reachedPermissions]
    .sort((left, right) => left - right)
    .map((permissionId) => apiCatalog?.iamPermissions[permissionId] ?? "");
  if (permissions.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-mapping";
    empty.textContent = "No IAM actions reached by the known API operations.";
    terraformResult.append(empty);
  } else {
    terraformResult.append(renderList(permissions));
  }
  setStatus(`${resource} selected for Terraform ${activeVersion}.`);
}

function clearTerraformSelection(): void {
  terraformCombobox?.clear();
  terraformResult.replaceChildren();
  const empty = document.createElement("p");
  empty.className = "empty-state";
  empty.textContent = "Select a Terraform resource to see lifecycle API operations.";
  terraformResult.append(empty);
}

async function loadTerraformVersion(version: string): Promise<void> {
  if (!terraformIndex || !apiCatalog) {
    return;
  }
  const token = ++terraformLoadToken;
  activeVersion = version;
  clearTerraformSelection();
  terraformCombobox?.setDisabled(true);
  versionSelect.disabled = true;
  setStatus(`Loading Terraform ${version} data…`);
  try {
    let snapshot: TerraformSnapshot;
    if (terraformSnapshots.has(version) && terraformRows) {
      snapshot = terraformSnapshots.get(version) as TerraformSnapshot;
    } else {
      const rowsPromise = terraformRows
        ? Promise.resolve(terraformRows)
        : fetchDocument("data/terraform/rows.json").then((value) =>
            parseTerraformRows(value, apiCatalog?.apiMethods.length ?? 0),
          );
      const snapshotPromise = fetchDocument(
        `data/terraform/versions/${version}.json`,
      );
      const [loadedRows, loadedSnapshotDocument] = await Promise.all([
        rowsPromise,
        snapshotPromise,
      ]);
      if (token !== terraformLoadToken) {
        return;
      }
      terraformRows = loadedRows;
      snapshot = parseTerraformSnapshot(
        loadedSnapshotDocument,
        version,
        terraformRows.length,
      );
      terraformSnapshots.set(version, snapshot);
    }
    if (token !== terraformLoadToken) {
      return;
    }
    terraformRows = terraformRows ?? [];
    terraformSnapshots.set(version, snapshot);
    terraformCombobox?.setDisabled(false);
    versionSelect.disabled = false;
    setStatus(`Terraform ${version} data loaded.`);
  } catch (error) {
    if (token !== terraformLoadToken) {
      return;
    }
    terraformCombobox?.setDisabled(true);
    versionSelect.disabled = false;
    const message = error instanceof Error ? error.message : "Unknown data error";
    showRetry(terraformResult, `Could not load Terraform ${version}: ${message}`, () => {
      void loadTerraformVersion(version);
    });
    setStatus(`Terraform ${version} data failed to load.`, true);
  }
}

function activateMode(mode: Mode): void {
  for (const tab of tabs) {
    const selected = tab.id === `tab-${mode}`;
    tab.setAttribute("aria-selected", String(selected));
    tab.tabIndex = selected ? 0 : -1;
  }
  for (const panel of panels) {
    panel.hidden = panel.id !== `panel-${mode}`;
  }
  if (mode === "terraform" && terraformIndex) {
    void loadTerraformVersion(versionSelect.value || terraformIndex.latest);
  }
}

function setupExplorer(): void {
  if (!apiCatalog || !terraformIndex) {
    return;
  }
  const apiOptions = apiCatalog.apiMethods.map((method) => ({
    display: method.canonical,
    value: method,
  }));
  const iamOptions = apiCatalog.iamPermissions.map((permission) => ({
    display: permission,
    value: permission,
  }));
  apiCombobox = new Combobox({
    input: apiInput,
    listbox: element("suggestions-api"),
    options: (query) => filteredOptions(apiOptions, query),
    onSelect: (option) => renderApiResult(option.value),
  });
  iamCombobox = new Combobox({
    input: iamInput,
    listbox: element("suggestions-iam"),
    options: (query) => filteredOptions(iamOptions, query),
    onSelect: (option) => renderIamResult(option.value),
  });
  terraformCombobox = new Combobox({
    input: terraformInput,
    listbox: element("suggestions-terraform"),
    options: (query) => {
      const snapshot = terraformSnapshots.get(activeVersion);
      return snapshot ? filteredOptions(resourcesForSnapshot(snapshot), query) : [];
    },
    onSelect: (option) => renderTerraformResult(option.value),
  });
  for (const version of terraformIndex.versions) {
    const option = document.createElement("option");
    option.value = version;
    option.textContent = version;
    versionSelect.append(option);
  }
  versionSelect.value = terraformIndex.latest;
  versionSelect.addEventListener("change", () => {
    if (versionSelect.value) {
      void loadTerraformVersion(versionSelect.value);
    }
  });
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      activateMode(tab.id.replace("tab-", "") as Mode);
    });
    tab.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") {
        return;
      }
      event.preventDefault();
      const direction = event.key === "ArrowRight" ? 1 : -1;
      const currentIndex = tabs.indexOf(tab);
      const nextTab = tabs[(currentIndex + direction + tabs.length) % tabs.length];
      nextTab.focus();
      activateMode(nextTab.id.replace("tab-", "") as Mode);
    });
  }
}

async function loadSharedCatalogs(): Promise<void> {
  setInteractive(false);
  setStatus("Loading catalog…");
  try {
    const [apiDocument, terraformDocument] = await Promise.all([
      fetchDocument("data/api.json"),
      fetchDocument("data/terraform/index.json"),
    ]);
    apiCatalog = parseApiCatalog(apiDocument);
    terraformIndex = parseTerraformIndex(terraformDocument);
    setupExplorer();
    setInteractive(true);
    setStatus("Catalog loaded. Choose a data path.");
  } catch (error) {
    const message = error instanceof Error ? error.message : "Unknown data error";
    setInteractive(false);
    showRetry(loadStatus, `Could not load CloudCover data: ${message}`, () => {
      void loadSharedCatalogs();
    });
    loadStatus.classList.add("error");
  }
}

void loadSharedCatalogs();
