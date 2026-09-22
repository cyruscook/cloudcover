#!/usr/bin/env node
'use strict';

// This program intentionally uses TypeScript's compiler API only. It creates a Program from
// source and declaration files; it never loads or executes analyzed project or SDK code.
const fs = require('fs');
const path = require('path');
const { createRequire } = require('module');

function fail(message) {
  process.stdout.write(JSON.stringify({ error: message }));
  process.exitCode = 0;
}

function loadTypeScript(root) {
  try {
    const projectRequire = createRequire(path.join(root, 'package.json'));
    return projectRequire('typescript');
  } catch (error) {
    throw new Error(`TypeScript compiler was not found from ${root}. Install typescript in the analyzed project: ${error.message}`);
  }
}

function configFor(ts, root) {
  const configName = ['tsconfig.json', 'jsconfig.json']
    .map((name) => path.join(root, name))
    .find((candidate) => fs.existsSync(candidate));
  if (!configName) {
    return {
      rootNames: discoverSources(root),
      options: {
        allowJs: true,
        checkJs: true,
        module: ts.ModuleKind.NodeNext,
        moduleResolution: ts.ModuleResolutionKind.NodeNext,
        target: ts.ScriptTarget.ESNext,
        skipLibCheck: true,
      },
    };
  }

  const read = ts.readConfigFile(configName, ts.sys.readFile);
  if (read.error) {
    throw new Error(renderDiagnostic(ts, read.error));
  }
  const defaults = path.basename(configName) === 'jsconfig.json' ? { allowJs: true } : undefined;
  const parsed = ts.parseJsonConfigFileContent(
    read.config,
    ts.sys,
    path.dirname(configName),
    defaults,
    configName,
  );
  if (parsed.errors.length > 0) {
    throw new Error(parsed.errors.map((diagnostic) => renderDiagnostic(ts, diagnostic)).join('\n'));
  }
  parsed.options.skipLibCheck = true;
  return { rootNames: parsed.fileNames.filter(isAnalyzableFile), options: parsed.options };
}

function renderDiagnostic(ts, diagnostic) {
  return ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n');
}

function isAnalyzableFile(file) {
  return /\.(?:[cm]?js|jsx|[cm]?ts|tsx)$/i.test(file) && !/\.d\.(?:[cm]?ts)$/i.test(file);
}

function discoverSources(root) {
  const results = [];
  const ignored = new Set(['node_modules', '.git', 'dist', 'build', 'out', 'coverage', '.next']);
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (!ignored.has(entry.name) && !entry.name.startsWith('.')) walk(path.join(directory, entry.name));
      } else if (entry.isFile() && isAnalyzableFile(entry.name)) {
        results.push(path.join(directory, entry.name));
      }
    }
  }
  walk(root);
  return results;
}

function packageInfo(fileName) {
  let current = path.dirname(fileName);
  for (;;) {
    const manifest = path.join(current, 'package.json');
    if (fs.existsSync(manifest)) {
      try {
        const value = JSON.parse(fs.readFileSync(manifest, 'utf8'));
        const parent = path.dirname(current);
        const scopedPackage = path.basename(path.dirname(parent)) === 'node_modules';
        const unscopedPackage = path.basename(parent) === 'node_modules';
        if ((scopedPackage || unscopedPackage)
            && typeof value.name === 'string' && typeof value.version === 'string') {
          return { path: value.name, version: value.version, root: current };
        }
      } catch (_) {
        return undefined;
      }
    }
    const parent = path.dirname(current);
    if (parent === current) return undefined;
    current = parent;
  }
}

function isAwsSdk(info) {
  return info && info.path.startsWith('@aws-sdk/');
}

function sourcePackage(symbol) {
  const seen = new Set();
  function walk(candidate) {
    if (!candidate || seen.has(candidate)) return undefined;
    seen.add(candidate);
    for (const declaration of candidate.declarations || []) {
      const info = packageInfo(declaration.getSourceFile().fileName);
      if (isAwsSdk(info)) return info;
    }
    return undefined;
  }
  return walk(symbol);
}

function unalias(checker, symbol) {
  const seen = new Set();
  while (symbol && (symbol.flags & 2097152) !== 0 && !seen.has(symbol)) { // SymbolFlags.Alias
    seen.add(symbol);
    symbol = checker.getAliasedSymbol(symbol);
  }
  return symbol;
}

function parentClassName(ts, symbol) {
  for (const declaration of symbol.declarations || []) {
    for (let current = declaration; current; current = current.parent) {
      if (ts.isClassDeclaration(current) || ts.isInterfaceDeclaration(current)) {
        return current.name && current.name.text;
      }
    }
  }
  return undefined;
}

function isAwsSpecifier(specifier) {
  return specifier === '@aws-sdk' || specifier.startsWith('@aws-sdk/');
}

function moduleSymbol(ts, checker, program, source, specifier, options) {
  if (!isAwsSpecifier(specifier)) return undefined;
  const resolved = ts.resolveModuleName(specifier, source.fileName, options, ts.sys).resolvedModule;
  if (!resolved) {
    throw new Error(`could not resolve AWS SDK module ${specifier} imported by ${source.fileName}`);
  }
  const info = packageInfo(resolved.resolvedFileName);
  if (!isAwsSdk(info)) {
    throw new Error(`could not identify AWS SDK package ${specifier} resolved from ${source.fileName}`);
  }
  const target = program.getSourceFile(resolved.resolvedFileName);
  const symbol = target && checker.getSymbolAtLocation(target);
  if (!symbol) {
    throw new Error(`could not read exports for AWS SDK module ${specifier} imported by ${source.fileName}`);
  }
  return { info, symbol, specifier };
}

function analyze(root) {
  const ts = loadTypeScript(root);
  const config = configFor(ts, root);
  const program = ts.createProgram({ rootNames: config.rootNames, options: config.options });
  const checker = program.getTypeChecker();
  const inputs = program.getSourceFiles().filter(
    (source) => !source.isDeclarationFile && !program.isSourceFileFromExternalLibrary(source),
  );
  const bindings = new Map();
  const methods = new Map();
  const packages = new Map();
  let analysisError;

  function addBinding(symbol, value) {
    if (symbol) bindings.set(symbol, value);
  }

  function bindingFor(node) {
    return bindings.get(checker.getSymbolAtLocation(node));
  }

  function addPackage(info) {
    const previous = packages.get(info.path);
    if (previous && previous.version !== info.version) {
      analysisError ||= `AWS SDK package ${info.path} resolved to both ${previous.version} and ${info.version}; analysis cannot safely combine operations from nested installations`;
      return false;
    }
    packages.set(info.path, info);
    return true;
  }

  function addMethod(info, receiver, name) {
    if (!addPackage(info)) return;
    const key = `${info.path}\u0000${receiver || ''}\u0000${name}`;
    methods.set(key, { package: info.path, receiver: receiver || undefined, name });
  }

  function symbolAt(node) {
    return unalias(checker, checker.getSymbolAtLocation(node));
  }

  function resolvedSymbol(node) {
    const direct = symbolAt(node);
    const directPackage = sourcePackage(direct);
    if (directPackage) return { symbol: direct, info: directPackage };

    if (ts.isIdentifier(node)) return bindingFor(node);
    if (ts.isPropertyAccessExpression(node)) {
      const directProperty = symbolAt(node.name);
      const directInfo = sourcePackage(directProperty);
      if (directInfo) return { symbol: directProperty, info: directInfo };
      const base = resolvedSymbol(node.expression);
      if (base && base.symbol) {
        const type = checker.getTypeOfSymbolAtLocation(base.symbol, node.expression);
        const member = checker.getPropertyOfType(type, node.name.text);
        const unaliased = unalias(checker, member);
        const info = sourcePackage(unaliased) || base.info;
        if (unaliased && info) return { symbol: unaliased, info };
      }
      if (base && base.info) {
        return { info: base.info, missingExport: node.name.text, specifier: base.specifier };
      }
    }
    return undefined;
  }

  function exportedSymbol(module, name, source) {
    const symbol = checker.getExportsOfModule(module.symbol)
      .find((candidate) => candidate.name === name);
    if (!symbol) {
      throw new Error(`AWS SDK module ${module.specifier} has no runtime export ${name}, imported by ${source.fileName}`);
    }
    return unalias(checker, symbol);
  }

  function hasRuntimeImport(clause) {
    if (!clause || clause.isTypeOnly || clause.name) return !!clause && !clause.isTypeOnly;
    if (clause.namedBindings && ts.isNamespaceImport(clause.namedBindings)) return true;
    return !!clause.namedBindings
      && clause.namedBindings.elements.some((item) => !item.isTypeOnly);
  }

  function collectBindings(source) {
    function visit(node) {
      if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)
          && (!node.importClause || hasRuntimeImport(node.importClause))) {
        const module = moduleSymbol(ts, checker, program, source, node.moduleSpecifier.text, config.options);
        if (module && node.importClause) {
          const clause = node.importClause;
          if (clause.name) {
            addBinding(
              checker.getSymbolAtLocation(clause.name),
              { ...module, symbol: exportedSymbol(module, 'default', source) },
            );
          }
          if (clause.namedBindings) {
            if (ts.isNamespaceImport(clause.namedBindings)) {
              addBinding(checker.getSymbolAtLocation(clause.namedBindings.name), module);
            } else {
              for (const item of clause.namedBindings.elements) {
                if (item.isTypeOnly) continue;
                const imported = item.propertyName || item.name;
                addBinding(checker.getSymbolAtLocation(item.name), {
                  ...module,
                  symbol: exportedSymbol(module, imported.text, source),
                  name: imported.text,
                });
              }
            }
          }
        }
      }
      if (ts.isVariableDeclaration(node) && node.initializer && ts.isCallExpression(node.initializer)
          && ts.isIdentifier(node.initializer.expression) && node.initializer.expression.text === 'require'
          && node.initializer.arguments.length === 1 && ts.isStringLiteral(node.initializer.arguments[0])) {
        const module = moduleSymbol(ts, checker, program, source, node.initializer.arguments[0].text, config.options);
        if (module) {
          if (ts.isIdentifier(node.name)) {
            addBinding(checker.getSymbolAtLocation(node.name), module);
          } else if (ts.isObjectBindingPattern(node.name)) {
            for (const element of node.name.elements) {
              if (!ts.isIdentifier(element.name)) continue;
              const imported = element.propertyName && ts.isIdentifier(element.propertyName)
                ? element.propertyName.text : element.name.text;
              addBinding(checker.getSymbolAtLocation(element.name), {
                ...module,
                symbol: exportedSymbol(module, imported, source),
                name: imported,
              });
            }
          }
        }
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
  }

  function isCommand(result) {
    const name = result && (result.symbol ? result.symbol.name : result.name);
    return !!result && !!result.info && typeof name === 'string' && name.endsWith('Command');
  }

  function commandFrom(argument) {
    if (ts.isNewExpression(argument)) return resolvedSymbol(argument.expression);
    if (ts.isIdentifier(argument)) {
      const symbol = symbolAt(argument);
      for (const declaration of symbol && symbol.declarations || []) {
        if (ts.isVariableDeclaration(declaration) && declaration.initializer && ts.isNewExpression(declaration.initializer)) {
          return resolvedSymbol(declaration.initializer.expression);
        }
      }
    }
    return undefined;
  }

  function looksLikeSdkSend(call) {
    if (!ts.isPropertyAccessExpression(call.expression) || call.expression.name.text !== 'send') return false;
    const type = checker.getTypeAtLocation(call.expression.expression);
    const symbol = unalias(checker, type && type.symbol);
    return isAwsSdk(sourcePackage(symbol));
  }

  function visitUsage(node) {
    if (analysisError) return;
    if (ts.isPropertyAccessExpression(node)) {
      const result = resolvedSymbol(node);
      if (result && result.missingExport) {
        analysisError = `AWS SDK module ${result.specifier || result.info.path} has no runtime export ${result.missingExport}, used in ${node.getSourceFile().fileName}:${ts.getLineAndCharacterOfPosition(node.getSourceFile(), node.getStart()).line + 1}`;
        return;
      }
    }
    if (ts.isNewExpression(node)) {
      const result = resolvedSymbol(node.expression);
      if (isCommand(result)) addMethod(result.info, undefined, result.symbol ? result.symbol.name : result.name);
    } else if (ts.isCallExpression(node)) {
      if (looksLikeSdkSend(node)) {
        const command = node.arguments[0] && commandFrom(node.arguments[0]);
        if (isCommand(command)) {
          addMethod(command.info, undefined, command.symbol ? command.symbol.name : command.name);
        } else {
          analysisError = `could not resolve the AWS SDK command passed to send() in ${node.getSourceFile().fileName}:${ts.getLineAndCharacterOfPosition(node.getSourceFile(), node.getStart()).line + 1}`;
          return;
        }
      }

      const result = resolvedSymbol(node.expression);
      if (result && result.info) {
        const name = result.symbol ? result.symbol.name : result.name;
        const receiver = result.symbol && parentClassName(ts, result.symbol);
        if (receiver && !receiver.endsWith('Client') && name !== 'send') {
          addMethod(result.info, receiver, name);
        } else if (!receiver && /^(?:paginate|waitFor|waitUntil)/.test(name || '')) {
          addMethod(result.info, undefined, name);
        }
      }
    }
    ts.forEachChild(node, visitUsage);
  }

  for (const source of inputs) collectBindings(source);
  for (const source of inputs) visitUsage(source);
  if (analysisError) throw new Error(analysisError);

  const compare = (left, right) => left.package.localeCompare(right.package)
    || String(left.receiver || '').localeCompare(String(right.receiver || ''))
    || left.name.localeCompare(right.name);
  return {
    methods: [...methods.values()].sort(compare),
    modules: [...packages.values()]
      .map((info) => ({ path: info.path, version: info.version }))
      .sort((left, right) => left.path.localeCompare(right.path) || left.version.localeCompare(right.version)),
  };
}

try {
  const root = process.argv[1];
  if (!root) throw new Error('analyzed directory argument is required');
  process.stdout.write(JSON.stringify(analyze(path.resolve(root))));
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}
