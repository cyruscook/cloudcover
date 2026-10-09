import type { ApiCatalog } from "../lib/catalog";
import {
  type Entry, type Lifecycle, type Mapping, LIFECYCLES,
  filterMappings, mappingPermissionIds,
} from "../lib/mappings";

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = "", className = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  result.className = className;
  return result;
}

function icon(path: string): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  const shape = document.createElementNS(svg.namespaceURI, "path");
  shape.setAttribute("d", path);
  svg.append(shape);
  return svg;
}

function count(amount: number, singular: string): string {
  return `${amount.toLocaleString()} ${singular}${amount === 1 ? "" : "s"}`;
}

function stepLabel(step: Lifecycle): string { return step[0].toUpperCase() + step.slice(1); }

export function mappingTable(
  catalog: ApiCatalog,
  mappings: Mapping[],
  terraform: boolean,
  link: (item: Entry) => HTMLElement,
): HTMLElement {
  const section = node("section", "", "mapping-panel");
  section.setAttribute("aria-labelledby", "mapping-title");
  const header = node("div", "", "mapping-header");
  const heading = node("div");
  const title = node("h2", "API & IAM mappings");
  title.id = "mapping-title";
  const total = node("p", `${count(mappings.length, "API action")} · ${count(mappingPermissionIds(mappings).length, "IAM permission")}`, "mapping-summary");
  heading.append(title, total);
  const copy = node("button", "", "copy-permissions");
  copy.type = "button";
  const copyLabel = node("span", "Copy permissions");
  copy.append(icon("M9 9h11v11H9z M15 5V3H3v12h2"), copyLabel);
  copy.title = "Copy the unique IAM permissions in the filtered table, one per line";
  header.append(heading, copy);
  section.append(header);

  let lifecycle: Lifecycle | undefined;
  const filters = node("div", "", "lifecycle-filters");
  const buttons = new Map<Lifecycle | undefined, HTMLButtonElement>();
  if (terraform) {
    filters.setAttribute("role", "group");
    filters.setAttribute("aria-label", "Filter by resource lifecycle");
    for (const step of [undefined, ...LIFECYCLES]) {
      const label = step ? stepLabel(step) : "All steps";
      const amount = step ? mappings.filter(mapping => mapping.lifecycles.includes(step)).length : mappings.length;
      const button = node("button", "", "lifecycle-filter");
      button.type = "button";
      button.setAttribute("aria-pressed", String(step === lifecycle));
      button.setAttribute("aria-controls", "mapping-table");
      button.setAttribute("aria-label", `${label}, ${count(amount, "API action")}`);
      button.append(node("span", label), node("span", String(amount), "filter-count"));
      button.addEventListener("click", () => {
        lifecycle = step;
        for (const [key, control] of buttons) control.setAttribute("aria-pressed", String(key === step));
        render();
      });
      buttons.set(step, button);
      filters.append(button);
    }
    section.append(filters);
  }

  const toolbar = node("div", "", "mapping-toolbar");
  const search = node("div", "", "mapping-search");
  const label = node("label", "Filter API actions or IAM permissions", "sr-only");
  label.htmlFor = "mapping-filter";
  const input = node("input");
  input.id = "mapping-filter";
  input.type = "search";
  input.placeholder = "Filter actions or permissions…";
  input.autocomplete = "off";
  input.setAttribute("aria-controls", "mapping-table");
  const clear = node("button", "", "clear-mapping-filter");
  clear.type = "button";
  clear.setAttribute("aria-label", "Clear mapping filter");
  clear.append(icon("m6 6 12 12 M6 18 18 6"));
  clear.hidden = true;
  clear.addEventListener("click", () => { input.value = ""; render(); input.focus(); });
  input.addEventListener("input", render);
  search.append(label, icon("M21 21l-5-5 M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0"), input, clear);
  const results = node("p", "", "mapping-results");
  results.setAttribute("role", "status");
  toolbar.append(search, results);
  section.append(toolbar);

  const scroll = node("div", "", "mapping-scroll");
  scroll.setAttribute("role", "region");
  scroll.setAttribute("aria-label", "API and IAM mapping table");
  scroll.tabIndex = 0;
  const table = node("table", "", `mapping-table${terraform ? " resource-table" : ""}`);
  table.id = "mapping-table";
  table.append(node("caption", `Each API action is grouped with its indexed IAM permissions.${terraform ? " Lifecycle steps indicate where Terraform uses the action." : ""}`, "sr-only"));
  const columns = node("colgroup");
  columns.append(node("col", "", "action-column"), node("col", "", "permission-column"));
  if (terraform) columns.append(node("col", "", "lifecycle-column"));
  table.append(columns);
  const tableHeader = node("thead");
  const headerRow = node("tr");
  for (const [name, kind] of [["API action", "api"], ["IAM permission", "iam"], ...(terraform ? [["Lifecycle", "terraform"]] : [])]) {
    const cell = node("th");
    cell.scope = "col";
    if (kind === "terraform") cell.classList.add("lifecycle-heading");
    const marker = node("span", "", "column-marker");
    marker.dataset.kind = kind;
    cell.append(marker, name);
    headerRow.append(cell);
  }
  tableHeader.append(headerRow);
  table.append(tableHeader);
  scroll.append(table);
  const empty = node("div", "", "mapping-empty");
  empty.hidden = true;
  const emptyTitle = node("h3");
  const emptyDescription = node("p");
  const reset = node("button", "Reset filters", "reset-mapping-filter");
  reset.type = "button";
  reset.addEventListener("click", () => {
    lifecycle = undefined;
    input.value = "";
    for (const [key, control] of buttons) control.setAttribute("aria-pressed", String(key === undefined));
    render();
    input.focus();
  });
  empty.append(emptyTitle, emptyDescription, reset);
  section.append(scroll, empty);

  const footer = node("div", "", "mapping-footer");
  footer.append(icon("M12 11v6 M12 7h.01 M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0"), node("p", "CloudCover includes every IAM permission that could be required. Configuration, resource names, and request parameters determine which ones you actually need."));
  section.append(footer);
  const copyFeedback = node("p", "", "sr-only");
  copyFeedback.setAttribute("role", "status");
  section.append(copyFeedback);
  let visible: Mapping[] = [];
  let copyAttempt = 0;
  copy.addEventListener("click", async () => {
    const attempt = ++copyAttempt;
    const permissions = mappingPermissionIds(visible).map(id => catalog.iamPermissions[id]);
    try {
      await navigator.clipboard.writeText(permissions.join("\n"));
      if (attempt !== copyAttempt) return;
      copyLabel.textContent = "Copied";
      copyFeedback.textContent = `${count(permissions.length, "IAM permission")} copied to clipboard.`;
    } catch {
      if (attempt !== copyAttempt) return;
      copyLabel.textContent = "Copy unavailable";
      copyFeedback.textContent = "Could not access the clipboard. Select and copy the permissions from the table.";
    }
  });

  function render(): void {
    copyAttempt++;
    copyLabel.textContent = "Copy permissions";
    copyFeedback.textContent = "";
    clear.hidden = input.value.length === 0;
    visible = filterMappings(mappings, catalog, input.value, lifecycle);
    const permissionCount = mappingPermissionIds(visible).length;
    copy.disabled = permissionCount === 0;
    copy.setAttribute("aria-label", `Copy ${count(permissionCount, "IAM permission")} from the filtered mappings`);
    results.textContent = `${count(visible.length, "action")} · ${count(permissionCount, "permission")}`;
    table.querySelectorAll("tbody").forEach(body => body.remove());
    for (const mapping of visible) {
      const method = catalog.apiMethods[mapping.apiMethodId];
      const body = node("tbody");
      const permissionIds: (number | undefined)[] = mapping.permissionIds.length ? mapping.permissionIds : [undefined];
      for (const [position, id] of permissionIds.entries()) {
        const row = node("tr");
        if (position === 0) {
          const action = node("th", "", "action-cell");
          action.scope = "rowgroup";
          action.rowSpan = permissionIds.length;
          const context = node("div", "", "action-context");
          context.append(link({ kind: "api", id: mapping.apiMethodId, label: method.canonical, normalized: method.canonical.toLowerCase() }));
          if (terraform) {
            const steps = node("div", "", "mobile-lifecycle lifecycle-tags");
            for (const step of mapping.lifecycles) steps.append(node("span", stepLabel(step), "lifecycle-tag"));
            context.append(steps);
          }
          action.append(context);
          row.append(action);
        }
        const permission = node("td", "", "permission-cell");
        if (id === undefined) permission.append(node("span", "No known permissions", "unmapped-permission"));
        else {
          const name = catalog.iamPermissions[id];
          permission.append(link({ kind: "iam", id, label: name, normalized: name.toLowerCase() }));
        }
        row.append(permission);
        if (terraform && position === 0) {
          const steps = node("td", "", "lifecycle-cell");
          steps.rowSpan = permissionIds.length;
          const list = node("div", "", "lifecycle-tags");
          for (const step of mapping.lifecycles) {
            const tag = node("span", stepLabel(step), "lifecycle-tag");
            list.append(tag);
          }
          steps.append(list);
          row.append(steps);
        }
        body.append(row);
      }
      table.append(body);
    }
    empty.hidden = visible.length > 0;
    if (!visible.length) {
      const filtered = Boolean(input.value.trim() || lifecycle);
      emptyTitle.textContent = filtered ? "No matching mappings" : "No API actions indexed";
      emptyDescription.textContent = filtered ? "Try another action or permission, or reset the filters." : "This catalog has no known API actions for this selection.";
      reset.hidden = !filtered;
    }
  }
  render();
  return section;
}
