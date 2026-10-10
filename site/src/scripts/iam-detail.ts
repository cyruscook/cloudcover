import {
  type IamPermission,
  type IamResource,
  filterContextKeys,
} from "../lib/iam";

function node<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text = "",
  className = "",
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  element.className = className;
  return element;
}
function copyButton(value: string, label: string): HTMLButtonElement {
  const button = node("button", label, "reference-copy");
  button.type = "button";
  button.setAttribute("aria-label", label);
  button.replaceChildren();
  const text = node("span", label);
  button.append(text);
  const feedback = node("span", "", "sr-only");
  feedback.setAttribute("role", "status");
  button.append(feedback);
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(value);
      text.textContent = "Copied";
      feedback.textContent = `${label} copied to clipboard.`;
    } catch {
      text.textContent = "Copy failed";
      feedback.textContent =
        "Could not copy. Select and copy the text manually.";
    }
    setTimeout(() => {
      text.textContent = label;
    }, 2000);
  });
  return button;
}
function code(value: string): HTMLElement {
  const content = node("code", "", "reference-code");
  for (const part of value.split(/(\$\{[^}]+\})/)) {
    if (part.startsWith("${"))
      content.append(node("span", part, "arn-variable"));
    else content.append(part);
  }
  return content;
}
function section(
  id: string,
  field: string,
  title: string,
  count: number,
): HTMLElement {
  const panel = node("section", "", "reference-panel");
  panel.id = id;
  panel.setAttribute("aria-labelledby", `${id}-title`);
  const heading = node("div", "", "reference-panel-heading");
  const text = node("div");
  const titleRow = node("div", "", "reference-title-row");
  const h2 = node("h2", title);
  h2.id = `${id}-title`;
  titleRow.append(h2, node("span", String(count), "reference-count"));
  text.append(titleRow);
  const policyField = node("code", field, "reference-field");
  policyField.setAttribute("aria-label", `IAM policy element: ${field}`);
  heading.append(text, policyField);
  panel.append(heading);
  return panel;
}
function resourceCard(resource: IamResource): HTMLElement {
  const card = node("article", "", "arn-card");
  const heading = node("div", "", "arn-card-heading");
  heading.append(
    node("h3", resource.name),
    node("span", "ARN pattern", "arn-caption"),
  );
  card.append(heading);
  if (!resource.arnFormats.length)
    card.append(node("p", "No ARN pattern listed in SAR.", "reference-empty"));
  for (const arn of resource.arnFormats) {
    const row = node("div", "", "arn-pattern");
    const button = copyButton(arn, "Copy ARN");
    button.setAttribute("aria-label", `Copy ARN pattern for ${resource.name}`);
    row.append(code(arn), button);
    card.append(row);
  }
  return card;
}
export function iamDetail(permission: IamPermission): HTMLElement {
  const root = node("div", "", "iam-reference");
  const overview = node("div", "", "reference-overview");
  const scope = node("div", "", "reference-scope");
  scope.append(
    node("span", "Resource scope", "reference-label"),
    node(
      "strong",
      permission.resources.length
        ? "Resource ARNs supported"
        : "Wildcard resource required",
    ),
  );
  const metrics = node("nav", "", "reference-metrics");
  metrics.setAttribute("aria-label", "Permission reference sections");
  for (const [id, count, label] of [
    ["permission-resources", permission.resources.length, "resource types"],
    ["permission-context", permission.contextKeys.length, "context keys"],
  ] as const) {
    const link = node("a");
    link.href = `#${id}`;
    link.append(node("strong", String(count)), node("span", label));
    metrics.append(link);
  }
  overview.append(
    scope,
    metrics,
    copyButton(permission.permission, "Copy permission"),
  );
  root.append(overview);

  const resources = section(
    "permission-resources",
    "Resource",
    "Resource ARNs",
    permission.resources.length,
  );
  if (!permission.resources.length) {
    const wildcard = node("div", "", "wildcard-resource");
    wildcard.append(
      node("code", '"Resource": "*"'),
      copyButton("*", "Copy resource"),
    );
    resources.append(wildcard);
  } else {
    const list = node("div", "", "arn-grid");
    list.append(...permission.resources.slice(0, 4).map(resourceCard));
    resources.append(list);
    if (permission.resources.length > 4) {
      const more = node("details", "", "more-resources");
      more.append(
        node(
          "summary",
          `Show ${permission.resources.length - 4} more resource types`,
        ),
      );
      const remainder = node("div", "", "arn-grid");
      remainder.append(...permission.resources.slice(4).map(resourceCard));
      more.append(remainder);
      resources.append(more);
    }
    const note = node("details", "", "reference-note reference-help");
    note.append(node("summary", "Resource requirements"));
    const explanation = node(
      "p",
      "Required resource combinations are not specified in the SAR feed. ",
    );
    const reference = node("a", "AWS service reference ↗");
    reference.href =
      "https://docs.aws.amazon.com/service-authorization/latest/reference/reference_policies_actions-resources-contextkeys.html";
    reference.target = "_blank";
    reference.rel = "noopener noreferrer";
    explanation.append(reference);
    note.append(explanation);
    resources.append(note);
  }
  root.append(resources);

  const context = section(
    "permission-context",
    "Condition",
    "Context keys",
    permission.contextKeys.length,
  );
  if (!permission.contextKeys.length) {
    context.append(
      node(
        "p",
        "No context keys listed in SAR.",
        "reference-empty context-empty",
      ),
    );
  } else {
    const toolbar = node("div", "", "context-toolbar");
    const search = node("label", "", "context-search");
    search.append(node("span", "Filter context keys", "sr-only"));
    const input = node("input");
    input.type = "search";
    input.placeholder = "Filter by key, value type, or resource…";
    search.append(input);
    const selectLabel = node("label", "", "context-select");
    selectLabel.append(node("span", "Resource", "sr-only"));
    const select = node("select");
    const all = node("option", "All resource types");
    all.value = "";
    select.append(all);
    for (const resource of permission.resources) {
      const option = node("option", resource.name);
      option.value = resource.name;
      select.append(option);
    }
    selectLabel.append(select);
    const count = node("p", "", "context-result-count");
    count.setAttribute("role", "status");
    toolbar.append(search);
    if (permission.resources.length) toolbar.append(selectLabel);
    toolbar.append(count);
    context.append(toolbar);
    const scroll = node("div", "", "context-scroll");
    scroll.tabIndex = 0;
    scroll.setAttribute("role", "region");
    scroll.setAttribute("aria-label", "Context key reference");
    const table = node("table", "", "context-table");
    const caption = node(
      "caption",
      "Context keys, value types, and their SAR applicability",
      "sr-only",
    );
    const head = node("thead");
    const headings = node("tr");
    for (const title of ["Context key", "Value type", "Listed for"]) {
      const th = node("th", title);
      th.scope = "col";
      headings.append(th);
    }
    head.append(headings);
    const body = node("tbody");
    table.append(caption, head, body);
    scroll.append(table);
    const empty = node("div", "", "context-no-results");
    empty.append(node("h3", "No context keys match"));
    const reset = node("button", "Clear filters", "reference-copy");
    reset.type = "button";
    empty.append(reset);
    context.append(scroll, empty);
    function render(): void {
      const keys = filterContextKeys(
        permission.contextKeys,
        input.value,
        select.value,
      );
      count.textContent = `${keys.length} of ${permission.contextKeys.length} keys`;
      body.replaceChildren();
      for (const key of keys) {
        const row = node("tr");
        const name = node("th");
        name.scope = "row";
        name.append(node("code", key.name));
        const types = node("td", "", "context-types");
        for (const type of key.types.length ? key.types : ["Not specified"])
          types.append(
            node(
              "span",
              type.replace(/^ArrayOf(.+)$/, "$1 list"),
              "value-type",
            ),
          );
        const scopes = node("td", "", "context-scopes");
        const scopeLabels: string[] = [];
        if (key.permission) scopeLabels.push("Permission");
        for (const resource of key.permissionResources) {
          if (!select.value || select.value === resource)
            scopeLabels.push(`Permission resource · ${resource}`);
        }
        for (const resource of key.resourceTypes) {
          if (!select.value || select.value === resource)
            scopeLabels.push(`Resource type · ${resource}`);
        }
        for (const label of scopeLabels.slice(0, 3))
          scopes.append(
            node(
              "span",
              label,
              label === "Permission"
                ? "key-scope permission-key-scope"
                : "key-scope",
            ),
          );
        if (scopeLabels.length > 3) {
          const more = node("details", "", "more-key-scopes");
          more.append(node("summary", `+ ${scopeLabels.length - 3} more`));
          for (const label of scopeLabels.slice(3))
            more.append(node("span", label, "key-scope"));
          scopes.append(more);
        }
        row.append(name, types, scopes);
        body.append(row);
      }
      scroll.hidden = !keys.length;
      empty.hidden = !!keys.length;
    }
    input.addEventListener("input", render);
    select.addEventListener("change", render);
    reset.addEventListener("click", () => {
      input.value = "";
      select.value = "";
      render();
      input.focus();
    });
    render();
  }
  const note = node("div", "", "context-guidance");
  if (permission.contextKeys.length) {
    const help = node("details", "", "reference-help");
    help.append(
      node("summary", "Key applicability"),
      node(
        "p",
        "Permission and permission resource keys come from the permission’s SAR entry. Resource type keys come from the resource definition; availability varies by request.",
      ),
    );
    note.append(help);
  }
  const global = node("a", "AWS global context keys ↗");
  global.href =
    "https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_condition-keys.html";
  global.target = "_blank";
  global.rel = "noopener noreferrer";
  note.append(global);
  context.append(note);
  root.append(context);
  const source = node("p", "", "reference-source");
  const link = node("a", "AWS Service Authorization Reference (JSON) ↗");
  link.href = `https://servicereference.us-east-1.amazonaws.com/v1/${encodeURIComponent(permission.permission.split(":")[0])}/${encodeURIComponent(permission.permission.split(":")[0])}.json`;
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  source.append("Source: ", link, ".");
  root.append(source);
  return root;
}
