package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"go/ast"
	"go/constant"
	"go/parser"
	"go/token"
	"go/types"
	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/static"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
	"os"
	"path/filepath"
	"slices"
	"strconv"
)

type apiMethod struct {
	Service string `json:"service"`
	Name    string `json:"name"`
}

type mappingRow struct {
	Package    string      `json:"package"`
	Receiver   string      `json:"receiver"`
	Method     string      `json:"method"`
	APIMethods []apiMethod `json:"api_methods"`
}

type mappingKey struct {
	pkg    string
	method string
	api    apiMethod
}

type paginatorClientField struct {
	name     string
	typeName string
	methods  map[string]struct{}
	direct   bool
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	serviceDir := flag.String("service-dir", "", "path to one aws-sdk-go-v2 service module")
	modulePath := flag.String("module-path", "", "Go module path for the selected service")
	fast := flag.Bool("fast", false, "parse generated client methods without SSA")
	flag.Parse()
	if *serviceDir == "" {
		return errors.New("--service-dir is required")
	}
	if *modulePath == "" {
		return errors.New("--module-path is required")
	}
	if flag.NArg() != 0 {
		return fmt.Errorf("unexpected positional arguments: %v", flag.Args())
	}

	var rows []mappingRow
	var err error
	if *fast {
		rows, err = loadFastServiceRows(*serviceDir, *modulePath)
	} else {
		rows, err = loadServiceRows(*serviceDir, *modulePath)
	}
	if err != nil {
		return err
	}
	paginatorRows, err := loadPaginatorRows(*serviceDir, *modulePath, rows)
	if err != nil {
		return err
	}
	rows = append(rows, paginatorRows...)
	slices.SortFunc(rows, compareMappingRows)
	rows = slices.CompactFunc(rows, func(left, right mappingRow) bool {
		return compareMappingRows(left, right) == 0
	})

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	return encoder.Encode(rows)
}

func loadFastServiceRows(serviceDir, modulePath string) ([]mappingRow, error) {
	service := filepath.Base(modulePath)
	packagePath := modulePath
	rows := make([]mappingRow, 0)
	err := filepath.WalkDir(serviceDir, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() || filepath.Ext(path) != ".go" {
			return nil
		}
		file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			return fmt.Errorf("parse %s: %w", path, err)
		}
		for _, declaration := range file.Decls {
			function, ok := declaration.(*ast.FuncDecl)
			if !ok || function.Recv == nil || !function.Name.IsExported() || len(function.Recv.List) == 0 {
				continue
			}
			if !isClientReceiver(function.Recv.List[0].Type) {
				continue
			}
			var operation string
			ast.Inspect(function.Body, func(node ast.Node) bool {
				if operation != "" {
					return false
				}
				call, ok := node.(*ast.CallExpr)
				if !ok {
					return true
				}
				selector, ok := call.Fun.(*ast.SelectorExpr)
				if !ok || selector.Sel.Name != "invokeOperation" {
					return true
				}
				for _, argument := range call.Args {
					literal, ok := argument.(*ast.BasicLit)
					if !ok || literal.Kind != token.STRING {
						continue
					}
					value, err := strconv.Unquote(literal.Value)
					if err == nil {
						operation = value
						break
					}
				}
				return true
			})
			if operation != "" {
				rows = append(rows, mappingRow{
					Package:    packagePath,
					Receiver:   "Client",
					Method:     function.Name.Name,
					APIMethods: []apiMethod{{Service: service, Name: operation}},
				})
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return rows, nil
}

func isClientReceiver(expression ast.Expr) bool {
	switch expression := expression.(type) {
	case *ast.Ident:
		return expression.Name == "Client"
	case *ast.StarExpr:
		return isClientReceiver(expression.X)
	default:
		return false
	}
}

// Paginator constructors only create local state. NextPage dispatches through a
// generated client field, which we resolve from the paginator's source rather
// than by matching an unrelated selector with the same name.
func loadPaginatorRows(serviceDir, modulePath string, clientRows []mappingRow) ([]mappingRow, error) {
	clientOperations, err := indexClientOperations(modulePath, clientRows)
	if err != nil {
		return nil, err
	}

	files, err := loadServiceFiles(serviceDir)
	if err != nil {
		return nil, err
	}
	paginatorStructs, clientInterfaces, err := indexPaginatorTypes(files)
	if err != nil {
		return nil, err
	}

	rows := make([]mappingRow, 0)
	seenPaginators := make(map[string]struct{})
	for _, file := range files {
		for _, declaration := range file.Decls {
			function, ok := declaration.(*ast.FuncDecl)
			if !ok || function.Recv == nil || function.Name.Name != "NextPage" || function.Body == nil {
				continue
			}
			receiver := receiverTypeName(function.Recv)
			if !isPaginatorReceiver(receiver) {
				continue
			}
			if _, ok := seenPaginators[receiver]; ok {
				return nil, fmt.Errorf("duplicate NextPage methods for paginator %s", receiver)
			}
			seenPaginators[receiver] = struct{}{}

			paginator, ok := paginatorStructs[receiver]
			if !ok {
				return nil, fmt.Errorf("paginator %s has NextPage but no source struct", receiver)
			}
			clientField, err := resolvePaginatorClientField(receiver, paginator, clientInterfaces)
			if err != nil {
				return nil, err
			}
			receiverVariable := receiverVariableName(function.Recv)
			if receiverVariable == "" {
				return nil, fmt.Errorf("paginator %s NextPage has an unnamed receiver", receiver)
			}
			calls := paginatorClientCalls(function.Body, receiverVariable, clientField.name)
			if len(calls) == 0 {
				return nil, fmt.Errorf("paginator %s NextPage does not call its client field %q", receiver, clientField.name)
			}
			if len(calls) != 1 {
				return nil, fmt.Errorf("paginator %s NextPage has %d calls through client field %q", receiver, len(calls), clientField.name)
			}
			clientMethod := calls[0]
			if !clientField.direct {
				if _, ok := clientField.methods[clientMethod]; !ok {
					return nil, fmt.Errorf("paginator %s NextPage calls %s through client field %q, but %s does not declare that method", receiver, clientMethod, clientField.name, clientField.typeName)
				}
			}
			operation, ok := clientOperations[clientMethod]
			if !ok {
				return nil, fmt.Errorf("paginator %s NextPage calls unknown Client.%s", receiver, clientMethod)
			}
			rows = append(rows, mappingRow{
				Package:    modulePath,
				Receiver:   receiver,
				Method:     function.Name.Name,
				APIMethods: []apiMethod{operation},
			})
		}
	}
	return rows, nil
}

func indexClientOperations(modulePath string, clientRows []mappingRow) (map[string]apiMethod, error) {
	operations := make(map[string]apiMethod, len(clientRows))
	for _, row := range clientRows {
		if row.Package != modulePath || row.Receiver != "Client" {
			continue
		}
		if len(row.APIMethods) != 1 {
			return nil, fmt.Errorf("Client.%s resolves to %d AWS operations", row.Method, len(row.APIMethods))
		}
		if _, exists := operations[row.Method]; exists {
			return nil, fmt.Errorf("duplicate mapping for Client.%s", row.Method)
		}
		operation := row.APIMethods[0]
		if operation.Name == "" || operation.Service == "" {
			return nil, fmt.Errorf("Client.%s has an incomplete AWS operation mapping", row.Method)
		}
		operations[row.Method] = operation
	}
	return operations, nil
}

func loadServiceFiles(serviceDir string) ([]*ast.File, error) {
	files := make([]*ast.File, 0)
	err := filepath.WalkDir(serviceDir, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() || filepath.Ext(path) != ".go" || filepath.Dir(path) != serviceDir {
			return nil
		}
		file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			return fmt.Errorf("parse %s: %w", path, err)
		}
		files = append(files, file)
		return nil
	})
	if err != nil {
		return nil, err
	}
	return files, nil
}

func indexPaginatorTypes(files []*ast.File) (map[string]*ast.StructType, map[string]*ast.InterfaceType, error) {
	paginatorStructs := make(map[string]*ast.StructType)
	clientInterfaces := make(map[string]*ast.InterfaceType)
	for _, file := range files {
		for _, declaration := range file.Decls {
			general, ok := declaration.(*ast.GenDecl)
			if !ok || general.Tok != token.TYPE {
				continue
			}
			for _, specification := range general.Specs {
				typeSpec, ok := specification.(*ast.TypeSpec)
				if !ok {
					continue
				}
				switch definition := typeSpec.Type.(type) {
				case *ast.StructType:
					if _, exists := paginatorStructs[typeSpec.Name.Name]; exists {
						return nil, nil, fmt.Errorf("duplicate source struct %s", typeSpec.Name.Name)
					}
					paginatorStructs[typeSpec.Name.Name] = definition
				case *ast.InterfaceType:
					if _, exists := clientInterfaces[typeSpec.Name.Name]; exists {
						return nil, nil, fmt.Errorf("duplicate source interface %s", typeSpec.Name.Name)
					}
					clientInterfaces[typeSpec.Name.Name] = definition
				}
			}
		}
	}
	return paginatorStructs, clientInterfaces, nil
}

func isPaginatorReceiver(receiver string) bool {
	const suffix = "Paginator"
	return len(receiver) > len(suffix) && receiver[len(receiver)-len(suffix):] == suffix
}

func resolvePaginatorClientField(receiver string, paginator *ast.StructType, clientInterfaces map[string]*ast.InterfaceType) (paginatorClientField, error) {
	var client *ast.Field
	for _, field := range paginator.Fields.List {
		for _, name := range field.Names {
			if name.Name != "client" {
				continue
			}
			if client != nil {
				return paginatorClientField{}, fmt.Errorf("paginator %s declares client field more than once", receiver)
			}
			client = field
		}
	}
	if client == nil {
		return paginatorClientField{}, fmt.Errorf("paginator %s has no client field", receiver)
	}
	typeName := expressionTypeName(client.Type)
	if typeName == "" {
		return paginatorClientField{}, fmt.Errorf("paginator %s client field has an unsupported type", receiver)
	}
	if typeName == "Client" {
		return paginatorClientField{name: "client", typeName: typeName, direct: true}, nil
	}
	clientInterface, ok := clientInterfaces[typeName]
	if !ok {
		return paginatorClientField{}, fmt.Errorf("paginator %s client field has unresolved type %s", receiver, typeName)
	}
	methods, err := interfaceMethodNames(typeName, clientInterface)
	if err != nil {
		return paginatorClientField{}, err
	}
	return paginatorClientField{name: "client", typeName: typeName, methods: methods}, nil
}

func interfaceMethodNames(name string, clientInterface *ast.InterfaceType) (map[string]struct{}, error) {
	methods := make(map[string]struct{})
	for _, field := range clientInterface.Methods.List {
		if len(field.Names) == 0 {
			return nil, fmt.Errorf("client interface %s embeds an unresolved method set", name)
		}
		for _, method := range field.Names {
			if _, exists := methods[method.Name]; exists {
				return nil, fmt.Errorf("client interface %s declares method %s more than once", name, method.Name)
			}
			methods[method.Name] = struct{}{}
		}
	}
	return methods, nil
}

func receiverVariableName(fields *ast.FieldList) string {
	if fields == nil || len(fields.List) == 0 || len(fields.List[0].Names) == 0 {
		return ""
	}
	return fields.List[0].Names[0].Name
}

func paginatorClientCalls(body *ast.BlockStmt, receiverVariable, clientField string) []string {
	calls := make([]string, 0, 1)
	ast.Inspect(body, func(node ast.Node) bool {
		if _, nestedFunction := node.(*ast.FuncLit); nestedFunction {
			return false
		}
		call, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		method, ok := call.Fun.(*ast.SelectorExpr)
		if !ok {
			return true
		}
		field, ok := method.X.(*ast.SelectorExpr)
		if !ok || field.Sel.Name != clientField {
			return true
		}
		receiver, ok := field.X.(*ast.Ident)
		if !ok || receiver.Name != receiverVariable {
			return true
		}
		calls = append(calls, method.Sel.Name)
		return true
	})
	return calls
}

func receiverTypeName(fields *ast.FieldList) string {
	if fields == nil || len(fields.List) == 0 {
		return ""
	}
	return expressionTypeName(fields.List[0].Type)
}

func expressionTypeName(expression ast.Expr) string {
	for {
		pointer, ok := expression.(*ast.StarExpr)
		if !ok {
			break
		}
		expression = pointer.X
	}
	identifier, _ := expression.(*ast.Ident)
	if identifier == nil {
		return ""
	}
	return identifier.Name
}

func discoverServiceDirs(sdkDir string) ([]string, error) {
	entries, err := os.ReadDir(filepath.Join(sdkDir, "service"))
	if err != nil {
		return nil, err
	}

	serviceDirs := make([]string, 0, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		serviceDir := filepath.Join(sdkDir, "service", entry.Name())
		if _, err := os.Stat(filepath.Join(serviceDir, "go.mod")); err == nil {
			serviceDirs = append(serviceDirs, serviceDir)
		} else if !errors.Is(err, os.ErrNotExist) {
			return nil, err
		}
	}

	slices.Sort(serviceDirs)
	return serviceDirs, nil
}

// Each aws-sdk-go-v2 service module is analyzed in isolation. We build SSA for
// the generated package, derive a static callgraph, then scan its edges for the
// standard request path used by generated client methods.
func loadServiceRows(serviceDir, modulePath string) ([]mappingRow, error) {
	initial, err := packages.Load(&packages.Config{
		Mode:  packages.LoadAllSyntax,
		Dir:   serviceDir,
		Tests: false,
	}, ".")
	if err != nil {
		return nil, fmt.Errorf("packages.Load failed for %s: %w", serviceDir, err)
	}
	if packages.PrintErrors(initial) > 0 {
		return nil, fmt.Errorf("packages.PrintErrors reported errors for %s", serviceDir)
	}

	prog, ssaPackages := ssautil.AllPackages(initial, ssa.InstantiateGenerics)
	for _, pkg := range ssaPackages {
		pkg.Build()
	}
	prog.Build()
	cg := static.CallGraph(prog)

	packagePath := modulePath
	rows := make([]mappingRow, 0)
	visitErr := callgraph.GraphVisitEdges(cg, func(edge *callgraph.Edge) error {
		row, ok, err := edgeToMapping(edge, packagePath)
		if err != nil {
			return err
		}
		if ok {
			rows = append(rows, row)
		}
		return nil
	})
	if visitErr != nil {
		return nil, fmt.Errorf("failed to inspect callgraph for %s: %w", serviceDir, visitErr)
	}

	return rows, nil
}

// We look for the generated pattern:
//
//	func (c *Client) GetObject(...) {
//	    ...
//	    result, metadata, err := c.invokeOperation(ctx, "GetObject", params, ...)
//	    ...
//	}
//
// The caller gives us the Go method reference users write in source
// (`service.(*Client).GetObject`); the callee and constant argument identify the
// API method it dispatches to. Static SSA edges are enough here because both
// sides live in generated code within the same package.
func edgeToMapping(edge *callgraph.Edge, packagePath string) (mappingRow, bool, error) {
	if edge == nil || edge.Caller == nil || edge.Callee == nil || edge.Site == nil {
		return mappingRow{}, false, nil
	}
	caller := edge.Caller.Func
	callee := edge.Callee.Func
	if caller == nil || callee == nil {
		return mappingRow{}, false, nil
	}
	callerPackage := caller.Package()
	calleePackage := callee.Package()
	if callerPackage == nil || calleePackage == nil || callerPackage.Pkg == nil || calleePackage.Pkg == nil {
		return mappingRow{}, false, nil
	}
	if callerPackage.Pkg.Path() != packagePath {
		return mappingRow{}, false, nil
	}
	if calleePackage.Pkg.Path() != packagePath || callee.Name() != "invokeOperation" {
		// Paginator NextPage methods are handled from their generated source
		// because their request call is indirect through an interface.
		return mappingRow{}, false, nil
	}
	if !caller.Object().Exported() {
		return mappingRow{}, false, nil
	}
	receiverName, ok := clientReceiverName(caller.Signature)
	if !ok {
		return mappingRow{}, false, nil
	}
	call := edge.Site.Common()
	if call == nil || len(call.Args) < 3 {
		return mappingRow{}, false, nil
	}
	operationConst, ok := call.Args[2].(*ssa.Const)
	if !ok || operationConst.Value == nil || operationConst.Value.Kind() != constant.String {
		return mappingRow{}, false, nil
	}

	return mappingRow{
		Package:  packagePath,
		Receiver: receiverName,
		Method:   caller.Name(),
		APIMethods: []apiMethod{{
			Service: filepath.Base(packagePath),
			Name:    constant.StringVal(operationConst.Value),
		}},
	}, true, nil
}

// Generated methods are defined on *Client. Strip pointer layers so the mapping
// remains stable even if SSA surfaces the receiver as Client or *Client.
func clientReceiverName(signature *types.Signature) (string, bool) {
	if signature == nil || signature.Recv() == nil {
		return "", false
	}
	receiverType := signature.Recv().Type()
	for {
		pointer, ok := receiverType.(*types.Pointer)
		if !ok {
			break
		}
		receiverType = pointer.Elem()
	}
	if named, ok := receiverType.(*types.Named); ok && named.Obj() != nil && named.Obj().Name() == "Client" {
		return named.Obj().Name(), true
	}
	return "", false
}
func compareMappingRows(left, right mappingRow) int {
	if cmp := compareStrings(left.Package, right.Package); cmp != 0 {
		return cmp
	}
	if cmp := compareStrings(left.Receiver, right.Receiver); cmp != 0 {
		return cmp
	}
	if cmp := compareStrings(left.Method, right.Method); cmp != 0 {
		return cmp
	}
	for index := range min(len(left.APIMethods), len(right.APIMethods)) {
		if cmp := compareAPIMethod(left.APIMethods[index], right.APIMethods[index]); cmp != 0 {
			return cmp
		}
	}
	return len(left.APIMethods) - len(right.APIMethods)
}

func compareAPIMethod(left, right apiMethod) int {
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
