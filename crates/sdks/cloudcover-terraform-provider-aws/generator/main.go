package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/static"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
)

const providerModulePath = "github.com/hashicorp/terraform-provider-aws"

type apiMethod struct {
	Service string `json:"service"`
	Name    string `json:"name"`
}

type sdkMappingRow struct {
	Package    string      `json:"package"`
	Receiver   string      `json:"receiver"`
	Method     string      `json:"method"`
	APIMethods []apiMethod `json:"api_methods"`
}

type mappingRow struct {
	Kind       string      `json:"kind"`
	TypeName   string      `json:"type_name"`
	Action     string      `json:"action"`
	APIMethods []apiMethod `json:"api_methods"`
}

type mappingKey struct {
	kind     string
	typeName string
	action   string
}

type sdkMethodKey struct {
	pkg      string
	receiver string
	method   string
}

type entrypointSpec struct {
	kind        string
	typeName    string
	factory     string
	sourceFile  string
	packagePath string
}

type packageIndex struct {
	byPath      map[string]*packages.Package
	byFile      map[string]*packages.Package
	funcDecls   map[*types.Func]*ast.FuncDecl
	ssaFuncs    map[*types.Func][]*ssa.Function
	ssaBySyntax map[ast.Node][]*ssa.Function
	nestedFuncs map[*ssa.Function][]*ssa.Function
	callers     map[*ssa.Function][]*ssa.Function
}

type handlerMethod struct {
	action string
	funcs  []*ssa.Function
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	providerDir := flag.String("provider-dir", "", "path to terraform-provider-aws checkout")
	sdkMapJSON := flag.String("sdk-map-json", "", "path to aws-sdk-go-v2 mapping JSON")
	flag.Parse()
	if *providerDir == "" {
		return errors.New("--provider-dir is required")
	}
	if *sdkMapJSON == "" {
		return errors.New("--sdk-map-json is required")
	}
	if flag.NArg() != 0 {
		return fmt.Errorf("unexpected positional arguments: %v", flag.Args())
	}

	sdkMappings, err := loadSDKMappings(*sdkMapJSON)
	if err != nil {
		return err
	}
	index, err := loadProviderIndex(*providerDir)
	if err != nil {
		return err
	}
	specs, err := discoverEntrypoints(index)
	if err != nil {
		return err
	}
	if len(specs) == 0 {
		return errors.New("discovered no terraform-provider-aws entrypoints")
	}

	rows := make([]mappingRow, 0, len(specs))
	for _, spec := range specs {
		handlers, err := resolveHandlers(index, spec)
		if err != nil {
			return fmt.Errorf("%s %s factory %s in %s: %w", spec.kind, spec.typeName, spec.factory, spec.sourceFile, err)
		}
		for _, handler := range handlers {
			apiMethods := collectAPIMethods(index, handler.funcs, sdkMappings)
			rows = append(rows, mappingRow{
				Kind:       spec.kind,
				TypeName:   spec.typeName,
				Action:     handler.action,
				APIMethods: apiMethods,
			})
		}
	}

	slices.SortFunc(rows, compareMappingRows)
	rows = compactMappingRows(rows)

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	return encoder.Encode(rows)
}

func loadSDKMappings(path string) (map[sdkMethodKey][]apiMethod, error) {
	contents, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var rows []sdkMappingRow
	if err := json.Unmarshal(contents, &rows); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	mappings := make(map[sdkMethodKey][]apiMethod, len(rows))
	for _, row := range rows {
		methods := append([]apiMethod(nil), row.APIMethods...)
		slices.SortFunc(methods, compareAPIMethods)
		methods = slices.Compact(methods)
		key := sdkMethodKey{pkg: row.Package, receiver: row.Receiver, method: row.Method}
		if existing, ok := mappings[key]; ok {
			if !slices.Equal(existing, methods) {
				return nil, fmt.Errorf("aws-sdk-go-v2 rows disagree for %s %s.%s", row.Package, row.Receiver, row.Method)
			}
			continue
		}
		mappings[key] = methods
	}
	return mappings, nil
}

func loadProviderIndex(providerDir string) (*packageIndex, error) {
	initial, err := packages.Load(&packages.Config{
		Mode:  packages.LoadAllSyntax,
		Dir:   providerDir,
		Tests: false,
	}, "./internal/service/...")
	if err != nil {
		return nil, fmt.Errorf("packages.Load failed for %s: %w", providerDir, err)
	}
	if packages.PrintErrors(initial) > 0 {
		return nil, fmt.Errorf("packages.Load reported errors for %s", providerDir)
	}

	prog, ssaPackages := ssautil.AllPackages(initial, ssa.InstantiateGenerics)
	for _, pkg := range ssaPackages {
		pkg.Build()
	}
	prog.Build()
	cg := static.CallGraph(prog)

	index := &packageIndex{
		byPath:      map[string]*packages.Package{},
		byFile:      map[string]*packages.Package{},
		funcDecls:   map[*types.Func]*ast.FuncDecl{},
		ssaFuncs:    map[*types.Func][]*ssa.Function{},
		ssaBySyntax: map[ast.Node][]*ssa.Function{},
		nestedFuncs: map[*ssa.Function][]*ssa.Function{},
		callers:     map[*ssa.Function][]*ssa.Function{},
	}

	packages.Visit(initial, nil, func(pkg *packages.Package) {
		if pkg == nil || pkg.Types == nil {
			return
		}
		index.byPath[pkg.PkgPath] = pkg
		for i, file := range pkg.Syntax {
			if i < len(pkg.GoFiles) {
				index.byFile[filepath.Clean(pkg.GoFiles[i])] = pkg
			}
			for _, decl := range file.Decls {
				funcDecl, ok := decl.(*ast.FuncDecl)
				if !ok || funcDecl.Name == nil {
					continue
				}
				obj, ok := pkg.TypesInfo.Defs[funcDecl.Name].(*types.Func)
				if ok {
					index.funcDecls[obj] = funcDecl
				}
			}
		}
	})

	for fn := range ssautil.AllFunctions(prog) {
		if fn == nil {
			continue
		}
		if syntax := fn.Syntax(); syntax != nil {
			index.ssaBySyntax[syntax] = append(index.ssaBySyntax[syntax], fn)
		}
		if parent := fn.Parent(); parent != nil {
			index.nestedFuncs[parent] = append(index.nestedFuncs[parent], fn)
		}
		obj, ok := fn.Object().(*types.Func)
		if ok {
			index.ssaFuncs[obj] = append(index.ssaFuncs[obj], fn)
		}
	}
	for _, fns := range index.ssaFuncs {
		slices.SortFunc(fns, func(left, right *ssa.Function) int {
			return compareStrings(left.String(), right.String())
		})
	}
	for _, fns := range index.ssaBySyntax {
		slices.SortFunc(fns, func(left, right *ssa.Function) int {
			return compareStrings(left.String(), right.String())
		})
	}
	for _, fns := range index.nestedFuncs {
		slices.SortFunc(fns, func(left, right *ssa.Function) int {
			return compareStrings(left.String(), right.String())
		})
	}

	if err := callgraph.GraphVisitEdges(cg, func(edge *callgraph.Edge) error {
		if edge == nil || edge.Caller == nil || edge.Callee == nil {
			return nil
		}
		caller := edge.Caller.Func
		callee := edge.Callee.Func
		if caller == nil || callee == nil {
			return nil
		}
		index.callers[caller] = append(index.callers[caller], callee)
		return nil
	}); err != nil {
		return nil, err
	}
	for caller, callees := range index.callers {
		slices.SortFunc(callees, func(left, right *ssa.Function) int {
			return compareStrings(left.String(), right.String())
		})
		index.callers[caller] = slices.Compact(callees)
	}

	return index, nil
}

func discoverEntrypoints(index *packageIndex) ([]entrypointSpec, error) {
	paths := make([]string, 0)
	for path := range index.byFile {
		if strings.Contains(path, string(filepath.Separator)+"internal"+string(filepath.Separator)+"service"+string(filepath.Separator)) &&
			strings.HasPrefix(filepath.Base(path), "service_package") &&
			strings.HasSuffix(path, ".go") {
			paths = append(paths, path)
		}
	}
	slices.Sort(paths)

	specs := make([]entrypointSpec, 0)
	for _, path := range paths {
		pkg := index.byFile[path]
		if pkg == nil {
			return nil, fmt.Errorf("no package found for %s", path)
		}
		file, err := fileForPath(pkg, path)
		if err != nil {
			return nil, err
		}
		for _, decl := range file.Decls {
			funcDecl, ok := decl.(*ast.FuncDecl)
			if !ok || !isServicePackageMethod(funcDecl) {
				continue
			}
			kind, ok := serviceMethodKind(funcDecl.Name.Name)
			if !ok {
				continue
			}
			methodSpecs, err := extractSpecsFromServiceMethod(pkg, funcDecl, kind, path)
			if err != nil {
				return nil, err
			}
			specs = append(specs, methodSpecs...)
		}
	}
	return specs, nil
}

func resolveHandlers(index *packageIndex, spec entrypointSpec) ([]handlerMethod, error) {
	pkg := index.byPath[spec.packagePath]
	if pkg == nil || pkg.Types == nil {
		return nil, fmt.Errorf("package %s not loaded", spec.packagePath)
	}
	factoryObj, ok := pkg.Types.Scope().Lookup(spec.factory).(*types.Func)
	if !ok {
		return nil, fmt.Errorf("factory not found")
	}
	decl := index.funcDecls[factoryObj]
	if decl == nil {
		return nil, fmt.Errorf("factory declaration not found")
	}

	switch spec.kind {
	case "resource":
		if isSDKFactory(factoryObj) {
			return resolveSchemaResourceHandlers(index, pkg, decl, map[string]string{
				"Create":               "create",
				"CreateContext":        "create",
				"CreateWithoutTimeout": "create",
				"Read":                 "read",
				"ReadContext":          "read",
				"ReadWithoutTimeout":   "read",
				"Update":               "update",
				"UpdateContext":        "update",
				"UpdateWithoutTimeout": "update",
				"Delete":               "delete",
				"DeleteContext":        "delete",
				"DeleteWithoutTimeout": "delete",
			})
		}
		return resolveConcreteTypeHandlers(index, pkg, decl, []namedMethodAction{{"Create", "create"}, {"Read", "read"}, {"Update", "update"}, {"Delete", "delete"}})
	case "data_source":
		if isSDKFactory(factoryObj) {
			return resolveSchemaResourceHandlers(index, pkg, decl, map[string]string{
				"Read":               "read",
				"ReadContext":        "read",
				"ReadWithoutTimeout": "read",
			})
		}
		return resolveConcreteTypeHandlers(index, pkg, decl, []namedMethodAction{{"Read", "read"}})
	case "list_resource":
		return resolveConcreteTypeHandlers(index, pkg, decl, []namedMethodAction{{"List", "list"}})
	case "ephemeral_resource":
		return resolveConcreteTypeHandlers(index, pkg, decl, []namedMethodAction{{"Open", "open"}, {"Renew", "renew"}, {"Close", "close"}})
	case "action":
		return resolveConcreteTypeHandlers(index, pkg, decl, []namedMethodAction{{"Invoke", "invoke"}})
	default:
		return nil, fmt.Errorf("unsupported kind %q", spec.kind)
	}
}

func isSDKFactory(factory *types.Func) bool {
	signature, ok := factory.Type().(*types.Signature)
	if !ok || signature.Results() == nil || signature.Results().Len() == 0 {
		return false
	}
	return isSchemaResourcePointer(signature.Results().At(0).Type())
}

type namedMethodAction struct {
	method string
	action string
}

func resolveSchemaResourceHandlers(index *packageIndex, pkg *packages.Package, decl *ast.FuncDecl, fieldActions map[string]string) ([]handlerMethod, error) {
	resourceLit, err := resolveReturnedSchemaResourceLiteral(pkg, decl)
	if err != nil {
		return nil, err
	}
	locals := collectLocalExprs(decl.Body)
	handlers := make([]handlerMethod, 0)
	seen := map[string]struct{}{}
	for _, elt := range resourceLit.Elts {
		kv, ok := elt.(*ast.KeyValueExpr)
		if !ok {
			continue
		}
		fieldName := exprIdentName(kv.Key)
		action, ok := fieldActions[fieldName]
		if !ok {
			continue
		}
		funcs, err := resolveFunctionExpr(index, pkg, kv.Value, locals)
		if err != nil {
			return nil, fmt.Errorf("resolve %s handler: %w", action, err)
		}
		if _, ok := seen[action]; ok {
			continue
		}
		seen[action] = struct{}{}
		handlers = append(handlers, handlerMethod{action: action, funcs: funcs})
	}
	if len(handlers) == 0 {
		return nil, errors.New("resolved no SDK resource handlers")
	}
	slices.SortFunc(handlers, compareHandlerMethods)
	return handlers, nil
}

func resolveConcreteTypeHandlers(index *packageIndex, pkg *packages.Package, decl *ast.FuncDecl, actions []namedMethodAction) ([]handlerMethod, error) {
	concreteType, err := resolveReturnedConcreteType(index, pkg, decl)
	if err != nil {
		return nil, err
	}
	named, err := namedType(concreteType)
	if err != nil {
		return nil, err
	}
	methodSet := types.NewMethodSet(types.NewPointer(named))
	handlers := make([]handlerMethod, 0)
	for _, action := range actions {
		selection := methodSet.Lookup(nil, action.method)
		if selection == nil {
			continue
		}
		funcObj, ok := selection.Obj().(*types.Func)
		if !ok {
			continue
		}
		funcs := index.ssaFuncs[funcObj]
		if len(funcs) == 0 {
			return nil, fmt.Errorf("SSA function missing for method %s", action.method)
		}
		handlers = append(handlers, handlerMethod{action: action.action, funcs: funcs})
	}
	if len(handlers) == 0 {
		return nil, errors.New("resolved no framework handlers")
	}
	slices.SortFunc(handlers, compareHandlerMethods)
	return handlers, nil
}

func resolveReturnedSchemaResourceLiteral(pkg *packages.Package, decl *ast.FuncDecl) (*ast.CompositeLit, error) {
	if decl.Body == nil {
		return nil, errors.New("factory has no body")
	}
	locals := collectLocalExprs(decl.Body)
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		lit, ok, err := unwrapSchemaResourceLiteral(pkg, resolveExpr(returnStmt.Results[0], locals), locals)
		if err != nil {
			return nil, err
		}
		if ok {
			return lit, nil
		}
	}
	return nil, errors.New("returned *schema.Resource literal not found")
}

func unwrapSchemaResourceLiteral(pkg *packages.Package, expr ast.Expr, locals map[string]ast.Expr) (*ast.CompositeLit, bool, error) {
	switch expr := expr.(type) {
	case *ast.Ident:
		resolved := resolveExpr(expr, locals)
		if resolved == expr {
			return nil, false, nil
		}
		return unwrapSchemaResourceLiteral(pkg, resolved, locals)
	case *ast.UnaryExpr:
		if expr.Op != token.AND {
			return nil, false, nil
		}
		lit, ok := expr.X.(*ast.CompositeLit)
		if !ok {
			return nil, false, nil
		}
		if !isSchemaResourceType(pkg.TypesInfo.TypeOf(lit)) {
			return nil, false, nil
		}
		return lit, true, nil
	case *ast.CompositeLit:
		if !isSchemaResourceType(pkg.TypesInfo.TypeOf(expr)) {
			return nil, false, nil
		}
		return expr, true, nil
	default:
		return nil, false, nil
	}
}

func resolveReturnedConcreteType(index *packageIndex, pkg *packages.Package, decl *ast.FuncDecl) (types.Type, error) {
	if decl.Body == nil {
		return nil, errors.New("factory has no body")
	}
	locals := collectLocalExprs(decl.Body)
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		typ, err := concreteTypeFromExpr(index, pkg, resolveExpr(returnStmt.Results[0], locals), locals, map[*types.Func]struct{}{})
		if err != nil {
			return nil, err
		}
		if typ != nil {
			return typ, nil
		}
	}
	return nil, errors.New("returned concrete type not found")
}

func concreteTypeFromExpr(
	index *packageIndex,
	pkg *packages.Package,
	expr ast.Expr,
	locals map[string]ast.Expr,
	seen map[*types.Func]struct{},
) (types.Type, error) {
	expr = resolveExpr(expr, locals)
	if typ := pkg.TypesInfo.TypeOf(expr); typ != nil && !isInterfaceType(typ) {
		return typ, nil
	}

	callExpr, ok := expr.(*ast.CallExpr)
	if !ok {
		return nil, nil
	}
	calleeObj, calleePkg, err := resolveFunctionObject(index, pkg, callExpr.Fun, locals)
	if err != nil {
		return nil, err
	}
	if calleeObj == nil {
		return nil, nil
	}
	if _, ok := seen[calleeObj]; ok {
		return nil, nil
	}
	seen[calleeObj] = struct{}{}

	decl := index.funcDecls[calleeObj]
	if decl == nil {
		return nil, fmt.Errorf("function declaration missing for %s", calleeObj.Name())
	}
	if calleePkg == nil {
		calleePkg = index.byPath[calleeObj.Pkg().Path()]
	}
	if calleePkg == nil {
		return nil, fmt.Errorf("package not loaded for %s", calleeObj.Name())
	}
	calleeLocals := collectLocalExprs(decl.Body)
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		typ, err := concreteTypeFromExpr(index, calleePkg, resolveExpr(returnStmt.Results[0], calleeLocals), calleeLocals, seen)
		if err != nil {
			return nil, err
		}
		if typ != nil {
			return typ, nil
		}
	}
	return nil, nil
}

func collectAPIMethods(index *packageIndex, roots []*ssa.Function, sdkMappings map[sdkMethodKey][]apiMethod) []apiMethod {
	queue := expandRootFunctions(index, roots)
	seen := map[*ssa.Function]struct{}{}
	methods := make([]apiMethod, 0)
	for len(queue) > 0 {
		fn := queue[len(queue)-1]
		queue = queue[:len(queue)-1]
		if fn == nil {
			continue
		}
		if _, ok := seen[fn]; ok {
			continue
		}
		seen[fn] = struct{}{}
		if key, ok := sdkKeyForFunction(fn); ok {
			methods = append(methods, sdkMappings[key]...)
		}
		methods = append(methods, collectDirectSDKAPIMethods(index, fn, sdkMappings)...)
		queue = append(queue, index.nestedFuncs[fn]...)
		queue = append(queue, index.callers[fn]...)
	}
	slices.SortFunc(methods, compareAPIMethods)
	return slices.Compact(methods)
}

func expandRootFunctions(index *packageIndex, roots []*ssa.Function) []*ssa.Function {
	expanded := append([]*ssa.Function(nil), roots...)
	for _, root := range roots {
		if root == nil || root.Syntax() == nil {
			continue
		}
		ast.Inspect(root.Syntax(), func(node ast.Node) bool {
			funcLit, ok := node.(*ast.FuncLit)
			if !ok {
				return true
			}
			expanded = append(expanded, index.ssaBySyntax[funcLit]...)
			return true
		})
	}
	return expanded
}

func collectDirectSDKAPIMethods(index *packageIndex, fn *ssa.Function, sdkMappings map[sdkMethodKey][]apiMethod) []apiMethod {
	methods := make([]apiMethod, 0)
	for _, block := range fn.Blocks {
		for _, instruction := range block.Instrs {
			callInstruction, ok := instruction.(ssa.CallInstruction)
			if !ok {
				continue
			}
			common := callInstruction.Common()
			if common == nil {
				continue
			}
			callee := common.StaticCallee()
			if callee == nil {
				continue
			}
			if key, ok := sdkKeyForFunction(callee); ok {
				methods = append(methods, sdkMappings[key]...)
			}
		}
	}
	methods = append(methods, collectDirectASTSDKAPIMethods(index, fn, sdkMappings)...)
	return methods
}

func collectDirectASTSDKAPIMethods(index *packageIndex, fn *ssa.Function, sdkMappings map[sdkMethodKey][]apiMethod) []apiMethod {
	if fn == nil || fn.Syntax() == nil {
		return nil
	}
	ssaPkg := fn.Package()
	if ssaPkg == nil || ssaPkg.Pkg == nil {
		return nil
	}
	sourcePkg := index.byPath[ssaPkg.Pkg.Path()]
	if sourcePkg == nil {
		return nil
	}
	methods := make([]apiMethod, 0)
	ast.Inspect(fn.Syntax(), func(node ast.Node) bool {
		callExpr, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		obj := calledFunctionObject(sourcePkg, callExpr)
		if obj == nil {
			return true
		}
		if key, ok := sdkKeyForObject(obj); ok {
			methods = append(methods, sdkMappings[key]...)
		}
		return true
	})
	return methods
}

func calledFunctionObject(pkg *packages.Package, callExpr *ast.CallExpr) *types.Func {
	switch fun := callExpr.Fun.(type) {
	case *ast.Ident:
		funcObj, _ := pkg.TypesInfo.ObjectOf(fun).(*types.Func)
		return funcObj
	case *ast.SelectorExpr:
		if selection, ok := pkg.TypesInfo.Selections[fun]; ok {
			funcObj, _ := selection.Obj().(*types.Func)
			return funcObj
		}
		funcObj, _ := pkg.TypesInfo.ObjectOf(fun.Sel).(*types.Func)
		return funcObj
	default:
		return nil
	}
}

func sdkKeyForObject(obj *types.Func) (sdkMethodKey, bool) {
	if obj == nil || obj.Pkg() == nil {
		return sdkMethodKey{}, false
	}
	signature, ok := obj.Type().(*types.Signature)
	if !ok {
		return sdkMethodKey{}, false
	}
	receiver := receiverName(signature)
	if receiver == "" {
		return sdkMethodKey{}, false
	}
	return sdkMethodKey{pkg: obj.Pkg().Path(), receiver: receiver, method: obj.Name()}, true
}

func sdkKeyForFunction(fn *ssa.Function) (sdkMethodKey, bool) {
	if fn == nil {
		return sdkMethodKey{}, false
	}
	pkg := fn.Package()
	if pkg == nil || pkg.Pkg == nil {
		return sdkMethodKey{}, false
	}
	receiver := receiverName(fn.Signature)
	if receiver == "" {
		return sdkMethodKey{}, false
	}
	return sdkMethodKey{pkg: pkg.Pkg.Path(), receiver: receiver, method: fn.Name()}, true
}

func receiverName(signature *types.Signature) string {
	if signature == nil || signature.Recv() == nil {
		return ""
	}
	receiverType := signature.Recv().Type()
	for {
		pointer, ok := receiverType.(*types.Pointer)
		if !ok {
			break
		}
		receiverType = pointer.Elem()
	}
	if named, ok := receiverType.(*types.Named); ok && named.Obj() != nil {
		return named.Obj().Name()
	}
	return ""
}

func extractSpecsFromServiceMethod(pkg *packages.Package, decl *ast.FuncDecl, kind, sourceFile string) ([]entrypointSpec, error) {
	if decl.Body == nil {
		return nil, fmt.Errorf("service package method %s has no body", decl.Name.Name)
	}
	locals := collectLocalExprs(decl.Body)
	specs := make([]entrypointSpec, 0)
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		items, ok, err := unwrapSpecList(resolveExpr(returnStmt.Results[0], locals), locals)
		if err != nil {
			return nil, err
		}
		if !ok {
			continue
		}
		for _, item := range items {
			spec, err := entrypointFromCompositeLit(pkg, kind, sourceFile, item)
			if err != nil {
				return nil, err
			}
			specs = append(specs, spec)
		}
	}
	return specs, nil
}

func unwrapSpecList(expr ast.Expr, locals map[string]ast.Expr) ([]*ast.CompositeLit, bool, error) {
	switch expr := expr.(type) {
	case *ast.Ident:
		resolved := resolveExpr(expr, locals)
		if resolved == expr {
			return nil, false, nil
		}
		return unwrapSpecList(resolved, locals)
	case *ast.CallExpr:
		if selector, ok := expr.Fun.(*ast.SelectorExpr); ok && selector.Sel != nil && selector.Sel.Name == "Values" && len(expr.Args) == 1 {
			return unwrapSpecList(resolveExpr(expr.Args[0], locals), locals)
		}
		return nil, false, nil
	case *ast.CompositeLit:
		items := make([]*ast.CompositeLit, 0, len(expr.Elts))
		for _, elt := range expr.Elts {
			lit, ok := elt.(*ast.CompositeLit)
			if !ok {
				return nil, false, fmt.Errorf("unexpected spec element %T", elt)
			}
			items = append(items, lit)
		}
		return items, true, nil
	default:
		return nil, false, nil
	}
}

func entrypointFromCompositeLit(pkg *packages.Package, kind, sourceFile string, lit *ast.CompositeLit) (entrypointSpec, error) {
	var factory string
	var typeName string
	for _, elt := range lit.Elts {
		kv, ok := elt.(*ast.KeyValueExpr)
		if !ok {
			continue
		}
		key := exprIdentName(kv.Key)
		switch key {
		case "Factory":
			factory = exprIdentName(kv.Value)
		case "TypeName":
			value, err := stringLiteralValue(kv.Value)
			if err != nil {
				return entrypointSpec{}, err
			}
			typeName = value
		}
	}
	if factory == "" || typeName == "" {
		return entrypointSpec{}, errors.New("entrypoint spec missing Factory or TypeName")
	}
	return entrypointSpec{
		kind:       kind,
		typeName:   typeName,
		factory:    factory,
		sourceFile: sourceFile,
		packagePath: pkg.PkgPath,
	}, nil
}

func resolveFunctionExpr(index *packageIndex, pkg *packages.Package, expr ast.Expr, locals map[string]ast.Expr) ([]*ssa.Function, error) {
	expr = resolveExpr(expr, locals)
	switch expr := expr.(type) {
	case *ast.Ident:
		obj, ok := pkg.TypesInfo.ObjectOf(expr).(*types.Func)
		if !ok {
			return nil, fmt.Errorf("%s does not resolve to a function", expr.Name)
		}
		return ssaFunctionsForObject(index, obj, expr.Name)
	case *ast.SelectorExpr:
		obj, ok := pkg.TypesInfo.ObjectOf(expr.Sel).(*types.Func)
		if !ok {
			return nil, errors.New("selector does not resolve to a function")
		}
		return ssaFunctionsForObject(index, obj, expr.Sel.Name)
	case *ast.FuncLit:
		funcs := index.ssaBySyntax[expr]
		if len(funcs) == 0 {
			return nil, errors.New("SSA function missing for function literal")
		}
		return funcs, nil
	case *ast.CallExpr:
		callee, err := resolveFunctionExpr(index, pkg, expr.Fun, locals)
		if err != nil {
			return nil, err
		}
		if len(callee) == 0 {
			return nil, errors.New("call expression callee resolved no functions")
		}
		calleeObj, ok := callee[0].Object().(*types.Func)
		if !ok {
			return nil, errors.New("call expression callee is not a named function")
		}
		decl := index.funcDecls[calleeObj]
		if decl == nil {
			return nil, fmt.Errorf("function declaration missing for %s", calleeObj.Name())
		}
		returned, err := resolveReturnedHandlerExpr(decl)
		if err != nil {
			return nil, err
		}
		switch returned := returned.(type) {
		case *ast.FuncLit:
			funcs := index.ssaBySyntax[returned]
			if len(funcs) == 0 {
				return nil, fmt.Errorf("SSA function missing for returned closure in %s", calleeObj.Name())
			}
			return funcs, nil
		default:
			return resolveFunctionExpr(index, pkg, returned, collectLocalExprs(decl.Body))
		}
	default:
		return nil, fmt.Errorf("unsupported handler expression %T", expr)
	}
}

func resolveFunctionObject(
	index *packageIndex,
	pkg *packages.Package,
	expr ast.Expr,
	locals map[string]ast.Expr,
) (*types.Func, *packages.Package, error) {
	expr = resolveExpr(expr, locals)
	switch expr := expr.(type) {
	case *ast.Ident:
		obj, ok := pkg.TypesInfo.ObjectOf(expr).(*types.Func)
		if !ok {
			return nil, nil, fmt.Errorf("%s does not resolve to a function", expr.Name)
		}
		return obj, pkg, nil
	case *ast.SelectorExpr:
		obj, ok := pkg.TypesInfo.ObjectOf(expr.Sel).(*types.Func)
		if !ok {
			return nil, nil, errors.New("selector does not resolve to a function")
		}
		var calleePkg *packages.Package
		if obj.Pkg() != nil {
			calleePkg = index.byPath[obj.Pkg().Path()]
		}
		return obj, calleePkg, nil
	default:
		return nil, nil, fmt.Errorf("unsupported function expression %T", expr)
	}
}

func ssaFunctionsForObject(index *packageIndex, obj *types.Func, name string) ([]*ssa.Function, error) {
	funcs := index.ssaFuncs[obj]
	if len(funcs) == 0 {
		return nil, fmt.Errorf("SSA function missing for %s", name)
	}
	return funcs, nil
}

func resolveReturnedHandlerExpr(decl *ast.FuncDecl) (ast.Expr, error) {
	if decl.Body == nil {
		return nil, errors.New("factory has no body")
	}
	locals := collectLocalExprs(decl.Body)
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		return resolveExpr(returnStmt.Results[0], locals), nil
	}
	return nil, errors.New("returned handler expression not found")
}

func namedType(typ types.Type) (*types.Named, error) {
	for {
		pointer, ok := typ.(*types.Pointer)
		if !ok {
			break
		}
		typ = pointer.Elem()
	}
	named, ok := typ.(*types.Named)
	if !ok {
		return nil, fmt.Errorf("expected named type, got %T", typ)
	}
	return named, nil
}

func collectLocalExprs(body *ast.BlockStmt) map[string]ast.Expr {
	locals := map[string]ast.Expr{}
	if body == nil {
		return locals
	}
	for _, stmt := range body.List {
		collectStmtLocals(stmt, locals)
	}
	return locals
}

func collectStmtLocals(stmt ast.Stmt, locals map[string]ast.Expr) {
	switch stmt := stmt.(type) {
	case *ast.AssignStmt:
		for i, lhs := range stmt.Lhs {
			ident, ok := lhs.(*ast.Ident)
			if !ok || ident.Name == "_" || i >= len(stmt.Rhs) {
				continue
			}
			locals[ident.Name] = stmt.Rhs[i]
		}
	case *ast.DeclStmt:
		gen, ok := stmt.Decl.(*ast.GenDecl)
		if !ok {
			return
		}
		for _, spec := range gen.Specs {
			valueSpec, ok := spec.(*ast.ValueSpec)
			if !ok {
				continue
			}
			for i, name := range valueSpec.Names {
				if name == nil || name.Name == "_" || i >= len(valueSpec.Values) {
					continue
				}
				locals[name.Name] = valueSpec.Values[i]
			}
		}
	case *ast.IfStmt:
		if stmt.Init != nil {
			collectStmtLocals(stmt.Init, locals)
		}
	case *ast.ForStmt:
		if stmt.Init != nil {
			collectStmtLocals(stmt.Init, locals)
		}
	case *ast.RangeStmt:
		if stmt.Key != nil {
			if ident, ok := stmt.Key.(*ast.Ident); ok && ident.Name != "_" {
				locals[ident.Name] = stmt.X
			}
		}
	}
}

func isInterfaceType(typ types.Type) bool {
	_, ok := typ.Underlying().(*types.Interface)
	return ok
}

func resolveExpr(expr ast.Expr, locals map[string]ast.Expr) ast.Expr {
	if _, ok := expr.(*ast.Ident); !ok {
		return expr
	}
	seen := map[string]struct{}{}
	current := expr
	for {
		ident, ok := current.(*ast.Ident)
		if !ok {
			return current
		}
		if _, ok := seen[ident.Name]; ok {
			return current
		}
		seen[ident.Name] = struct{}{}
		next, ok := locals[ident.Name]
		if !ok {
			return current
		}
		current = next
	}
}

func isServicePackageMethod(decl *ast.FuncDecl) bool {
	if decl == nil || decl.Recv == nil || len(decl.Recv.List) != 1 {
		return false
	}
	return receiverTypeName(decl.Recv.List[0].Type) == "servicePackage"
}

func receiverTypeName(expr ast.Expr) string {
	switch expr := expr.(type) {
	case *ast.Ident:
		return expr.Name
	case *ast.StarExpr:
		return receiverTypeName(expr.X)
	default:
		return ""
	}
}

func serviceMethodKind(name string) (string, bool) {
	switch name {
	case "SDKResources", "FrameworkResources":
		return "resource", true
	case "SDKDataSources", "FrameworkDataSources":
		return "data_source", true
	case "FrameworkListResources", "SDKListResources":
		return "list_resource", true
	case "EphemeralResources":
		return "ephemeral_resource", true
	case "Actions":
		return "action", true
	default:
		return "", false
	}
}

func fileForPath(pkg *packages.Package, path string) (*ast.File, error) {
	for i, filePath := range pkg.GoFiles {
		if filepath.Clean(filePath) == filepath.Clean(path) {
			return pkg.Syntax[i], nil
		}
	}
	return nil, fmt.Errorf("file %s not found in package %s", path, pkg.PkgPath)
}

func stringLiteralValue(expr ast.Expr) (string, error) {
	basic, ok := expr.(*ast.BasicLit)
	if !ok || basic.Kind != token.STRING {
		return "", fmt.Errorf("expected string literal, got %T", expr)
	}
	return strconv.Unquote(basic.Value)
}

func exprIdentName(expr ast.Expr) string {
	switch expr := expr.(type) {
	case *ast.Ident:
		return expr.Name
	case *ast.SelectorExpr:
		return expr.Sel.Name
	default:
		return ""
	}
}

func isSchemaResourcePointer(typ types.Type) bool {
	pointer, ok := typ.(*types.Pointer)
	if !ok {
		return false
	}
	return isSchemaResourceType(pointer.Elem())
}

func isSchemaResourceType(typ types.Type) bool {
	named, ok := typ.(*types.Named)
	if !ok || named.Obj() == nil || named.Obj().Pkg() == nil {
		return false
	}
	return named.Obj().Pkg().Path() == "github.com/hashicorp/terraform-plugin-sdk/v2/helper/schema" && named.Obj().Name() == "Resource"
}

func compareHandlerMethods(left, right handlerMethod) int {
	return compareStrings(left.action, right.action)
}

func compareMappingRows(left, right mappingRow) int {
	if cmp := compareStrings(left.Kind, right.Kind); cmp != 0 {
		return cmp
	}
	if cmp := compareStrings(left.TypeName, right.TypeName); cmp != 0 {
		return cmp
	}
	if cmp := compareStrings(left.Action, right.Action); cmp != 0 {
		return cmp
	}
	return compareAPIMethodSlices(left.APIMethods, right.APIMethods)
}

func compactMappingRows(rows []mappingRow) []mappingRow {
	if len(rows) < 2 {
		return rows
	}
	compacted := rows[:1]
	for _, row := range rows[1:] {
		if compareMappingRows(compacted[len(compacted)-1], row) == 0 {
			continue
		}
		compacted = append(compacted, row)
	}
	return compacted
}

func compareAPIMethodSlices(left, right []apiMethod) int {
	limit := len(left)
	if len(right) < limit {
		limit = len(right)
	}
	for i := 0; i < limit; i++ {
		if cmp := compareAPIMethods(left[i], right[i]); cmp != 0 {
			return cmp
		}
	}
	if len(left) < len(right) {
		return -1
	}
	if len(left) > len(right) {
		return 1
	}
	return 0
}

func compareAPIMethods(left, right apiMethod) int {
	if cmp := compareStrings(left.Service, right.Service); cmp != 0 {
		return cmp
	}
	return compareStrings(left.Name, right.Name)
}

func compareStrings(left, right string) int {
	if left < right {
		return -1
	}
	if left > right {
		return 1
	}
	return 0
}
