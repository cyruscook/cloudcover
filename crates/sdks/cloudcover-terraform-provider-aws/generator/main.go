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
	"os/exec"
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
	prog                 *ssa.Program
	byTypes              map[*types.Package]*packages.Package
	byPath               map[string][]*packages.Package
	byFile               map[string]*packages.Package
	funcDecls            map[*types.Func]*ast.FuncDecl
	funcDeclsByIdentity  map[functionIdentity][]*ast.FuncDecl
	ssaFuncs             map[*types.Func][]*ssa.Function
	ssaFuncsByIdentity   map[functionIdentity][]*ssa.Function
	ssaBySyntax          map[ast.Node][]*ssa.Function
	nestedFuncs          map[*ssa.Function][]*ssa.Function
	callers              map[*ssa.Function][]*ssa.Function
	interfaceCalleeCache map[functionIdentity]providerInterfaceCalleeResolution
}

type functionIdentity struct {
	packagePath string
	receiver    string
	method      string
	signature   string
}
type sourceDeclarationIdentity struct {
	packagePath string
	receiver    string
	method      string
	file        string
	start       int
	end         int
	declaration *ast.FuncDecl
}

type ssaFunctionSourceIdentity struct {
	declarationFile   string
	declarationOffset int
	declarationSyntax *ast.FuncDecl
	origin            functionIdentity
}

type handlerMethod struct {
	action string
	funcs  []*ssa.Function
}

type handlerAPIMethods struct {
	methods []apiMethod
}

type apiMethodSummaryState uint8

const (
	apiMethodSummaryInProgress apiMethodSummaryState = iota + 1
	apiMethodSummaryComplete
)

type apiMethodFunctionSummary struct {
	state   apiMethodSummaryState
	direct  []apiMethod
	callees []*ssa.Function
	methods []apiMethod
}

type providerInterfaceCalleeResolution struct {
	callees  []*ssa.Function
	relevant bool
	err      error
}

type apiMethodAnalyzer struct {
	index       *packageIndex
	sdkMappings map[sdkMethodKey][]apiMethod
	summaries   map[*ssa.Function]*apiMethodFunctionSummary
}

func newAPIMethodAnalyzer(index *packageIndex, sdkMappings map[sdkMethodKey][]apiMethod) *apiMethodAnalyzer {
	return &apiMethodAnalyzer{
		index:       index,
		sdkMappings: sdkMappings,
		summaries:   make(map[*ssa.Function]*apiMethodFunctionSummary),
	}
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
	analyzer := newAPIMethodAnalyzer(index, sdkMappings)

	rows := make([]mappingRow, 0, len(specs))
	for _, spec := range specs {
		handlers, err := resolveHandlers(index, spec)
		if err != nil {
			return fmt.Errorf(
				"resolve %s %s factory %s in %s: %w",
				spec.kind,
				spec.typeName,
				spec.factory,
				spec.sourceFile,
				err,
			)
		}
		for _, handler := range handlers {
			analysis, err := analyzer.collect(handler.funcs)
			if err != nil {
				return fmt.Errorf(
					"collect API methods for %s %s %s handler: %w",
					spec.kind,
					spec.typeName,
					handler.action,
					err,
				)
			}
			rows = append(rows, mappingRow{
				Kind:       spec.kind,
				TypeName:   spec.typeName,
				Action:     handler.action,
				APIMethods: analysis.methods,
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
		if !isAWSV2ServicePackage(row.Package) && !isAWSV1ServicePackage(row.Package) {
			return nil, fmt.Errorf("SDK mapping row has a non-AWS service package %s", row.Package)
		}
		methods := append([]apiMethod(nil), row.APIMethods...)
		slices.SortFunc(methods, compareAPIMethods)
		methods = slices.Compact(methods)
		if len(methods) == 0 {
			return nil, fmt.Errorf("SDK mapping row %s %s.%s has no API methods", row.Package, row.Receiver, row.Method)
		}
		for _, method := range methods {
			if method.Service == "" || method.Name == "" {
				return nil, fmt.Errorf("SDK mapping row %s %s.%s has an incomplete API method", row.Package, row.Receiver, row.Method)
			}
		}
		key := sdkMethodKey{pkg: row.Package, receiver: row.Receiver, method: row.Method}
		if existing, ok := mappings[key]; ok {
			if !slices.Equal(existing, methods) {
				return nil, fmt.Errorf("SDK mapping rows disagree for %s %s.%s", row.Package, row.Receiver, row.Method)
			}
			continue
		}
		mappings[key] = methods
	}
	return mappings, nil
}

func loadProviderIndex(providerDir string) (*packageIndex, error) {
	patterns := []string{"./aws"}
	packageEnv := os.Environ()
	if info, err := os.Stat(filepath.Join(providerDir, "internal", "service")); err == nil && info.IsDir() {
		patterns = []string{"./internal/service/...", "./internal/provider"}
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	hasModule := false
	if _, err := os.Stat(filepath.Join(providerDir, "go.mod")); errors.Is(err, os.ErrNotExist) {
		gopath := filepath.Dir(filepath.Dir(filepath.Dir(filepath.Dir(providerDir))))
		packageEnv = append(packageEnv, "GO111MODULE=off", "GOPATH="+gopath)
	} else if err != nil {
		return nil, err
	} else {
		hasModule = true
	}
	if hasModule {
		cmdArgs := []string{"mod", "edit", "-droprequire=github.com/golangci/golangci-lint", "-dropgodebug=tlskyber"}
		if module, err := os.ReadFile(filepath.Join(providerDir, "go.mod")); err == nil &&
			!strings.Contains(string(module), providerModulePath) {
			_ = os.Remove(filepath.Join(providerDir, "go.sum"))
			packageEnv = append(packageEnv, "GOSUMDB=off", "GOFLAGS=-mod=mod")
		}
		cmd := exec.Command("go", cmdArgs...)
		cmd.Dir = providerDir
		_ = cmd.Run()
	}
	initial, err := packages.Load(&packages.Config{
		Mode:       packages.LoadAllSyntax,
		Dir:        providerDir,
		Env:        packageEnv,
		Tests:      false,
		BuildFlags: nil,
		Overlay:    nil,
	}, patterns...)
	if err != nil {
		return nil, fmt.Errorf("packages.Load failed for %s: %w", providerDir, err)
	}
	if count := packages.PrintErrors(initial); count != 0 {
		return nil, fmt.Errorf("package loading reported %d diagnostics for %s", count, providerDir)
	}
	prog, ssaPackages := ssautil.AllPackages(initial, ssa.InstantiateGenerics)
	for _, pkg := range ssaPackages {
		if pkg == nil || pkg.Pkg == nil {
			return nil, errors.New("SSA package missing after package load")
		}
		pkg.Build()
	}
	cg := static.CallGraph(prog)

	index := &packageIndex{
		prog:                 prog,
		byTypes:              map[*types.Package]*packages.Package{},
		byPath:               map[string][]*packages.Package{},
		byFile:               map[string]*packages.Package{},
		funcDecls:            map[*types.Func]*ast.FuncDecl{},
		funcDeclsByIdentity:  map[functionIdentity][]*ast.FuncDecl{},
		ssaFuncs:             map[*types.Func][]*ssa.Function{},
		ssaFuncsByIdentity:   map[functionIdentity][]*ssa.Function{},
		ssaBySyntax:          map[ast.Node][]*ssa.Function{},
		nestedFuncs:          map[*ssa.Function][]*ssa.Function{},
		callers:              map[*ssa.Function][]*ssa.Function{},
		interfaceCalleeCache: map[functionIdentity]providerInterfaceCalleeResolution{},
	}

	missingTypes := make([]string, 0)
	packages.Visit(initial, nil, func(pkg *packages.Package) {
		if pkg == nil {
			missingTypes = append(missingTypes, "<nil>")
			return
		}
		if pkg.Types == nil {
			missingTypes = append(missingTypes, pkg.PkgPath)
			return
		}
		index.byTypes[pkg.Types] = pkg
		index.byPath[pkg.PkgPath] = append(index.byPath[pkg.PkgPath], pkg)
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
				if !ok {
					continue
				}
				index.funcDecls[obj] = funcDecl
				if identity, ok := canonicalFunctionIdentity(obj); ok {
					index.funcDeclsByIdentity[identity] = append(index.funcDeclsByIdentity[identity], funcDecl)
				}
			}
		}
	})
	if len(missingTypes) != 0 {
		slices.Sort(missingTypes)
		return nil, fmt.Errorf("package loading omitted type information for %s", strings.Join(slices.Compact(missingTypes), ", "))
	}

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
		if !ok {
			continue
		}
		index.ssaFuncs[obj] = append(index.ssaFuncs[obj], fn)
		if identity, ok := canonicalFunctionIdentity(obj); ok {
			index.ssaFuncsByIdentity[identity] = append(index.ssaFuncsByIdentity[identity], fn)
		}
	}
	for _, fns := range index.ssaFuncs {
		slices.SortFunc(fns, func(left, right *ssa.Function) int {
			return compareStrings(left.String(), right.String())
		})
	}
	for _, fns := range index.ssaFuncsByIdentity {
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

type servicePackageRegistryHelper struct {
	kind                 string
	field                string
	argumentCount        int
	requiresSingleton    bool
	usesRegistrationName bool
	factoryType          types.Type
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

	registryHelpers := make(map[*packages.Package]map[*types.Func]servicePackageRegistryHelper)
	for _, path := range paths {
		pkg := index.byFile[path]
		if pkg == nil || pkg.TypesInfo == nil {
			return nil, fmt.Errorf("service package registry has no type information for %s", path)
		}
		file, err := fileForPath(pkg, path)
		if err != nil {
			return nil, err
		}
		for _, decl := range file.Decls {
			funcDecl, ok := decl.(*ast.FuncDecl)
			if !ok {
				continue
			}
			helper, ok := servicePackageRegistration(pkg, funcDecl)
			if !ok {
				continue
			}
			function, ok := pkg.TypesInfo.Defs[funcDecl.Name].(*types.Func)
			if !ok {
				return nil, fmt.Errorf("service package registry function %s has no type information", funcDecl.Name.Name)
			}
			if registryHelpers[pkg] == nil {
				registryHelpers[pkg] = make(map[*types.Func]servicePackageRegistryHelper)
			}
			registryHelpers[pkg][function] = helper
		}
	}

	specs := make([]entrypointSpec, 0)
	for _, path := range paths {
		pkg := index.byFile[path]
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
			if field, ok := serviceMethodFactoryCollectionField(pkg, funcDecl, kind); ok {
				if !hasServicePackageRegistryHelper(registryHelpers[pkg], kind, field) {
					return nil, fmt.Errorf("service package method %s returned factory collection field %s without a matching registry helper", funcDecl.Name.Name, field)
				}
			}
			methodSpecs, err := extractSpecsFromServiceMethod(index, pkg, funcDecl, kind, path)
			if err != nil {
				return nil, err
			}
			specs = append(specs, methodSpecs...)
		}
	}
	registryPackages := make([]*packages.Package, 0, len(registryHelpers))
	for pkg := range registryHelpers {
		registryPackages = append(registryPackages, pkg)
	}
	slices.SortFunc(registryPackages, func(left, right *packages.Package) int {
		return compareStrings(left.PkgPath, right.PkgPath)
	})
	for _, pkg := range registryPackages {
		registrySpecs, err := discoverServicePackageRegistrations(index, pkg, registryHelpers[pkg])
		if err != nil {
			return nil, err
		}
		specs = append(specs, registrySpecs...)
	}
	if len(paths) != 0 {
		legacySpecs, err := discoverProviderMapEntrypoints(index)
		if err != nil {
			return nil, err
		}
		specs = append(specs, legacySpecs...)
		if len(specs) == 0 {
			return nil, errors.New("service package entrypoint files yielded no entrypoints")
		}
		return compactEntrypointSpecs(specs)
	}
	providerSpecs, err := discoverProviderMapEntrypoints(index)
	if err != nil {
		return nil, err
	}
	if len(providerSpecs) == 0 {
		return nil, errors.New("discovered no terraform-provider-aws entrypoints")
	}
	return providerSpecs, nil
}

func servicePackageRegistration(pkg *packages.Package, decl *ast.FuncDecl) (servicePackageRegistryHelper, bool) {
	if decl == nil || decl.Name == nil {
		return servicePackageRegistryHelper{}, false
	}
	switch decl.Name.Name {
	case "registerFrameworkResourceFactory":
		helper := servicePackageRegistryHelper{kind: "resource", field: "frameworkResourceFactories", argumentCount: 1}
		if !servicePackageRegistrationHasFactoryArgument(decl) {
			return servicePackageRegistryHelper{}, false
		}
		if decl.Recv != nil && (!isServicePackageMethod(decl) || !servicePackageRegistrationStoresFactory(decl, helper.field)) {
			return servicePackageRegistryHelper{}, false
		}
		return helper, true
	case "registerFrameworkDataSourceFactory":
		helper := servicePackageRegistryHelper{kind: "data_source", field: "frameworkDataSourceFactories", argumentCount: 1}
		if !servicePackageRegistrationHasFactoryArgument(decl) {
			return servicePackageRegistryHelper{}, false
		}
		if decl.Recv != nil && (!isServicePackageMethod(decl) || !servicePackageRegistrationStoresFactory(decl, helper.field)) {
			return servicePackageRegistryHelper{}, false
		}
		return helper, true
	case "registerSDKResourceFactory":
		return sdkServicePackageRegistration(pkg, decl, "resource", "sdkResourceFactories")
	case "registerSDKDataSourceFactory":
		return sdkServicePackageRegistration(pkg, decl, "data_source", "sdkDataSourceFactories")
	default:
		return servicePackageRegistryHelper{}, false
	}
}

func servicePackageRegistrationHasFactoryArgument(decl *ast.FuncDecl) bool {
	return decl.Type != nil && decl.Type.Params != nil && len(decl.Type.Params.List) == 1 &&
		(decl.Type.Results == nil || len(decl.Type.Results.List) == 0) &&
		func() bool {
			_, ok := decl.Type.Params.List[0].Type.(*ast.FuncType)
			return ok
		}()
}

func sdkServicePackageRegistration(pkg *packages.Package, decl *ast.FuncDecl, kind, field string) (servicePackageRegistryHelper, bool) {
	if pkg == nil || pkg.TypesInfo == nil || decl == nil || decl.Recv == nil || !isServicePackageMethod(decl) ||
		decl.Type == nil || decl.Type.Params == nil || len(decl.Type.Params.List) != 2 ||
		(decl.Type.Results != nil && len(decl.Type.Results.List) != 0) {
		return servicePackageRegistryHelper{}, false
	}
	if len(decl.Recv.List) != 1 || len(decl.Recv.List[0].Names) != 1 ||
		len(decl.Type.Params.List[0].Names) != 1 || len(decl.Type.Params.List[1].Names) != 1 {
		return servicePackageRegistryHelper{}, false
	}
	function, ok := pkg.TypesInfo.Defs[decl.Name].(*types.Func)
	if !ok {
		return servicePackageRegistryHelper{}, false
	}
	signature, ok := function.Type().(*types.Signature)
	if !ok || signature.Recv() == nil || signature.Params().Len() != 2 || signature.Results().Len() != 0 ||
		!types.Identical(signature.Params().At(0).Type(), types.Typ[types.String]) {
		return servicePackageRegistryHelper{}, false
	}
	factoryType := signature.Params().At(1).Type()
	if !isSDKFactoryType(factoryType) || !servicePackageRegistrationStoresSDKFactory(pkg, decl, field, factoryType) {
		return servicePackageRegistryHelper{}, false
	}
	return servicePackageRegistryHelper{
		kind:                 kind,
		field:                field,
		argumentCount:        2,
		requiresSingleton:    true,
		usesRegistrationName: true,
		factoryType:          factoryType,
	}, true
}

func isSDKFactoryType(typ types.Type) bool {
	signature, ok := typ.Underlying().(*types.Signature)
	return ok && signature.Params().Len() == 0 && signature.Results().Len() == 1 &&
		func() bool {
			_, ok := signature.Results().At(0).Type().(*types.Pointer)
			return ok
		}()
}

func servicePackageRegistrationStoresFactory(decl *ast.FuncDecl, field string) bool {
	if decl == nil || decl.Body == nil || decl.Recv == nil || len(decl.Recv.List) != 1 ||
		decl.Type == nil || decl.Type.Params == nil || len(decl.Type.Params.List) != 1 ||
		len(decl.Recv.List[0].Names) != 1 || len(decl.Type.Params.List[0].Names) != 1 {
		return false
	}
	receiverName := decl.Recv.List[0].Names[0].Name
	factoryName := decl.Type.Params.List[0].Names[0].Name
	for _, stmt := range decl.Body.List {
		assign, ok := stmt.(*ast.AssignStmt)
		if !ok || len(assign.Lhs) != 1 || len(assign.Rhs) != 1 ||
			!servicePackageFieldSelector(assign.Lhs[0], receiverName, field) {
			continue
		}
		call, ok := assign.Rhs[0].(*ast.CallExpr)
		if !ok || exprIdentName(call.Fun) != "append" || len(call.Args) != 2 ||
			!servicePackageFieldSelector(call.Args[0], receiverName, field) {
			continue
		}
		factory, ok := call.Args[1].(*ast.Ident)
		if ok && factory.Name == factoryName {
			return true
		}
	}
	return false
}

func servicePackageRegistrationStoresSDKFactory(
	pkg *packages.Package,
	decl *ast.FuncDecl,
	field string,
	factoryType types.Type,
) bool {
	if decl == nil || decl.Body == nil || decl.Recv == nil || len(decl.Recv.List) != 1 || decl.Type == nil ||
		decl.Type.Params == nil || len(decl.Type.Params.List) != 2 ||
		len(decl.Recv.List[0].Names) != 1 || len(decl.Type.Params.List[0].Names) != 1 || len(decl.Type.Params.List[1].Names) != 1 {
		return false
	}
	receiverName := decl.Recv.List[0].Names[0].Name
	typeName := decl.Type.Params.List[0].Names[0].Name
	factoryName := decl.Type.Params.List[1].Names[0].Name
	for _, stmt := range decl.Body.List {
		assign, ok := stmt.(*ast.AssignStmt)
		if !ok || len(assign.Lhs) != 1 || len(assign.Rhs) != 1 || !servicePackageFieldSelector(assign.Lhs[0], receiverName, field) {
			continue
		}
		call, ok := assign.Rhs[0].(*ast.CallExpr)
		if !ok || exprIdentName(call.Fun) != "append" || len(call.Args) != 2 || !servicePackageFieldSelector(call.Args[0], receiverName, field) {
			continue
		}
		literal, ok := call.Args[1].(*ast.CompositeLit)
		if !ok || !isSDKFactoryEntry(pkg.TypesInfo.TypeOf(literal), factoryType) || len(literal.Elts) != 2 {
			continue
		}
		hasTypeName, hasFactory := false, false
		for _, element := range literal.Elts {
			kv, ok := element.(*ast.KeyValueExpr)
			if !ok {
				break
			}
			value, ok := kv.Value.(*ast.Ident)
			if !ok {
				break
			}
			switch exprIdentName(kv.Key) {
			case "TypeName":
				hasTypeName = value.Name == typeName
			case "Factory":
				hasFactory = value.Name == factoryName
			}
		}
		if hasTypeName && hasFactory {
			return true
		}
	}
	return false
}

func isSDKFactoryEntry(typ, factoryType types.Type) bool {
	if typ == nil {
		return false
	}
	entry, ok := typ.Underlying().(*types.Struct)
	return ok && entry.NumFields() == 2 &&
		entry.Field(0).Name() == "TypeName" && types.Identical(entry.Field(0).Type(), types.Typ[types.String]) &&
		entry.Field(1).Name() == "Factory" && types.Identical(entry.Field(1).Type(), factoryType)
}

func servicePackageFieldSelector(expr ast.Expr, receiver, field string) bool {
	selector, ok := expr.(*ast.SelectorExpr)
	if !ok || selector.Sel == nil || selector.Sel.Name != field {
		return false
	}
	ident, ok := selector.X.(*ast.Ident)
	return ok && ident.Name == receiver
}

func hasServicePackageRegistryHelper(helpers map[*types.Func]servicePackageRegistryHelper, kind, field string) bool {
	matches := 0
	for _, helper := range helpers {
		if helper.kind == kind && helper.field == field {
			matches++
		}
	}
	return matches == 1
}

func servicePackageRegistrationFunction(pkg *packages.Package, expr ast.Expr) (*types.Func, bool) {
	if pkg == nil || pkg.TypesInfo == nil {
		return nil, false
	}
	switch expr := expr.(type) {
	case *ast.Ident:
		function, ok := pkg.TypesInfo.ObjectOf(expr).(*types.Func)
		return function, ok
	case *ast.SelectorExpr:
		function, ok := pkg.TypesInfo.ObjectOf(expr.Sel).(*types.Func)
		return function, ok
	default:
		return nil, false
	}
}

func isServicePackageRegistrationCall(pkg *packages.Package, expr ast.Expr) bool {
	switch expr := expr.(type) {
	case *ast.Ident:
		return true
	case *ast.SelectorExpr:
		return isServicePackageSingleton(pkg, expr.X)
	default:
		return false
	}
}

func isServicePackageSingleton(pkg *packages.Package, expr ast.Expr) bool {
	ident, ok := expr.(*ast.Ident)
	if !ok || ident.Name != "_sp" || pkg == nil || pkg.Types == nil || pkg.TypesInfo == nil {
		return false
	}
	singleton, ok := pkg.TypesInfo.ObjectOf(ident).(*types.Var)
	if !ok || singleton.Parent() != pkg.Types.Scope() {
		return false
	}
	pointer, ok := singleton.Type().(*types.Pointer)
	if !ok || receiverTypeNameForType(pointer.Elem()) != "servicePackage" {
		return false
	}
	for _, file := range pkg.Syntax {
		for _, decl := range file.Decls {
			gen, ok := decl.(*ast.GenDecl)
			if !ok || gen.Tok != token.VAR {
				continue
			}
			for _, spec := range gen.Specs {
				valueSpec, ok := spec.(*ast.ValueSpec)
				if !ok {
					continue
				}
				for i, name := range valueSpec.Names {
					if i >= len(valueSpec.Values) || pkg.TypesInfo.ObjectOf(name) != singleton {
						continue
					}
					value, ok := valueSpec.Values[i].(*ast.UnaryExpr)
					if !ok || value.Op != token.AND {
						return false
					}
					literal, ok := value.X.(*ast.CompositeLit)
					if !ok {
						return false
					}
					return types.Identical(pkg.TypesInfo.TypeOf(literal), pointer.Elem())
				}
			}
		}
	}
	return false
}

func receiverTypeNameForType(typ types.Type) string {
	named, ok := typ.(*types.Named)
	if !ok || named.Obj() == nil {
		return ""
	}
	return named.Obj().Name()
}

func discoverServicePackageRegistrations(index *packageIndex, pkg *packages.Package, helpers map[*types.Func]servicePackageRegistryHelper) ([]entrypointSpec, error) {
	if pkg == nil || pkg.Types == nil || pkg.TypesInfo == nil {
		return nil, errors.New("service package registry has no type information")
	}
	paths := append([]string(nil), pkg.GoFiles...)
	slices.Sort(paths)

	specs := make([]entrypointSpec, 0)
	for _, path := range paths {
		file, err := fileForPath(pkg, path)
		if err != nil {
			return nil, err
		}
		for _, decl := range file.Decls {
			initDecl, ok := decl.(*ast.FuncDecl)
			if !ok || initDecl.Recv != nil || initDecl.Name == nil || initDecl.Name.Name != "init" || initDecl.Body == nil {
				continue
			}
			for _, statement := range initDecl.Body.List {
				exprStmt, ok := statement.(*ast.ExprStmt)
				if !ok {
					continue
				}
				call, ok := exprStmt.X.(*ast.CallExpr)
				if !ok {
					continue
				}
				helper, ok := servicePackageRegistrationFunction(pkg, call.Fun)
				if !ok {
					continue
				}
				registryHelper, ok := helpers[helper]
				if !ok {
					continue
				}
				if !isServicePackageRegistrationCall(pkg, call.Fun) {
					return nil, fmt.Errorf("%s registry call %s must use the service package singleton _sp", registryHelper.kind, helper.Name())
				}
				if len(call.Args) != registryHelper.argumentCount {
					if registryHelper.usesRegistrationName {
						return nil, fmt.Errorf("%s registry call %s has %d arguments, want %d", registryHelper.kind, helper.Name(), len(call.Args), registryHelper.argumentCount)
					}
					return nil, fmt.Errorf("%s registry call %s has %d factory arguments, want %d", registryHelper.kind, helper.Name(), len(call.Args), registryHelper.argumentCount)
				}
				var spec entrypointSpec
				var err error
				if registryHelper.usesRegistrationName {
					spec, err = registeredSDKEntrypointSpec(index, pkg, registryHelper, path, call.Args)
				} else {
					spec, err = registeredEntrypointSpec(index, pkg, registryHelper.kind, path, call.Args[0])
				}
				if err != nil {
					return nil, fmt.Errorf("%s registry call %s: %w", registryHelper.kind, helper.Name(), err)
				}
				specs = append(specs, spec)
			}
		}
	}
	return specs, nil
}

func registeredEntrypointSpec(index *packageIndex, pkg *packages.Package, kind, sourceFile string, expr ast.Expr) (entrypointSpec, error) {
	factory, factoryPkg, err := resolveFunctionObject(index, pkg, expr, nil)
	if err != nil {
		return entrypointSpec{}, err
	}
	if factory == nil || factoryPkg == nil || factory.Pkg() == nil {
		return entrypointSpec{}, errors.New("factory does not resolve to a package function")
	}
	typeName, err := registeredEntrypointTypeName(index, factoryPkg, factory)
	if err != nil {
		return entrypointSpec{}, err
	}
	return entrypointSpec{
		kind:        kind,
		typeName:    typeName,
		factory:     factory.Name(),
		sourceFile:  sourceFile,
		packagePath: factory.Pkg().Path(),
	}, nil
}

func registeredSDKEntrypointSpec(
	index *packageIndex,
	pkg *packages.Package,
	helper servicePackageRegistryHelper,
	sourceFile string,
	args []ast.Expr,
) (entrypointSpec, error) {
	if len(args) != 2 {
		return entrypointSpec{}, fmt.Errorf("has %d arguments, want 2", len(args))
	}
	typeName, err := stringLiteralValue(args[0])
	if err != nil {
		return entrypointSpec{}, fmt.Errorf("TypeName: %w", err)
	}
	if typeName == "" {
		return entrypointSpec{}, errors.New("TypeName is empty")
	}
	factory, factoryPkg, err := resolveFunctionObject(index, pkg, args[1], nil)
	if err != nil {
		return entrypointSpec{}, err
	}
	if factory == nil || factoryPkg == nil || factory.Pkg() == nil {
		return entrypointSpec{}, errors.New("factory does not resolve to a package function")
	}
	if !types.Identical(factory.Type(), helper.factoryType) {
		return entrypointSpec{}, fmt.Errorf("factory %s has type %s, want %s", factory.Name(), factory.Type(), helper.factoryType)
	}
	return entrypointSpec{
		kind:        helper.kind,
		typeName:    typeName,
		factory:     factory.Name(),
		sourceFile:  sourceFile,
		packagePath: factory.Pkg().Path(),
	}, nil
}

func registeredEntrypointTypeName(index *packageIndex, pkg *packages.Package, factory *types.Func) (string, error) {
	decl := index.funcDecls[factory]
	if decl == nil {
		return "", fmt.Errorf("factory declaration missing for %s", factory.Name())
	}
	concrete, err := resolveReturnedConcreteType(index, pkg, decl)
	if err != nil {
		return "", fmt.Errorf("resolve concrete type for %s: %w", factory.Name(), err)
	}
	metadata := types.NewMethodSet(concrete).Lookup(nil, "Metadata")
	if metadata == nil {
		return "", fmt.Errorf("concrete type for %s has no Metadata method", factory.Name())
	}
	metadataFunc, ok := metadata.Obj().(*types.Func)
	if !ok {
		return "", fmt.Errorf("Metadata for %s does not resolve to a function", factory.Name())
	}
	metadataDecl := index.funcDecls[metadataFunc]
	if metadataDecl == nil {
		return "", fmt.Errorf("Metadata declaration missing for %s", factory.Name())
	}
	typeName, err := metadataTypeName(metadataDecl)
	if err != nil {
		return "", fmt.Errorf("Metadata for %s: %w", factory.Name(), err)
	}
	return typeName, nil
}

func metadataTypeName(decl *ast.FuncDecl) (string, error) {
	if decl == nil || decl.Body == nil {
		return "", errors.New("has no body")
	}
	responseName, err := metadataResponseParameterName(decl)
	if err != nil {
		return "", err
	}
	var typeNames []string
	var visitErr error
	ast.Inspect(decl.Body, func(node ast.Node) bool {
		if visitErr != nil {
			return false
		}
		if _, ok := node.(*ast.FuncLit); ok {
			return false
		}
		assign, ok := node.(*ast.AssignStmt)
		if !ok {
			return true
		}
		if len(assign.Lhs) != 1 || len(assign.Rhs) != 1 {
			return true
		}
		selector, ok := assign.Lhs[0].(*ast.SelectorExpr)
		if !ok || selector.Sel == nil || selector.Sel.Name != "TypeName" {
			return true
		}
		receiver, ok := selector.X.(*ast.Ident)
		if !ok || receiver.Name != responseName {
			return true
		}
		typeName, err := stringLiteralValue(assign.Rhs[0])
		if err != nil {
			visitErr = fmt.Errorf("TypeName assignment: %w", err)
			return false
		}
		if typeName == "" {
			visitErr = errors.New("TypeName assignment is empty")
			return false
		}
		typeNames = append(typeNames, typeName)
		return true
	})
	if visitErr != nil {
		return "", visitErr
	}
	if len(typeNames) != 1 {
		return "", fmt.Errorf("has %d response TypeName assignments, want 1", len(typeNames))
	}
	return typeNames[0], nil
}

func metadataResponseParameterName(decl *ast.FuncDecl) (string, error) {
	if decl.Type == nil || decl.Type.Params == nil || len(decl.Type.Params.List) == 0 {
		return "", errors.New("has no response parameter")
	}
	response := decl.Type.Params.List[len(decl.Type.Params.List)-1]
	if len(response.Names) != 1 || response.Names[0] == nil || response.Names[0].Name == "_" {
		return "", errors.New("has no named response parameter")
	}
	return response.Names[0].Name, nil
}

func discoverProviderMapEntrypoints(index *packageIndex) ([]entrypointSpec, error) {
	paths := make([]string, 0)
	for path := range index.byFile {
		if filepath.Base(path) == "provider.go" {
			paths = append(paths, path)
		}
	}
	slices.Sort(paths)

	specs := make([]entrypointSpec, 0)
	for _, path := range paths {
		pkg := index.byFile[path]
		if pkg == nil || pkg.Types == nil {
			return nil, fmt.Errorf("package not loaded for %s", path)
		}
		file, err := fileForPath(pkg, path)
		if err != nil {
			return nil, err
		}
		var visitErr error
		ast.Inspect(file, func(node ast.Node) bool {
			if visitErr != nil {
				return false
			}
			field, ok := node.(*ast.KeyValueExpr)
			if !ok {
				return true
			}
			var kind string
			switch exprIdentName(field.Key) {
			case "ResourcesMap":
				kind = "resource"
			case "DataSourcesMap":
				kind = "data_source"
			default:
				return true
			}
			registry, ok := field.Value.(*ast.CompositeLit)
			if !ok {
				return true
			}
			for _, element := range registry.Elts {
				entry, ok := element.(*ast.KeyValueExpr)
				if !ok {
					continue
				}
				typeName, err := stringLiteralValue(entry.Key)
				if err != nil || !strings.HasPrefix(typeName, "aws_") {
					continue
				}
				call, ok := entry.Value.(*ast.CallExpr)
				if !ok {
					visitErr = fmt.Errorf("%s %s entrypoint is not a factory call", kind, typeName)
					return false
				}
				factory := calledFunctionObject(pkg, call)
				if factory == nil || factory.Pkg() == nil {
					visitErr = fmt.Errorf("%s %s factory does not resolve to a function", kind, typeName)
					return false
				}
				factoryPackagePath := factory.Pkg().Path()
				if _, err := packageForTypes(index, factory.Pkg()); err != nil {
					visitErr = fmt.Errorf("%s %s factory package %s is not loaded: %w", kind, typeName, factoryPackagePath, err)
					return false
				}
				specs = append(specs, entrypointSpec{
					kind:        kind,
					typeName:    typeName,
					factory:     factory.Name(),
					sourceFile:  path,
					packagePath: factoryPackagePath,
				})
			}
			return false
		})
		if visitErr != nil {
			return nil, fmt.Errorf("discover entrypoints in %s: %w", path, visitErr)
		}
	}
	return compactEntrypointSpecs(specs)
}

func compactEntrypointSpecs(specs []entrypointSpec) ([]entrypointSpec, error) {
	slices.SortFunc(specs, func(left, right entrypointSpec) int {
		if result := compareStrings(left.kind, right.kind); result != 0 {
			return result
		}
		if result := compareStrings(left.typeName, right.typeName); result != 0 {
			return result
		}
		if result := compareStrings(left.factory, right.factory); result != 0 {
			return result
		}
		if result := compareStrings(left.packagePath, right.packagePath); result != 0 {
			return result
		}
		return compareStrings(left.sourceFile, right.sourceFile)
	})
	for i := 1; i < len(specs); i++ {
		previous, current := specs[i-1], specs[i]
		if previous.kind == current.kind && previous.typeName == current.typeName &&
			(previous.factory != current.factory || previous.packagePath != current.packagePath) {
			return nil, fmt.Errorf(
				"conflicting %s entrypoints for %s: %s in %s and %s in %s",
				current.kind,
				current.typeName,
				previous.factory,
				previous.sourceFile,
				current.factory,
				current.sourceFile,
			)
		}
	}
	return slices.CompactFunc(specs, func(left, right entrypointSpec) bool {
		return left.kind == right.kind && left.typeName == right.typeName &&
			left.factory == right.factory && left.packagePath == right.packagePath
	}), nil
}

func resolveHandlers(index *packageIndex, spec entrypointSpec) ([]handlerMethod, error) {
	pkg, err := packageForPath(index, spec.packagePath)
	if err != nil {
		return nil, err
	}
	if pkg.Types == nil {
		return nil, fmt.Errorf("package %s has no type information", spec.packagePath)
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
			handlers, err := resolveSchemaResourceHandlers(index, pkg, decl, map[string]string{
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
			if err == nil {
				return handlers, nil
			}
			legacy, legacyErr := resolveLegacySchemaResourceHandlers(index, pkg, decl, []namedMethodAction{
				{"Create", "create"},
				{"Read", "read"},
				{"Update", "update"},
				{"Delete", "delete"},
			})
			if legacyErr != nil {
				return nil, fmt.Errorf("resolve SDK handlers: %w; resolve legacy handlers: %v", err, legacyErr)
			}
			return legacy, nil
		}
		return resolveConcreteTypeHandlers(index, pkg, decl, []namedMethodAction{{"Create", "create"}, {"Read", "read"}, {"Update", "update"}, {"Delete", "delete"}})
	case "data_source":
		if isSDKFactory(factoryObj) {
			handlers, err := resolveSchemaResourceHandlers(index, pkg, decl, map[string]string{
				"Read":               "read",
				"ReadContext":        "read",
				"ReadWithoutTimeout": "read",
			})
			if err == nil {
				return handlers, nil
			}
			legacy, legacyErr := resolveLegacySchemaResourceHandlers(index, pkg, decl, []namedMethodAction{{"Read", "read"}})
			if legacyErr != nil {
				return nil, fmt.Errorf("resolve SDK handlers: %w; resolve legacy handlers: %v", err, legacyErr)
			}
			return legacy, nil
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
		if !ok && strings.HasSuffix(fieldName, "WithoutTimeout") {
			action, ok = fieldActions[strings.TrimSuffix(fieldName, "WithoutTimeout")]
		}
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
	if isSchemaResourcePointer(concreteType) {
		return resolveLegacySchemaResourceHandlers(index, pkg, decl, actions)
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
		funcs, found, err := directSSAFunctionsForSelection(index, selection, action.method)
		if err != nil {
			return nil, fmt.Errorf("resolve %s handler: %w", action.action, err)
		}
		if !found {
			return nil, fmt.Errorf("%s handler does not resolve to a concrete method selection", action.method)
		}
		handlers = append(handlers, handlerMethod{action: action.action, funcs: funcs})
	}
	if len(handlers) == 0 {
		return nil, errors.New("resolved no framework handlers")
	}
	slices.SortFunc(handlers, compareHandlerMethods)
	return handlers, nil
}

func resolveLegacySchemaResourceHandlers(index *packageIndex, pkg *packages.Package, decl *ast.FuncDecl, actions []namedMethodAction) ([]handlerMethod, error) {
	handlers := make([]handlerMethod, 0, len(actions))
	for _, action := range actions {
		funcs, err := collectLegacyHandlerFunctions(index, pkg, decl, action.method, map[*types.Func]struct{}{})
		if err != nil {
			return nil, fmt.Errorf("resolve %s handler: %w", action.action, err)
		}
		if len(funcs) == 0 {
			continue
		}
		handlers = append(handlers, handlerMethod{action: action.action, funcs: funcs})
	}
	if len(handlers) == 0 {
		return nil, errors.New("resolved no legacy resource handlers")
	}
	slices.SortFunc(handlers, compareHandlerMethods)
	return handlers, nil
}

func collectLegacyHandlerFunctions(index *packageIndex, pkg *packages.Package, decl *ast.FuncDecl, method string, seen map[*types.Func]struct{}) ([]*ssa.Function, error) {
	if decl == nil || decl.Body == nil {
		return nil, errors.New("factory has no body")
	}
	locals := collectLocalExprs(decl.Body)
	funcs := make([]*ssa.Function, 0)
	for _, stmt := range decl.Body.List {
		switch stmt := stmt.(type) {
		case *ast.AssignStmt:
			for i, lhs := range stmt.Lhs {
				selector, ok := lhs.(*ast.SelectorExpr)
				if !ok || !legacyHandlerFieldMatches(selector.Sel.Name, method) || i >= len(stmt.Rhs) {
					continue
				}
				resolved, err := resolveFunctionExpr(index, pkg, stmt.Rhs[i], locals)
				if err != nil {
					return nil, err
				}
				for _, fn := range resolved {
					funcs = append(funcs, fn)
				}
			}
		case *ast.ReturnStmt:
			if len(stmt.Results) == 0 {
				continue
			}
			expr := resolveExpr(stmt.Results[0], locals)
			if lit, ok := expr.(*ast.UnaryExpr); ok && lit.Op == token.AND {
				expr = lit.X
			}
			if literal, ok := expr.(*ast.CompositeLit); ok {
				for _, element := range literal.Elts {
					field, ok := element.(*ast.KeyValueExpr)
					if !ok || !legacyHandlerFieldMatches(exprIdentName(field.Key), method) || exprIdentName(field.Value) == "nil" {
						continue
					}
					resolved, err := resolveFunctionExpr(index, pkg, field.Value, locals)
					if err != nil {
						return nil, err
					}
					funcs = append(funcs, resolved...)
				}
				continue
			}
			call, ok := expr.(*ast.CallExpr)
			if !ok {
				continue
			}
			callee, calleePkg, err := resolveFunctionObject(index, pkg, call.Fun, locals)
			if err != nil {
				return nil, err
			}
			if callee == nil {
				return nil, errors.New("returned factory call does not resolve to a function")
			}
			if _, ok := seen[callee]; ok {
				continue
			}
			seen[callee] = struct{}{}
			if calleePkg == nil {
				if callee.Pkg() == nil {
					return nil, fmt.Errorf("package missing for %s", callee.Name())
				}
				calleePkg, err = packageForTypes(index, callee.Pkg())
				if err != nil {
					return nil, fmt.Errorf("package not loaded for %s: %w", callee.Name(), err)
				}
			}
			calleeDecl := index.funcDecls[callee]
			if calleeDecl == nil {
				return nil, fmt.Errorf("factory declaration missing for %s", callee.Name())
			}
			if handlers, err := resolveSchemaResourceHandlers(index, calleePkg, calleeDecl, map[string]string{method: method}); err == nil {
				for _, handler := range handlers {
					funcs = append(funcs, handler.funcs...)
				}
				continue
			}
			inherited, err := collectLegacyHandlerFunctions(index, calleePkg, calleeDecl, method, seen)
			if err != nil {
				return nil, err
			}
			funcs = append(funcs, inherited...)
		}
	}
	return slices.Compact(funcs), nil
}

func legacyHandlerFieldMatches(fieldName, method string) bool {
	return fieldName == method || fieldName == method+"WithoutTimeout"
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
		if !isSchemaResourceType(pkg.TypesInfo.TypeOf(lit)) && !isSchemaResourcePointer(pkg.TypesInfo.TypeOf(lit)) {
			return nil, false, nil
		}
		return lit, true, nil
	case *ast.CompositeLit:
		if !isSchemaResourceType(pkg.TypesInfo.TypeOf(expr)) && !isSchemaResourcePointer(pkg.TypesInfo.TypeOf(expr)) {
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
	if decl.Body == nil {
		return nil, fmt.Errorf("function declaration missing a body for %s", calleeObj.Name())
	}
	if calleePkg == nil {
		if calleeObj.Pkg() == nil {
			return nil, fmt.Errorf("package missing for %s", calleeObj.Name())
		}
		calleePkg, err = packageForTypes(index, calleeObj.Pkg())
		if err != nil {
			return nil, fmt.Errorf("package not loaded for %s: %w", calleeObj.Name(), err)
		}
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

func collectAPIMethods(index *packageIndex, roots []*ssa.Function, sdkMappings map[sdkMethodKey][]apiMethod) (handlerAPIMethods, error) {
	return newAPIMethodAnalyzer(index, sdkMappings).collect(roots)
}

func (analyzer *apiMethodAnalyzer) collect(roots []*ssa.Function) (handlerAPIMethods, error) {
	if len(roots) == 0 {
		return handlerAPIMethods{}, errors.New("handler resolved no SSA functions")
	}
	pending := make(map[*ssa.Function]*apiMethodFunctionSummary)
	expandedRoots := expandRootFunctions(analyzer.index, roots)
	for _, root := range expandedRoots {
		if err := analyzer.discover(root, pending); err != nil {
			analyzer.discardInProgress(pending)
			return handlerAPIMethods{}, err
		}
	}
	analyzer.complete(pending)

	methods := make([]apiMethod, 0)
	for _, root := range expandedRoots {
		methods = append(methods, analyzer.summaries[root].methods...)
	}
	slices.SortFunc(methods, compareAPIMethods)
	methods = slices.Compact(methods)
	return handlerAPIMethods{methods: methods}, nil
}

func (analyzer *apiMethodAnalyzer) discover(fn *ssa.Function, pending map[*ssa.Function]*apiMethodFunctionSummary) error {
	if fn == nil {
		return errors.New("handler contains a missing SSA function")
	}
	if summary := analyzer.summaries[fn]; summary != nil {
		return nil
	}
	summary := &apiMethodFunctionSummary{state: apiMethodSummaryInProgress}
	analyzer.summaries[fn] = summary
	pending[fn] = summary

	if obj, ok := fn.Object().(*types.Func); ok {
		resolved, err := apiMethodsForSDKCallable(obj, analyzer.sdkMappings, isProviderSSAFunction(fn))
		if err != nil {
			return err
		}
		summary.direct = append(summary.direct, resolved...)
	}
	direct, err := collectDirectSDKAPIMethods(analyzer.index, fn, analyzer.sdkMappings)
	if err != nil {
		return err
	}
	summary.direct = append(summary.direct, direct...)

	// The static call graph omits some calls inside range-over-function iterators.
	// Preserve those provider-local edges from the type-checked AST.
	localCallees, err := collectDirectLocalCallees(analyzer.index, fn)
	if err != nil {
		return err
	}
	summary.callees = append(summary.callees, localCallees...)
	summary.callees = append(summary.callees, analyzer.index.nestedFuncs[fn]...)
	summary.callees = append(summary.callees, analyzer.index.callers[fn]...)
	for _, callee := range summary.callees {
		if err := analyzer.discover(callee, pending); err != nil {
			return err
		}
	}
	return nil
}

func (analyzer *apiMethodAnalyzer) complete(pending map[*ssa.Function]*apiMethodFunctionSummary) {
	for {
		changed := false
		for _, summary := range pending {
			methods := append([]apiMethod(nil), summary.direct...)
			for _, callee := range summary.callees {
				methods = append(methods, analyzer.summaries[callee].methods...)
			}
			slices.SortFunc(methods, compareAPIMethods)
			methods = slices.Compact(methods)
			if !slices.Equal(summary.methods, methods) {
				summary.methods = methods
				changed = true
			}
		}
		if !changed {
			break
		}
	}
	for _, summary := range pending {
		summary.state = apiMethodSummaryComplete
	}
}

func (analyzer *apiMethodAnalyzer) discardInProgress(pending map[*ssa.Function]*apiMethodFunctionSummary) {
	for fn, summary := range pending {
		if summary.state == apiMethodSummaryInProgress {
			delete(analyzer.summaries, fn)
		}
	}
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

func collectDirectLocalCallees(index *packageIndex, fn *ssa.Function) ([]*ssa.Function, error) {
	if fn == nil || fn.Syntax() == nil {
		return nil, nil
	}
	ssaPkg := fn.Package()
	if ssaPkg == nil || ssaPkg.Pkg == nil || !isProviderPackage(ssaPkg.Pkg.Path()) {
		return nil, nil
	}
	sourcePkg, err := packageForTypes(index, ssaPkg.Pkg)
	if err != nil {
		return nil, fmt.Errorf("source package %s is not indexed: %w", ssaPkg.Pkg.Path(), err)
	}

	var callees []*ssa.Function
	var visitErr error
	ast.Inspect(fn.Syntax(), func(node ast.Node) bool {
		if visitErr != nil {
			return false
		}
		callExpr, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		obj := calledFunctionObject(sourcePkg, callExpr)
		if obj == nil || obj.Pkg() == nil || !isProviderPackage(obj.Pkg().Path()) {
			return true
		}
		if selection, ok := callExpr.Fun.(*ast.SelectorExpr); ok {
			if methodSelection := sourcePkg.TypesInfo.Selections[selection]; methodSelection != nil {
				if isBodylessInterfaceMethod(obj) {
					resolved, relevant, err := providerLocalInterfaceCallees(index, methodSelection, obj.FullName())
					if err != nil {
						visitErr = err
						return false
					}
					if relevant {
						callees = append(callees, resolved...)
					}
					return true
				}
				resolved, found, err := directSSAFunctionsForSelection(index, methodSelection, obj.FullName())
				if err != nil {
					visitErr = err
					return false
				}
				if found {
					callees = append(callees, resolved...)
					return true
				}
			}
		}
		if isBodylessInterfaceMethod(obj) {
			// An interface method has no implementation to inspect. Its dynamic
			// dispatch cannot itself be a direct SDK call.
			return true
		}
		if isBodylessLinknameFunction(index, obj) {
			// A go:linkname declaration is implemented outside the provider package.
			// It has no local body to inspect and cannot contain a direct SDK call.
			return true
		}
		resolved, err := ssaFunctionsForObject(index, obj, obj.FullName())
		if err != nil {
			visitErr = err
			return false
		}
		callees = append(callees, resolved...)
		return true
	})
	if visitErr != nil {
		return nil, visitErr
	}
	return callees, nil
}

func providerLocalInterfaceCallees(index *packageIndex, interfaceSelection *types.Selection, name string) ([]*ssa.Function, bool, error) {
	if index == nil || interfaceSelection == nil || interfaceSelection.Kind() != types.MethodVal {
		return nil, false, nil
	}
	method, ok := interfaceSelection.Obj().(*types.Func)
	if !ok {
		return nil, false, fmt.Errorf("unresolved provider interface dispatch for %s: selected object is not a function", name)
	}
	if isFrameworkConversionMethod(method) || isKnownPureProviderInterfaceMethod(interfaceSelection) {
		return nil, false, nil
	}
	identity, ok := canonicalFunctionIdentity(method)
	if !ok {
		return collectProviderLocalInterfaceCallees(index, interfaceSelection, name)
	}
	if index.interfaceCalleeCache == nil {
		index.interfaceCalleeCache = make(map[functionIdentity]providerInterfaceCalleeResolution)
	}
	if cached, ok := index.interfaceCalleeCache[identity]; ok {
		return cached.callees, cached.relevant, cached.err
	}
	callees, relevant, err := collectProviderLocalInterfaceCallees(index, interfaceSelection, name)
	index.interfaceCalleeCache[identity] = providerInterfaceCalleeResolution{
		callees:  callees,
		relevant: relevant,
		err:      err,
	}
	return callees, relevant, err
}

func collectProviderLocalInterfaceCallees(index *packageIndex, interfaceSelection *types.Selection, name string) ([]*ssa.Function, bool, error) {
	if index == nil || interfaceSelection == nil || interfaceSelection.Kind() != types.MethodVal {
		return nil, false, nil
	}
	interfaceType, ok := interfaceSelection.Recv().Underlying().(*types.Interface)
	if !ok {
		return nil, false, nil
	}
	interfaceType.Complete()
	method, ok := interfaceSelection.Obj().(*types.Func)
	if !ok {
		return nil, false, fmt.Errorf("unresolved provider interface dispatch for %s: selected object is not a function", name)
	}

	callees := make([]*ssa.Function, 0, 1)
	seen := make(map[ssaFunctionSourceIdentity]struct{})
	for typePkg := range index.byTypes {
		if typePkg == nil || !isProviderPackage(typePkg.Path()) {
			continue
		}
		scope := typePkg.Scope()
		for _, typeName := range scope.Names() {
			typeObject, ok := scope.Lookup(typeName).(*types.TypeName)
			if !ok {
				continue
			}
			named, ok := types.Unalias(typeObject.Type()).(*types.Named)
			if !ok || isInterfaceType(named) {
				continue
			}
			for _, receiver := range []types.Type{named, types.NewPointer(named)} {
				if !types.Implements(receiver, interfaceType) {
					continue
				}
				implementation := types.NewMethodSet(receiver).Lookup(method.Pkg(), method.Name())
				if implementation == nil {
					continue
				}
				concrete, ok := implementation.Obj().(*types.Func)
				if !ok || concrete.Pkg() == nil || !isProviderPackage(concrete.Pkg().Path()) {
					continue
				}
				if isBodylessLinknameFunction(index, concrete) {
					continue
				}
				resolved, found, err := directSSAFunctionsForSelection(index, implementation, concrete.FullName())
				if err != nil {
					return nil, true, fmt.Errorf("unresolved provider interface dispatch for %s: %w", name, err)
				}
				if !found || len(resolved) == 0 {
					return nil, true, fmt.Errorf("unresolved provider interface dispatch for %s: SSA function missing for %s", name, concrete.FullName())
				}
				for _, fn := range resolved {
					declaration, ok := fn.Syntax().(*ast.FuncDecl)
					if !ok || declaration.Body == nil {
						return nil, true, fmt.Errorf("unresolved provider interface dispatch for %s: SSA function missing for %s", name, concrete.FullName())
					}
					origin := fn.Origin()
					if origin == nil {
						origin = fn
					}
					obj, ok := origin.Object().(*types.Func)
					if !ok {
						return nil, true, fmt.Errorf("unresolved provider interface dispatch for %s: SSA function missing for %s", name, concrete.FullName())
					}
					identity, ok := canonicalFunctionIdentity(obj)
					if !ok {
						return nil, true, fmt.Errorf("unresolved provider interface dispatch for %s: SSA function missing for %s", name, concrete.FullName())
					}
					source := ssaFunctionSourceIdentityFor(fn, declaration, identity)
					if _, ok := seen[source]; ok {
						continue
					}
					seen[source] = struct{}{}
					callees = append(callees, fn)
				}
			}
		}
	}
	if len(callees) == 0 {
		return nil, false, nil
	}
	if len(callees) != 1 {
		return nil, true, fmt.Errorf(
			"ambiguous provider interface dispatch for %s: %d concrete implementations",
			name,
			len(callees),
		)
	}
	return callees, true, nil
}

func isFrameworkConversionMethod(method *types.Func) bool {
	if method == nil || method.Pkg() == nil {
		return false
	}
	if method.Pkg().Path() != providerModulePath+"/internal/framework/flex" {
		return false
	}
	switch method.Name() {
	case "Elements", "Expand", "ExpandTo", "Flatten":
		return true
	default:
		return false
	}
}

func isKnownPureProviderInterfaceMethod(selection *types.Selection) bool {
	if selection == nil || selection.Kind() != types.MethodVal {
		return false
	}
	method, ok := selection.Obj().(*types.Func)
	if !ok || method == nil || method.Pkg() == nil || method.Name() != "SubFrom" {
		return false
	}
	if method.Pkg().Path() != providerModulePath+"/internal/service/acm" {
		return false
	}
	receiver, ok := types.Unalias(selection.Recv()).(*types.Named)
	return ok && receiver.Obj() != nil && receiver.Obj().Name() == "hybridDurationValue"
}

func isProviderPackage(path string) bool {
	if path == providerModulePath {
		return true
	}
	relative, ok := strings.CutPrefix(path, providerModulePath+"/")
	return ok && relative != "vendor" && !strings.HasPrefix(relative, "vendor/")
}

func isBodylessLinknameFunction(index *packageIndex, obj *types.Func) bool {
	if index == nil || obj == nil {
		return false
	}
	declarations := make([]*ast.FuncDecl, 0, 1)
	if declaration := index.funcDecls[obj]; declaration != nil {
		declarations = append(declarations, declaration)
	}
	if identity, ok := canonicalFunctionIdentity(obj); ok {
		declarations = append(declarations, index.funcDeclsByIdentity[identity]...)
	}
	seen := make(map[*ast.FuncDecl]bool, len(declarations))
	hasLinkname := false
	for _, declaration := range declarations {
		if declaration == nil || seen[declaration] {
			continue
		}
		seen[declaration] = true
		if declaration.Body != nil {
			return false
		}
		if declaration.Doc == nil {
			continue
		}
		for _, comment := range declaration.Doc.List {
			fields := strings.Fields(comment.Text)
			if len(fields) == 3 && fields[0] == "//go:linkname" && fields[1] == obj.Name() {
				hasLinkname = true
			}
		}
	}
	return hasLinkname
}

func isProviderSSAFunction(fn *ssa.Function) bool {
	if fn == nil || fn.Package() == nil || fn.Package().Pkg == nil {
		return false
	}
	return isProviderPackage(fn.Package().Pkg.Path())
}

func collectDirectSDKAPIMethods(index *packageIndex, fn *ssa.Function, sdkMappings map[sdkMethodKey][]apiMethod) ([]apiMethod, error) {
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
			obj, ok := callee.Object().(*types.Func)
			if !ok {
				continue
			}
			resolved, err := apiMethodsForSDKCallable(obj, sdkMappings, isProviderSSAFunction(fn))
			if err != nil {
				return nil, err
			}
			methods = append(methods, resolved...)
		}
	}
	astMethods, err := collectDirectASTSDKAPIMethods(index, fn, sdkMappings)
	if err != nil {
		return nil, err
	}
	return append(methods, astMethods...), nil
}

func collectDirectASTSDKAPIMethods(index *packageIndex, fn *ssa.Function, sdkMappings map[sdkMethodKey][]apiMethod) ([]apiMethod, error) {
	if fn == nil || fn.Syntax() == nil {
		return nil, nil
	}
	ssaPkg := fn.Package()
	if ssaPkg == nil || ssaPkg.Pkg == nil {
		return nil, nil
	}
	sourcePkg, err := packageForTypes(index, ssaPkg.Pkg)
	if err != nil {
		return nil, fmt.Errorf("source package %s is not indexed: %w", ssaPkg.Pkg.Path(), err)
	}
	methods := make([]apiMethod, 0)
	var visitErr error
	ast.Inspect(fn.Syntax(), func(node ast.Node) bool {
		if visitErr != nil {
			return false
		}
		callExpr, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		obj := calledFunctionObject(sourcePkg, callExpr)
		if obj == nil {
			return true
		}
		resolved, err := apiMethodsForSDKCallable(obj, sdkMappings, isProviderSSAFunction(fn))
		if err != nil {
			visitErr = err
			return false
		}
		methods = append(methods, resolved...)
		return true
	})
	if visitErr != nil {
		return nil, visitErr
	}
	return methods, nil
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
func isBodylessInterfaceMethod(obj *types.Func) bool {
	if obj == nil {
		return false
	}
	signature, ok := obj.Type().(*types.Signature)
	return ok && signature.Recv() != nil && isInterfaceType(signature.Recv().Type())
}

func apiMethodsForSDKCallable(obj *types.Func, sdkMappings map[sdkMethodKey][]apiMethod, strictV2 bool) ([]apiMethod, error) {
	key, ok := sdkKeyForObject(obj)
	if !ok {
		return nil, nil
	}
	if isAWSV2ServicePackage(key.pkg) {
		if !awsSDKGoV2Operation(obj) {
			return nil, nil
		}
		if methods, ok := sdkMappings[key]; ok {
			return methods, nil
		}
		if strictV2 {
			return nil, fmt.Errorf("aws-sdk-go-v2 operation %s %s.%s is absent from the exact mapping", key.pkg, key.receiver, key.method)
		}
		return nil, nil
	}
	if methods, ok := sdkMappings[key]; ok {
		return methods, nil
	}
	if operation, ok := awsSDKGoV1Operation(obj); ok {
		return []apiMethod{operation}, nil
	}
	return nil, nil
}

func isAWSV2ServicePackage(path string) bool {
	const prefix = "github.com/aws/aws-sdk-go-v2/service/"
	service, ok := strings.CutPrefix(path, prefix)
	return ok && service != "" && !strings.ContainsRune(service, '/')
}

func awsSDKGoV2Operation(obj *types.Func) bool {
	if obj == nil || obj.Pkg() == nil || !obj.Exported() || !isAWSV2ServicePackage(obj.Pkg().Path()) {
		return false
	}
	signature, ok := obj.Type().(*types.Signature)
	if !ok {
		return false
	}
	if awsSDKGoV2ClientReceiver(signature, obj.Pkg()) {
		return awsSDKGoV2ClientOperationSignature(signature, obj.Pkg(), obj.Name())
	}
	return awsSDKGoV2PaginatorOperationSignature(signature, obj.Pkg(), obj.Name())
}
func isAWSV1ServicePackage(path string) bool {
	const prefix = "github.com/aws/aws-sdk-go/service/"
	service, ok := strings.CutPrefix(path, prefix)
	return ok && service != "" && !strings.ContainsRune(service, '/')
}

func awsSDKGoV1Operation(obj *types.Func) (apiMethod, bool) {
	if obj == nil || obj.Pkg() == nil || !obj.Exported() || !isAWSV1ServicePackage(obj.Pkg().Path()) {
		return apiMethod{}, false
	}
	if isForbiddenSDKOperationName(obj.Name()) {
		return apiMethod{}, false
	}
	operation, ok := awsSDKGoV1OperationName(obj.Name())
	if !ok {
		return apiMethod{}, false
	}
	signature, ok := obj.Type().(*types.Signature)
	if !ok || signature.Recv() == nil {
		return apiMethod{}, false
	}
	receiver, ok := types.Unalias(signature.Recv().Type()).(*types.Pointer)
	if !ok {
		return apiMethod{}, false
	}
	named, ok := types.Unalias(receiver.Elem()).(*types.Named)
	if !ok || named.Obj() == nil || named.Obj().Pkg() != obj.Pkg() {
		return apiMethod{}, false
	}
	if types.NewMethodSet(types.NewPointer(named)).Lookup(nil, operation+"Request") == nil {
		return apiMethod{}, false
	}
	service, _ := strings.CutPrefix(obj.Pkg().Path(), "github.com/aws/aws-sdk-go/service/")
	return apiMethod{Service: service, Name: operation}, true
}

func awsSDKGoV1OperationName(method string) (string, bool) {
	for _, prefix := range []string{"WaitUntil", "Presign"} {
		if strings.HasPrefix(method, prefix) {
			return "", false
		}
	}
	for _, suffix := range []string{"PagesWithContext", "WithContext", "Pages"} {
		if operation, ok := strings.CutSuffix(method, suffix); ok {
			return operation, operation != ""
		}
	}
	return method, method != ""
}

func isForbiddenSDKOperationName(name string) bool {
	return name == "String" ||
		name == "Validate" ||
		name == "New" ||
		name == "newClient" ||
		name == "NormalizeBucketLocation" ||
		strings.HasSuffix(name, "_Values")
}

func awsSDKGoV2ClientReceiver(signature *types.Signature, pkg *types.Package) bool {
	if signature == nil || signature.Recv() == nil {
		return false
	}
	receiver, ok := signature.Recv().Type().(*types.Pointer)
	if !ok {
		return false
	}
	named, ok := receiver.Elem().(*types.Named)
	return ok && named.Obj() != nil && named.Obj().Pkg() == pkg && named.Obj().Name() == "Client" &&
		isStructType(named)
}

func awsSDKGoV2ClientOperationSignature(signature *types.Signature, pkg *types.Package, operation string) bool {
	if signature.Params().Len() != 3 || !signature.Variadic() ||
		!isContextType(signature.Params().At(0).Type()) ||
		!isNamedPointer(signature.Params().At(1).Type(), pkg, operation+"Input") ||
		!isOptionsVariadic(signature.Params().At(2).Type(), pkg) {
		return false
	}
	return isOperationResults(signature.Results(), pkg, operation+"Output")
}

func awsSDKGoV2PaginatorOperationSignature(signature *types.Signature, pkg *types.Package, method string) bool {
	if method != "NextPage" {
		return false
	}
	if signature.Recv() == nil || signature.Params().Len() != 2 || !signature.Variadic() ||
		!isContextType(signature.Params().At(0).Type()) || !isOptionsVariadic(signature.Params().At(1).Type(), pkg) {
		return false
	}
	receiver, ok := signature.Recv().Type().(*types.Pointer)
	if !ok {
		return false
	}
	named, ok := receiver.Elem().(*types.Named)
	if !ok || named.Obj() == nil || named.Obj().Pkg() != pkg || !isStructType(named) {
		return false
	}
	operation, ok := strings.CutSuffix(named.Obj().Name(), "Paginator")
	if !ok || operation == "" {
		return false
	}
	return isOperationResults(signature.Results(), pkg, operation+"Output")
}

func isContextType(typ types.Type) bool {
	named, ok := typ.(*types.Named)
	return ok && named.Obj() != nil && named.Obj().Pkg() != nil &&
		named.Obj().Pkg().Path() == "context" && named.Obj().Name() == "Context"
}

func isNamedPointer(typ types.Type, pkg *types.Package, name string) bool {
	pointer, ok := typ.(*types.Pointer)
	if !ok {
		return false
	}
	named, ok := pointer.Elem().(*types.Named)
	return ok && named.Obj() != nil && named.Obj().Pkg() == pkg && named.Obj().Name() == name
}

func isOptionsVariadic(typ types.Type, pkg *types.Package) bool {
	slice, ok := typ.(*types.Slice)
	if !ok {
		return false
	}
	option, ok := slice.Elem().Underlying().(*types.Signature)
	return ok && option.Recv() == nil && option.Params().Len() == 1 && option.Results().Len() == 0 &&
		isNamedPointer(option.Params().At(0).Type(), pkg, "Options")
}

func isOperationResults(results *types.Tuple, pkg *types.Package, output string) bool {
	return results.Len() == 2 && isNamedPointer(results.At(0).Type(), pkg, output) &&
		types.Identical(results.At(1).Type(), types.Universe.Lookup("error").Type())
}

func isStructType(named *types.Named) bool {
	_, ok := named.Underlying().(*types.Struct)
	return ok
}

func sdkKeyForObject(obj *types.Func) (sdkMethodKey, bool) {
	if obj == nil || obj.Pkg() == nil {
		return sdkMethodKey{}, false
	}
	signature, ok := obj.Type().(*types.Signature)
	if !ok {
		return sdkMethodKey{}, false
	}
	return sdkKeyForCallable(obj.Pkg().Path(), receiverName(signature), obj.Name())
}

func sdkKeyForFunction(fn *ssa.Function) (sdkMethodKey, bool) {
	if fn == nil {
		return sdkMethodKey{}, false
	}
	if obj, ok := fn.Object().(*types.Func); ok {
		return sdkKeyForObject(obj)
	}
	pkg := fn.Package()
	if pkg == nil || pkg.Pkg == nil {
		return sdkMethodKey{}, false
	}
	return sdkKeyForCallable(pkg.Pkg.Path(), receiverName(fn.Signature), fn.Name())
}

func sdkKeyForCallable(pkg, receiver, method string) (sdkMethodKey, bool) {
	if pkg == "" || method == "" {
		return sdkMethodKey{}, false
	}
	return sdkMethodKey{pkg: pkg, receiver: receiver, method: method}, true
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

func extractSpecsFromServiceMethod(index *packageIndex, pkg *packages.Package, decl *ast.FuncDecl, kind, sourceFile string) ([]entrypointSpec, error) {
	if decl.Body == nil {
		return nil, fmt.Errorf("service package method %s has no body", decl.Name.Name)
	}
	locals := collectLocalExprs(decl.Body)
	specs := make([]entrypointSpec, 0)
	hasEmptySDKFactoryMapReturn := false
	hasSDKFactoryCollectionReturn := false
	hasFrameworkFactoryCollectionReturn := false
	hasFactoryCollectionReturn := false
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		resolved := resolveExpr(returnStmt.Results[0], locals)
		if mapSpecs, ok, err := entrypointSpecsFromMapLit(pkg, kind, sourceFile, resolved); ok {
			if err != nil {
				return nil, err
			}
			specs = append(specs, mapSpecs...)
			if len(mapSpecs) == 0 && servicePackageEmptySDKFactoryMapReturn(pkg, decl, kind, resolved) {
				hasEmptySDKFactoryMapReturn = true
			}
			continue
		}
		if frameworkSpecs, ok, err := frameworkFactoryCollectionSpecs(index, pkg, decl, kind, sourceFile, resolved); ok {
			if err != nil {
				return nil, err
			}
			specs = append(specs, frameworkSpecs...)
			hasFrameworkFactoryCollectionReturn = true
			continue
		}
		if sdkSpecs, ok, err := sdkFactoryCollectionSpecs(index, pkg, decl, kind, sourceFile, resolved); ok {
			if err != nil {
				return nil, err
			}
			specs = append(specs, sdkSpecs...)
			hasSDKFactoryCollectionReturn = true
			continue
		}
		items, ok, err := unwrapSpecList(pkg, resolved, locals)
		if err != nil {
			return nil, err
		}
		if !ok {
			if _, ok := servicePackageFactoryCollectionReturn(pkg, decl, kind, resolved); ok {
				hasFactoryCollectionReturn = true
				continue
			}
			return nil, fmt.Errorf("service package method %s returned an unresolvable entrypoint specification", decl.Name.Name)
		}
		for _, item := range items {
			spec, err := entrypointFromCompositeLit(pkg, kind, sourceFile, item)
			if err != nil {
				return nil, err
			}
			specs = append(specs, spec)
		}
	}
	if len(specs) == 0 && !hasFactoryCollectionReturn && !hasFrameworkFactoryCollectionReturn &&
		!hasSDKFactoryCollectionReturn && !hasEmptySDKFactoryMapReturn {
		return nil, fmt.Errorf("service package method %s yielded no entrypoints", decl.Name.Name)
	}
	return specs, nil
}

func serviceMethodFactoryCollectionField(pkg *packages.Package, decl *ast.FuncDecl, kind string) (string, bool) {
	if decl == nil || decl.Body == nil {
		return "", false
	}
	locals := collectLocalExprs(decl.Body)
	for _, stmt := range decl.Body.List {
		returnStmt, ok := stmt.(*ast.ReturnStmt)
		if !ok || len(returnStmt.Results) == 0 {
			continue
		}
		if field, ok := servicePackageFactoryCollectionReturn(pkg, decl, kind, resolveExpr(returnStmt.Results[0], locals)); ok {
			return field, true
		}
	}
	return "", false
}

func servicePackageFactoryCollectionField(method, kind string) (string, bool) {
	switch {
	case method == "FrameworkDataSources" && kind == "data_source":
		return "frameworkDataSourceFactories", true
	case method == "FrameworkResources" && kind == "resource":
		return "frameworkResourceFactories", true
	case method == "SDKDataSources" && kind == "data_source":
		return "sdkDataSourceFactories", true
	case method == "SDKResources" && kind == "resource":
		return "sdkResourceFactories", true
	default:
		return "", false
	}
}

func metadataFactoryCollectionMethod(method, kind string) bool {
	if method == "EphemeralResources" && kind == "ephemeral_resource" {
		return true
	}
	_, ok := servicePackageFactoryCollectionField(method, kind)
	return ok && strings.HasPrefix(method, "Framework")
}

func servicePackageFactoryCollectionReturn(pkg *packages.Package, decl *ast.FuncDecl, kind string, expr ast.Expr) (string, bool) {
	if decl == nil || decl.Name == nil {
		return "", false
	}
	expectedField, ok := servicePackageFactoryCollectionField(decl.Name.Name, kind)
	if !ok {
		return "", false
	}
	selector, ok := expr.(*ast.SelectorExpr)
	if !ok || selector.Sel == nil || decl == nil || decl.Recv == nil || len(decl.Recv.List) != 1 {
		return "", false
	}
	receiver, ok := selector.X.(*ast.Ident)
	if !ok || len(decl.Recv.List[0].Names) != 1 || receiver.Name != decl.Recv.List[0].Names[0].Name {
		return "", false
	}
	if pkg == nil || pkg.TypesInfo == nil {
		return "", false
	}
	field, ok := pkg.TypesInfo.ObjectOf(selector.Sel).(*types.Var)
	if !ok || !field.IsField() || field.Name() != expectedField {
		return "", false
	}
	method, ok := pkg.TypesInfo.ObjectOf(decl.Name).(*types.Func)
	if !ok {
		return "", false
	}
	signature, ok := method.Type().(*types.Signature)
	if !ok || signature.Results().Len() != 1 || !types.Identical(field.Type(), signature.Results().At(0).Type()) {
		return "", false
	}
	entries, ok := signature.Results().At(0).Type().Underlying().(*types.Slice)
	if !ok {
		return "", false
	}
	if strings.HasPrefix(decl.Name.Name, "Framework") {
		if _, ok := entries.Elem().Underlying().(*types.Signature); !ok {
			return "", false
		}
	} else if !isSDKFactoryCollectionEntry(entries.Elem()) {
		return "", false
	}
	return expectedField, true
}

func frameworkFactoryCollectionSpecs(
	index *packageIndex,
	pkg *packages.Package,
	decl *ast.FuncDecl,
	kind, sourceFile string,
	expr ast.Expr,
) ([]entrypointSpec, bool, error) {
	if decl == nil || decl.Name == nil || !metadataFactoryCollectionMethod(decl.Name.Name, kind) {
		return nil, false, nil
	}
	lit, ok := expr.(*ast.CompositeLit)
	if !ok || pkg == nil || pkg.TypesInfo == nil {
		return nil, false, nil
	}
	method, ok := pkg.TypesInfo.ObjectOf(decl.Name).(*types.Func)
	if !ok {
		return nil, false, nil
	}
	signature, ok := method.Type().(*types.Signature)
	if !ok || signature.Results().Len() != 1 || !types.Identical(pkg.TypesInfo.TypeOf(lit), signature.Results().At(0).Type()) {
		return nil, false, nil
	}
	entries, ok := signature.Results().At(0).Type().Underlying().(*types.Slice)
	if !ok {
		return nil, false, nil
	}
	factoryType, direct, entryFields, ok := frameworkFactoryCollectionEntry(entries.Elem())
	if !ok {
		return nil, false, nil
	}
	specs := make([]entrypointSpec, 0, len(lit.Elts))
	for _, elt := range lit.Elts {
		factoryExpr := elt
		typeName := ""
		if !direct {
			entry, ok := entrypointListItem(elt)
			if !ok {
				return nil, true, fmt.Errorf("service package method %s framework factory list contains %T, not a factory entry", decl.Name.Name, elt)
			}
			seen := make(map[string]bool, len(entry.Elts))
			for _, element := range entry.Elts {
				field, ok := element.(*ast.KeyValueExpr)
				if !ok {
					return nil, true, fmt.Errorf("service package method %s framework factory entry contains an unkeyed field", decl.Name.Name)
				}
				name := exprIdentName(field.Key)
				fieldType, ok := entryFields[name]
				if !ok {
					return nil, true, fmt.Errorf("service package method %s framework factory entry contains unexpected field %s", decl.Name.Name, name)
				}
				if seen[name] {
					return nil, true, fmt.Errorf("service package method %s framework factory entry repeats %s", decl.Name.Name, name)
				}
				seen[name] = true
				if name == "Factory" {
					factoryExpr = field.Value
					continue
				}
				if name == "TypeName" {
					value, err := stringLiteralValue(field.Value)
					if err != nil {
						return nil, true, fmt.Errorf("service package method %s framework factory TypeName: %w", decl.Name.Name, err)
					}
					typeName = value
					continue
				}
				if valueType := pkg.TypesInfo.TypeOf(field.Value); valueType == nil || !types.AssignableTo(valueType, fieldType) {
					return nil, true, fmt.Errorf("service package method %s framework factory field %s has incompatible type", decl.Name.Name, name)
				}
			}
			if factoryExpr == nil {
				return nil, true, fmt.Errorf("service package method %s framework factory entry must contain Factory", decl.Name.Name)
			}
		}
		switch factoryExpr.(type) {
		case *ast.Ident, *ast.SelectorExpr:
		default:
			return nil, true, fmt.Errorf("service package method %s framework factory list contains %T, not an identifier or selector", decl.Name.Name, factoryExpr)
		}
		factory, factoryPkg, err := resolveFunctionObject(index, pkg, factoryExpr, nil)
		if err != nil {
			return nil, true, err
		}
		if factory == nil || factoryPkg == nil || factory.Pkg() == nil {
			return nil, true, errors.New("framework factory does not resolve to a package function")
		}
		if !types.Identical(factory.Type(), factoryType) {
			return nil, true, fmt.Errorf("framework factory %s has type %s, want %s", factory.Name(), factory.Type(), factoryType)
		}
		if typeName == "" {
			typeName, err = registeredEntrypointTypeName(index, factoryPkg, factory)
			if err != nil {
				return nil, true, err
			}
		}
		specs = append(specs, entrypointSpec{
			kind:        kind,
			typeName:    typeName,
			factory:     factory.Name(),
			sourceFile:  sourceFile,
			packagePath: factory.Pkg().Path(),
		})
	}
	return specs, true, nil
}

func frameworkFactoryCollectionEntry(typ types.Type) (types.Type, bool, map[string]types.Type, bool) {
	if _, ok := typ.Underlying().(*types.Signature); ok {
		return typ, true, nil, true
	}
	if pointer, ok := typ.(*types.Pointer); ok {
		typ = pointer.Elem()
	}
	entry, ok := typ.Underlying().(*types.Struct)
	if !ok {
		return nil, false, nil, false
	}
	fields := make(map[string]types.Type, entry.NumFields())
	var factoryType types.Type
	for i := range entry.NumFields() {
		field := entry.Field(i)
		switch field.Name() {
		case "Factory":
			if _, ok := field.Type().Underlying().(*types.Signature); !ok {
				return nil, false, nil, false
			}
			factoryType = field.Type()
		case "Name":
			if !types.Identical(field.Type(), types.Typ[types.String]) {
				return nil, false, nil, false
			}
		case "TypeName":
			if !types.Identical(field.Type(), types.Typ[types.String]) {
				return nil, false, nil, false
			}
		case "Tags":
			if !metadataStructFieldType(field.Type(), true) {
				return nil, false, nil, false
			}
		case "Region", "Identity", "Import":
			if !metadataStructFieldType(field.Type(), false) {
				return nil, false, nil, false
			}
		default:
			return nil, false, nil, false
		}
		fields[field.Name()] = field.Type()
	}
	if factoryType == nil {
		return nil, false, nil, false
	}
	return factoryType, false, fields, true
}

func metadataStructFieldType(typ types.Type, allowPointer bool) bool {
	if allowPointer {
		if pointer, ok := typ.(*types.Pointer); ok {
			typ = pointer.Elem()
		}
	}
	_, ok := typ.Underlying().(*types.Struct)
	return ok
}

func sdkFactoryCollectionSpecs(
	index *packageIndex,
	pkg *packages.Package,
	decl *ast.FuncDecl,
	kind, sourceFile string,
	expr ast.Expr,
) ([]entrypointSpec, bool, error) {
	if decl == nil || decl.Name == nil || !strings.HasPrefix(decl.Name.Name, "SDK") {
		return nil, false, nil
	}
	if _, ok := servicePackageFactoryCollectionField(decl.Name.Name, kind); !ok {
		return nil, false, nil
	}
	lit, ok := expr.(*ast.CompositeLit)
	if !ok || pkg == nil || pkg.TypesInfo == nil {
		return nil, false, nil
	}
	method, ok := pkg.TypesInfo.ObjectOf(decl.Name).(*types.Func)
	if !ok {
		return nil, false, nil
	}
	signature, ok := method.Type().(*types.Signature)
	if !ok || signature.Results().Len() != 1 || !types.Identical(pkg.TypesInfo.TypeOf(lit), signature.Results().At(0).Type()) {
		return nil, false, nil
	}
	entries, ok := signature.Results().At(0).Type().Underlying().(*types.Slice)
	if !ok {
		return nil, false, nil
	}
	entryType := entries.Elem()
	if pointer, ok := entryType.(*types.Pointer); ok {
		entryType = pointer.Elem()
	}
	entry, ok := entryType.Underlying().(*types.Struct)
	if !ok {
		return nil, false, nil
	}
	entryFields := make(map[string]types.Type, entry.NumFields())
	var factoryType types.Type
	hasTypeName := false
	for i := range entry.NumFields() {
		field := entry.Field(i)
		switch field.Name() {
		case "Factory":
			if !isSDKFactoryType(field.Type()) {
				return nil, false, nil
			}
			factoryType = field.Type()
		case "TypeName":
			if !types.Identical(field.Type(), types.Typ[types.String]) {
				return nil, false, nil
			}
			hasTypeName = true
		case "Name":
			if !types.Identical(field.Type(), types.Typ[types.String]) {
				return nil, false, nil
			}
		case "Tags":
			if !metadataStructFieldType(field.Type(), true) {
				return nil, false, nil
			}
		case "Region", "Identity", "Import":
			if !metadataStructFieldType(field.Type(), false) {
				return nil, false, nil
			}
		default:
			return nil, false, nil
		}
		entryFields[field.Name()] = field.Type()
	}
	if factoryType == nil || !hasTypeName {
		return nil, false, nil
	}

	specs := make([]entrypointSpec, 0, len(lit.Elts))
	for _, elt := range lit.Elts {
		entryLit, ok := entrypointListItem(elt)
		if !ok {
			return nil, true, fmt.Errorf("service package method %s SDK factory list contains %T, not a factory entry", decl.Name.Name, elt)
		}
		seen := make(map[string]bool, len(entryLit.Elts))
		var typeName string
		var factoryExpr ast.Expr
		for _, element := range entryLit.Elts {
			field, ok := element.(*ast.KeyValueExpr)
			if !ok {
				return nil, true, fmt.Errorf("service package method %s SDK factory entry contains an unkeyed field", decl.Name.Name)
			}
			name := exprIdentName(field.Key)
			fieldType, ok := entryFields[name]
			if !ok {
				return nil, true, fmt.Errorf("service package method %s SDK factory entry contains unexpected field %s", decl.Name.Name, name)
			}
			if seen[name] {
				return nil, true, fmt.Errorf("service package method %s SDK factory entry repeats %s", decl.Name.Name, name)
			}
			seen[name] = true
			switch name {
			case "Factory":
				factoryExpr = field.Value
			case "TypeName":
				value, err := stringLiteralValue(field.Value)
				if err != nil {
					return nil, true, fmt.Errorf("service package method %s SDK factory TypeName: %w", decl.Name.Name, err)
				}
				if value == "" {
					return nil, true, fmt.Errorf("service package method %s SDK factory TypeName is empty", decl.Name.Name)
				}
				typeName = value
			default:
				if valueType := pkg.TypesInfo.TypeOf(field.Value); valueType == nil || !types.AssignableTo(valueType, fieldType) {
					return nil, true, fmt.Errorf("service package method %s SDK factory field %s has incompatible type", decl.Name.Name, name)
				}
			}
		}
		if factoryExpr == nil || typeName == "" {
			return nil, true, fmt.Errorf("service package method %s SDK factory entry is incomplete", decl.Name.Name)
		}
		switch factoryExpr.(type) {
		case *ast.Ident, *ast.SelectorExpr:
		default:
			return nil, true, fmt.Errorf("service package method %s SDK factory is %T, not an identifier or selector", decl.Name.Name, factoryExpr)
		}
		factory, factoryPkg, err := resolveFunctionObject(index, pkg, factoryExpr, nil)
		if err != nil {
			return nil, true, err
		}
		if factory == nil || factoryPkg == nil || factory.Pkg() == nil {
			return nil, true, errors.New("SDK factory does not resolve to a package function")
		}
		if !types.Identical(factory.Type(), factoryType) {
			return nil, true, fmt.Errorf("SDK factory %s has type %s, want %s", factory.Name(), factory.Type(), factoryType)
		}
		specs = append(specs, entrypointSpec{
			kind:        kind,
			typeName:    typeName,
			factory:     factory.Name(),
			sourceFile:  sourceFile,
			packagePath: factory.Pkg().Path(),
		})
	}
	return specs, true, nil
}

func servicePackageEmptySDKFactoryMapReturn(pkg *packages.Package, decl *ast.FuncDecl, kind string, expr ast.Expr) bool {
	if decl == nil || decl.Name == nil {
		return false
	}
	if _, ok := servicePackageFactoryCollectionField(decl.Name.Name, kind); !ok {
		return false
	}
	lit, ok := expr.(*ast.CompositeLit)
	if !ok || len(lit.Elts) != 0 || pkg == nil || pkg.TypesInfo == nil {
		return false
	}
	method, ok := pkg.TypesInfo.ObjectOf(decl.Name).(*types.Func)
	if !ok {
		return false
	}
	signature, ok := method.Type().(*types.Signature)
	if !ok || signature.Results().Len() != 1 || !types.Identical(pkg.TypesInfo.TypeOf(lit), signature.Results().At(0).Type()) {
		return false
	}
	entries, ok := signature.Results().At(0).Type().Underlying().(*types.Map)
	if !ok || !types.Identical(entries.Key(), types.Typ[types.String]) || !isSDKFactoryType(entries.Elem()) {
		return false
	}
	return true
}

func isSDKFactoryCollectionEntry(typ types.Type) bool {
	if pointer, ok := typ.(*types.Pointer); ok {
		typ = pointer.Elem()
	}
	entry, ok := typ.Underlying().(*types.Struct)
	if !ok {
		return false
	}
	var hasTypeName, hasFactory bool
	for i := range entry.NumFields() {
		field := entry.Field(i)
		switch field.Name() {
		case "TypeName":
			hasTypeName = types.Identical(field.Type(), types.Typ[types.String])
		case "Factory":
			hasFactory = isSDKFactoryType(field.Type())
		case "Name":
			if !types.Identical(field.Type(), types.Typ[types.String]) {
				return false
			}
		case "Tags":
			pointer, ok := field.Type().(*types.Pointer)
			if !ok {
				return false
			}
			if _, ok := pointer.Elem().Underlying().(*types.Struct); !ok {
				return false
			}
		default:
			return false
		}
	}
	return hasTypeName && hasFactory
}

func unwrapSpecList(pkg *packages.Package, expr ast.Expr, locals map[string]ast.Expr) ([]*ast.CompositeLit, bool, error) {
	switch expr := expr.(type) {
	case *ast.Ident:
		resolved := resolveExpr(expr, locals)
		if resolved == expr {
			return nil, false, nil
		}
		return unwrapSpecList(pkg, resolved, locals)
	case *ast.CallExpr:
		if selector, ok := expr.Fun.(*ast.SelectorExpr); ok && selector.Sel != nil && selector.Sel.Name == "Values" && len(expr.Args) == 1 {
			return unwrapSpecList(pkg, resolveExpr(expr.Args[0], locals), locals)
		}
		return nil, false, nil
	case *ast.CompositeLit:
		if specCompositeLit(expr) {
			return []*ast.CompositeLit{expr}, true, nil
		}
		if !isListCompositeLit(pkg, expr) {
			return nil, false, nil
		}
		items := make([]*ast.CompositeLit, 0, len(expr.Elts))
		for _, elt := range expr.Elts {
			lit, ok := entrypointListItem(elt)
			if !ok {
				return nil, false, fmt.Errorf("entrypoint list contains %T, not a composite literal", elt)
			}
			items = append(items, lit)
		}
		return items, true, nil
	default:
		return nil, false, nil
	}
}

func entrypointListItem(expr ast.Expr) (*ast.CompositeLit, bool) {
	switch expr := expr.(type) {
	case *ast.CompositeLit:
		return expr, true
	case *ast.UnaryExpr:
		if expr.Op != token.AND {
			return nil, false
		}
		lit, ok := expr.X.(*ast.CompositeLit)
		return lit, ok
	default:
		return nil, false
	}
}

func entrypointSpecsFromMapLit(pkg *packages.Package, kind, sourceFile string, expr ast.Expr) ([]entrypointSpec, bool, error) {
	lit, ok := expr.(*ast.CompositeLit)
	if !ok {
		return nil, false, nil
	}
	if !isMapCompositeLit(pkg, lit) {
		return nil, false, nil
	}
	specs := make([]entrypointSpec, 0, len(lit.Elts))
	for _, elt := range lit.Elts {
		kv, ok := elt.(*ast.KeyValueExpr)
		if !ok {
			return nil, true, fmt.Errorf("entrypoint map contains %T, not a key/value entry", elt)
		}
		typeName, err := stringLiteralValue(kv.Key)
		if err != nil {
			return nil, true, fmt.Errorf("entrypoint map key: %w", err)
		}
		factory, packagePath, err := entrypointFactory(pkg, kv.Value)
		if err != nil {
			return nil, true, fmt.Errorf("map entrypoint %s: %w", typeName, err)
		}
		specs = append(specs, entrypointSpec{
			kind:        kind,
			typeName:    typeName,
			factory:     factory,
			sourceFile:  sourceFile,
			packagePath: packagePath,
		})
	}
	return specs, true, nil
}

func isMapCompositeLit(pkg *packages.Package, lit *ast.CompositeLit) bool {
	if _, ok := lit.Type.(*ast.MapType); ok {
		return true
	}
	if pkg == nil || pkg.TypesInfo == nil {
		return false
	}
	typ := pkg.TypesInfo.TypeOf(lit)
	if typ == nil {
		return false
	}
	_, ok := typ.Underlying().(*types.Map)
	return ok
}

func isListCompositeLit(pkg *packages.Package, lit *ast.CompositeLit) bool {
	if _, ok := lit.Type.(*ast.ArrayType); ok {
		return true
	}
	if pkg == nil || pkg.TypesInfo == nil {
		return false
	}
	typ := pkg.TypesInfo.TypeOf(lit)
	if typ == nil {
		return false
	}
	switch typ.Underlying().(type) {
	case *types.Array, *types.Slice:
		return true
	default:
		return false
	}
}
func specCompositeLit(expr *ast.CompositeLit) bool {
	for _, elt := range expr.Elts {
		kv, ok := elt.(*ast.KeyValueExpr)
		if ok && (exprIdentName(kv.Key) == "Factory" || exprIdentName(kv.Key) == "TypeName") {
			return true
		}
	}
	return false

}

var errIncompleteEntrypointSpec = errors.New("incomplete entrypoint spec")

func entrypointFromCompositeLit(pkg *packages.Package, kind, sourceFile string, lit *ast.CompositeLit) (entrypointSpec, error) {
	var factoryExpr ast.Expr
	var typeName string
	for _, elt := range lit.Elts {
		kv, ok := elt.(*ast.KeyValueExpr)
		if !ok {
			continue
		}
		key := exprIdentName(kv.Key)
		switch key {
		case "Factory":
			factoryExpr = kv.Value
		case "TypeName":
			value, err := stringLiteralValue(kv.Value)
			if err != nil {
				return entrypointSpec{}, err
			}
			typeName = value
		}
	}
	if factoryExpr == nil || typeName == "" {
		return entrypointSpec{}, fmt.Errorf("%w: missing Factory or TypeName", errIncompleteEntrypointSpec)
	}
	factory, packagePath, err := entrypointFactory(pkg, factoryExpr)
	if err != nil {
		return entrypointSpec{}, err
	}
	return entrypointSpec{
		kind:        kind,
		typeName:    typeName,
		factory:     factory,
		sourceFile:  sourceFile,
		packagePath: packagePath,
	}, nil
}

func entrypointFactory(pkg *packages.Package, expr ast.Expr) (string, string, error) {
	var object types.Object
	switch expr := expr.(type) {
	case *ast.Ident:
		object = pkg.TypesInfo.ObjectOf(expr)
	case *ast.SelectorExpr:
		object = pkg.TypesInfo.ObjectOf(expr.Sel)
	default:
		return "", "", fmt.Errorf("factory expression %T is unsupported", expr)
	}
	factory, ok := object.(*types.Func)
	if !ok || factory.Pkg() == nil {
		return "", "", errors.New("factory does not resolve to a package function")
	}
	return factory.Name(), factory.Pkg().Path(), nil
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
		if obj.Pkg() == nil {
			return nil, nil, fmt.Errorf("selector function %s has no package", obj.Name())
		}
		calleePkg, err := packageForTypes(index, obj.Pkg())
		if err != nil {
			return nil, nil, fmt.Errorf("selector function package %s is not loaded: %w", obj.Pkg().Path(), err)
		}
		return obj, calleePkg, nil
	default:
		return nil, nil, fmt.Errorf("unsupported function expression %T", expr)
	}
}

func packageForTypes(index *packageIndex, typePkg *types.Package) (*packages.Package, error) {
	if typePkg == nil {
		return nil, errors.New("type package is missing")
	}
	if pkg := index.byTypes[typePkg]; pkg != nil {
		return pkg, nil
	}
	return packageForPath(index, typePkg.Path())
}

func packageForPath(index *packageIndex, path string) (*packages.Package, error) {
	packagesForPath := index.byPath[path]
	switch len(packagesForPath) {
	case 0:
		return nil, fmt.Errorf("package %s is not loaded", path)
	case 1:
		return packagesForPath[0], nil
	default:
		return nil, fmt.Errorf("package %s is ambiguous across %d type universes", path, len(packagesForPath))
	}
}

func directSSAFunctionsForSelection(index *packageIndex, selection *types.Selection, name string) ([]*ssa.Function, bool, error) {
	if index == nil || selection == nil || selection.Kind() != types.MethodVal {
		return nil, false, nil
	}
	method, ok := selection.Obj().(*types.Func)
	if !ok {
		return nil, true, fmt.Errorf("method selection for %s has no declared function", name)
	}
	if index.prog != nil {
		if fn := index.prog.MethodValue(selection); fn != nil && fn.Syntax() != nil {
			funcs, err := canonicalConcreteSSAFunctionsForObject([]*ssa.Function{fn}, method, name)
			return funcs, true, err
		}
		if fn := index.prog.FuncValue(method); fn != nil {
			funcs, err := canonicalConcreteSSAFunctionsForObject([]*ssa.Function{fn}, method, name)
			return funcs, true, err
		}
	}
	if method.Origin() != method {
		funcs, err := ssaFunctionsForMethodSource(index, method, name)
		return funcs, true, err
	}
	funcs, err := ssaFunctionsForObject(index, method, name)
	if err != nil {
		return nil, true, err
	}
	funcs, err = canonicalConcreteSSAFunctionsForObject(funcs, method, name)
	return funcs, true, err
}

func ssaFunctionsForMethodSource(index *packageIndex, method *types.Func, name string) ([]*ssa.Function, error) {
	if index == nil || method == nil {
		return nil, fmt.Errorf("SSA function missing for %s", name)
	}
	origin := method.Origin()
	identity, ok := canonicalFunctionIdentity(origin)
	if !ok {
		return nil, fmt.Errorf("SSA function missing for %s", name)
	}

	declarations := append([]*ast.FuncDecl(nil), index.funcDeclsByIdentity[identity]...)
	if declaration := index.funcDecls[origin]; declaration != nil {
		declarations = append(declarations, declaration)
	}
	groups := make(map[sourceDeclarationIdentity][]*ast.FuncDecl, len(declarations))
	for _, declaration := range declarations {
		if declaration == nil || declaration.Body == nil {
			continue
		}
		source := sourceDeclarationIdentityFor(index, declaration, identity)
		groups[source] = append(groups[source], declaration)
	}
	switch len(groups) {
	case 0:
		return nil, fmt.Errorf("SSA function missing for %s", name)
	case 1:
	default:
		return nil, fmt.Errorf(
			"SSA function resolution ambiguous for concrete %s: %d source declarations",
			name,
			len(groups),
		)
	}

	var source sourceDeclarationIdentity
	var sourceDeclarations []*ast.FuncDecl
	for source, sourceDeclarations = range groups {
		break
	}
	funcs := make([]*ssa.Function, 0)
	seen := make(map[*ssa.Function]struct{})
	add := func(fn *ssa.Function) {
		if fn == nil {
			return
		}
		declaration, ok := fn.Syntax().(*ast.FuncDecl)
		if !ok || sourceDeclarationIdentityFor(index, declaration, identity) != source {
			return
		}
		if _, ok := seen[fn]; ok {
			return
		}
		seen[fn] = struct{}{}
		funcs = append(funcs, fn)
	}
	for _, declaration := range sourceDeclarations {
		for _, fn := range index.ssaBySyntax[declaration] {
			add(fn)
		}
	}
	if index.prog != nil {
		add(index.prog.FuncValue(origin))
	}
	return canonicalConcreteSSAFunctionsForObject(funcs, method, name)
}

func sourceDeclarationIdentityFor(index *packageIndex, declaration *ast.FuncDecl, origin functionIdentity) sourceDeclarationIdentity {
	identity := sourceDeclarationIdentity{
		packagePath: origin.packagePath,
		receiver:    origin.receiver,
		method:      origin.method,
		declaration: declaration,
	}
	if index == nil || index.prog == nil || index.prog.Fset == nil || declaration == nil {
		return identity
	}
	start := index.prog.Fset.PositionFor(declaration.Pos(), false)
	end := index.prog.Fset.PositionFor(declaration.End(), false)
	if start.Filename == "" || end.Filename == "" {
		return identity
	}
	identity.file = filepath.Clean(start.Filename)
	identity.start = start.Offset
	identity.end = end.Offset
	identity.declaration = nil
	return identity
}

func ssaFunctionsForObject(index *packageIndex, obj *types.Func, name string) ([]*ssa.Function, error) {
	if obj == nil {
		return nil, fmt.Errorf("SSA function missing for %s", name)
	}
	if funcs := index.ssaFuncs[obj]; len(funcs) != 0 {
		return canonicalConcreteSSAFunctions(funcs, name)
	}
	if decl := index.funcDecls[obj]; decl != nil && decl.Body != nil {
		if funcs := index.ssaBySyntax[decl]; len(funcs) != 0 {
			return canonicalConcreteSSAFunctions(funcs, name)
		}
	}

	// packages.Load can retain a second go/types universe for the same source
	// declaration, so the Func pointer used by method selection need not be
	// the pointer held by SSA.

	identity, ok := canonicalFunctionIdentity(obj)
	if !ok {
		return nil, fmt.Errorf("SSA function missing for %s", name)
	}
	syntaxFuncs := make([]*ssa.Function, 0)
	for _, decl := range index.funcDeclsByIdentity[identity] {
		if decl != nil && decl.Body != nil {
			syntaxFuncs = append(syntaxFuncs, index.ssaBySyntax[decl]...)
		}
	}
	if len(syntaxFuncs) != 0 {
		return canonicalConcreteSSAFunctions(syntaxFuncs, name)
	}
	return canonicalConcreteSSAFunctions(index.ssaFuncsByIdentity[identity], name)
}

func canonicalFunctionIdentity(obj *types.Func) (functionIdentity, bool) {
	if obj == nil {
		return functionIdentity{}, false
	}
	obj = obj.Origin()
	if obj.Pkg() == nil {
		return functionIdentity{}, false
	}
	signature, ok := obj.Type().(*types.Signature)
	if !ok {
		return functionIdentity{}, false
	}
	return functionIdentity{
		packagePath: obj.Pkg().Path(),
		receiver:    canonicalReceiverIdentity(signature),
		method:      obj.Name(),
		signature:   canonicalTypeIdentity(signature),
	}, true
}

func canonicalReceiverIdentity(signature *types.Signature) string {
	if signature == nil || signature.Recv() == nil {
		return ""
	}
	return canonicalTypeIdentity(signature.Recv().Type())
}

func canonicalConcreteSSAFunctionsForObject(funcs []*ssa.Function, method *types.Func, name string) ([]*ssa.Function, error) {
	expected, ok := canonicalFunctionIdentity(method)
	if !ok {
		return nil, fmt.Errorf("SSA function missing for %s", name)
	}
	matching := make([]*ssa.Function, 0, len(funcs))
	for _, fn := range funcs {
		if fn == nil {
			continue
		}
		origin := fn.Origin()
		if origin == nil {
			origin = fn
		}
		obj, ok := origin.Object().(*types.Func)
		if !ok {
			continue
		}
		identity, ok := canonicalFunctionIdentity(obj)
		if !ok || identity != expected {
			continue
		}
		matching = append(matching, fn)
	}
	return canonicalConcreteSSAFunctions(matching, name)
}

func canonicalTypeIdentity(typ types.Type) string {
	return types.TypeString(typ, func(pkg *types.Package) string {
		if pkg == nil {
			return ""
		}
		return pkg.Path()
	})
}

func canonicalConcreteSSAFunctions(funcs []*ssa.Function, name string) ([]*ssa.Function, error) {
	unique := make([]*ssa.Function, 0, len(funcs))
	seen := make(map[*ssa.Function]struct{}, len(funcs))
	sources := make(map[ssaFunctionSourceIdentity]struct{}, len(funcs))
	for _, fn := range funcs {
		if fn == nil {
			continue
		}
		decl, ok := fn.Syntax().(*ast.FuncDecl)
		if !ok || decl.Body == nil {
			continue
		}
		genericOrigin := fn.Origin()
		if genericOrigin == nil {
			genericOrigin = fn
		}
		obj, ok := genericOrigin.Object().(*types.Func)
		if !ok {
			continue
		}
		origin, ok := canonicalFunctionIdentity(obj)
		if !ok {
			continue
		}
		if _, ok := seen[fn]; ok {
			continue
		}
		seen[fn] = struct{}{}
		unique = append(unique, fn)
		sources[ssaFunctionSourceIdentityFor(fn, decl, origin)] = struct{}{}
	}

	switch len(unique) {
	case 0:
		return nil, fmt.Errorf("SSA function missing for %s", name)
	case 1:
		return unique, nil
	}
	if len(sources) == 1 {
		return unique, nil
	}
	return nil, fmt.Errorf(
		"SSA function resolution ambiguous for concrete %s: %d source declarations across %d SSA functions",
		name,
		len(sources),
		len(unique),
	)
}

func ssaFunctionSourceIdentityFor(fn *ssa.Function, declaration *ast.FuncDecl, origin functionIdentity) ssaFunctionSourceIdentity {
	identity := ssaFunctionSourceIdentity{origin: origin}
	if fn == nil || fn.Prog == nil || fn.Prog.Fset == nil || declaration == nil {
		identity.declarationSyntax = declaration
		return identity
	}
	position := fn.Prog.Fset.PositionFor(declaration.Pos(), false)
	if position.Filename == "" {
		identity.declarationSyntax = declaration
		return identity
	}
	identity.declarationFile = position.Filename
	identity.declarationOffset = position.Offset
	return identity
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
	return strings.HasSuffix(named.Obj().Pkg().Path(), "/helper/schema") && named.Obj().Name() == "Resource"
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
