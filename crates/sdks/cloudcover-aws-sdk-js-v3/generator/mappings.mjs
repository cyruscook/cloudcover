import ts from "typescript";

function normalizeFiles(packageVersion, files) {
  const normalized = new Map();
  const hasCompiledFiles = [...files].some(([file]) => file.startsWith("dist-es/") || file.startsWith("dist/es/"));
  for (const [file, text] of files) {
    if (file.endsWith(".d.ts") || (hasCompiledFiles && file.endsWith(".ts"))) continue;

    let outputFile = file;
    let outputText = text;
    if (file.endsWith(".ts")) {
      const relative = file.startsWith("src/") ? file.slice("src/".length) : file;
      outputFile = `dist-es/${relative.slice(0, -".ts".length)}.js`;
      const result = ts.transpileModule(text, {
        fileName: `${packageVersion.package}/${file}`,
        reportDiagnostics: true,
        compilerOptions: {
          module: ts.ModuleKind.ESNext,
          target: ts.ScriptTarget.ES2020,
          newLine: ts.NewLineKind.LineFeed,
        },
      });
      const errors = result.diagnostics?.filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error) ?? [];
      if (errors.length > 0) {
        const detail = errors.map((diagnostic) => ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")).join("; ");
        throw new Error(`${packageVersion.package}@${packageVersion.version}:${file}: TypeScript transpilation failed: ${detail}`);
      }
      outputText = result.outputText;
    }

    if (normalized.has(outputFile)) {
      throw new Error(`${packageVersion.package}@${packageVersion.version}: duplicate normalized source ${outputFile}`);
    }
    normalized.set(outputFile, outputText);
  }
  return normalized;
}

function sourceFiles(files, prefix) {
  const prefixes = prefix.startsWith("dist-es/")
    ? [prefix, prefix.replace("dist-es/", "dist/es/")]
    : [prefix];
  return [...files].filter(([name]) => prefixes.some((candidate) => name.startsWith(candidate)) && name.endsWith(".js"));
}

function matchOne(text, pattern, description) {
  const matches = [...text.matchAll(pattern)];
  if (matches.length !== 1) throw new Error(`${description}: expected one match, got ${matches.length}`);
  return matches[0];
}

function canonicalSigningService(packageName, files) {
  const candidates = sourceFiles(files, "dist-es/").filter(([name]) =>
    /runtimeConfig\.shared\.js$/.test(name) || /auth\/httpAuthSchemeProvider\.js$/.test(name));
  const services = new Set();
  const serviceIds = new Set();
  for (const [name, text] of candidates) {
    const constants = new Map([...text.matchAll(/\bconst\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*["']([^"']+)["']/g)].map((match) => [match[1], match[2]]));
    for (const match of text.matchAll(/\bsigningService\s*:\s*(?:config\.signingService\s*\?\?\s*)?([A-Za-z_$][A-Za-z0-9_$]*|["'][^"']+["'])/g)) {
      const value = match[1];
      const service = value.startsWith("\"") || value.startsWith("'") ? value.slice(1, -1) : constants.get(value);
      if (!service) throw new Error(`${packageName}:${name}: cannot resolve signingService ${value}`);
      services.add(service);
    }
    for (const match of text.matchAll(/\bsigningProperties\s*:\s*\{\s*name\s*:\s*["']([^"']+)["']/g)) services.add(match[1]);
    for (const match of text.matchAll(/\bserviceId\s*:[\s\S]{0,160}?["']([^"']+)["']/g)) serviceIds.add(match[1]);
  }
  if (services.size === 0) {
    for (const service of serviceIds) services.add(service);
  }
  if (services.size !== 1) {
    throw new Error(`${packageName}: expected one signing service in SDK metadata, found ${[...services].join(", ") || "none"}`);
  }
  return [...services][0];
}

function commandMappings(packageName, files, service) {
  const mappings = [];
  const commands = new Map();
  for (const [file, text] of sourceFiles(files, "dist-es/commands/")) {
    const classMatch = text.match(/(?:export\s+class|var)\s+([A-Za-z0-9_]+Command)\b/);
    if (!classMatch) continue;
    const command = classMatch[1];
    const operation = command.slice(0, -"Command".length);
    if (commands.has(command)) throw new Error(`${packageName}: duplicate command ${command}`);
    commands.set(command, operation);
    mappings.push({ package: packageName, receiver: null, method: command, api_methods: [{ service, name: operation }] });
  }
  if (commands.size === 0) throw new Error(`${packageName}: no exported commands found`);
  return { commands, mappings };
}

function clientMappings(packageName, files, service, commands, version) {
  const filesWithAggregatedClient = sourceFiles(files, "dist-es/").filter(([, text]) => /createAggregatedClient\(/.test(text));
  if (filesWithAggregatedClient.length === 0) {
    const legacy = [];
    for (const [file, text] of sourceFiles(files, "dist-es/")) {
      const legacyClass = text.match(/\b([A-Za-z0-9_]+)\.prototype\./);
      const receiver = legacyClass?.[1] ?? text.match(/export\s+class\s+([A-Za-z0-9_]+)\s+extends\s+[A-Za-z0-9_]+Client\b/)?.[1];
      if (!receiver) continue;
      const methodPattern = legacyClass
        ? /\b[A-Za-z0-9_]+\.prototype\.([A-Za-z0-9_]+)\s*=\s*function[\s\S]*?new\s+([A-Za-z0-9_]+Command)\s*\(/g
        : /^\s*([A-Za-z0-9_]+)\([^)]*\)\s*\{[\s\S]*?new\s+([A-Za-z0-9_]+Command)\s*\(/gm;
      for (const match of text.matchAll(methodPattern)) {
        const method = match[1];
        const command = match[2];
        const operation = commands.get(command);
        if (!operation) throw new Error(`${packageName}:${file}: client references unknown ${command}`);
        legacy.push({ package: packageName, receiver, method, api_methods: [{ service, name: operation }] });
      }
    }
    if (legacy.length === 0) throw new Error(`${packageName}@${version}: no aggregated or legacy client methods found`);
    return legacy;
  }
  if (filesWithAggregatedClient.length !== 1) {
    throw new Error(`${packageName}@${version}: expected one aggregated client module, found ${filesWithAggregatedClient.length}`);
  }
  const [file, text] = filesWithAggregatedClient[0];
  const aggregatedMatch = matchOne(
    text,
    /createAggregatedClient\(\s*commands\s*,\s*([A-Za-z0-9_]+)\s*(?:,|\))/g,
    `${packageName}:${file}`,
  );
  const receiver = aggregatedMatch[1];
  if (!new RegExp(`export\\s+class\\s+${receiver}\\b`, "g").test(text)) {
    throw new Error(`${packageName}:${file}: aggregated client class not found`);
  }
  const commandBlock = matchOne(text, /const\s+commands\s*=\s*\{([\s\S]*?)\n\};/g, `${packageName}:${file}`)[1];
  const aggregatedCommands = new Set([...commandBlock.matchAll(/\b([A-Za-z0-9_]+Command)\b/g)].map((match) => match[1]));
  if (aggregatedCommands.size === 0) throw new Error(`${packageName}:${file}: no aggregated command symbols`);
  return [...aggregatedCommands].sort().map((command) => {
    const operation = commands.get(command);
    if (!operation) throw new Error(`${packageName}:${file}: aggregated client references unknown ${command}`);
    return {
      package: packageName,
      receiver,
      method: command.slice(0, -"Command".length).replace(/^./, (letter) => letter.toLowerCase()),
      api_methods: [{ service, name: operation }],
    };
  });
}

function helperMappings(packageName, files, service, commands) {
  const mappings = [];
  for (const [file, text] of sourceFiles(files, "dist-es/pagination/")) {
    for (const match of text.matchAll(/export\s+(?:(?:async\s+)?function\s*\*?|const)\s+(paginate[A-Za-z0-9_]+)/g)) {
      const method = match[1];
      const command = `${method.slice("paginate".length)}Command`;
      const operation = commands.get(command);
      if (operation) {
        mappings.push({ package: packageName, receiver: null, method, api_methods: [{ service, name: operation }] });
      }
    }
  }
  for (const [file, text] of sourceFiles(files, "dist-es/waiters/")) {
    const names = [
      ...text.matchAll(/export\s+(?:(?:async\s+)?function\s*\*?|const)\s+(wait(?:For|Until)[A-Za-z0-9_]+)/g),
      ...text.matchAll(/\bvar\s+(wait(?:For|Until)[A-Za-z0-9_]+)\s*=/g),
    ].map((match) => match[1]);
    const commandMatch = text.match(/new\s+([A-Za-z0-9_]+Command)\s*\(/);
    if (!commandMatch) continue;
    const operation = commands.get(commandMatch[1]);
    if (!operation) continue;
    for (const method of names) mappings.push({ package: packageName, receiver: null, method, api_methods: [{ service, name: operation }] });
  }
  return mappings;
}

export function compareMappings(left, right) {
  return String(left.receiver).localeCompare(String(right.receiver)) ||
    left.method.localeCompare(right.method);
}

export function inspectFiles(packageVersion, files) {
  const normalizedFiles = normalizeFiles(packageVersion, files);
  const service = canonicalSigningService(`${packageVersion.package}@${packageVersion.version}`, normalizedFiles);
  const { commands, mappings } = commandMappings(packageVersion.package, normalizedFiles, service);
  mappings.push(...clientMappings(packageVersion.package, normalizedFiles, service, commands, packageVersion.version));
  mappings.push(...helperMappings(packageVersion.package, normalizedFiles, service, commands));
  for (const mapping of mappings) delete mapping.package;
  mappings.sort(compareMappings);
  for (let index = 1; index < mappings.length; index += 1) {
    if (compareMappings(mappings[index - 1], mappings[index]) === 0) {
      throw new Error(`${packageVersion.package}: duplicate mapping ${mappings[index].receiver ?? ""}.${mappings[index].method}`);
    }
  }
  return { ...packageVersion, mappings };
}
