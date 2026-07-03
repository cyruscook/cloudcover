package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"go/constant"
	"go/types"
	"os"
	"path/filepath"
	"slices"

	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/static"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
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

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	sdkDir := flag.String("sdk-dir", "", "path to aws-sdk-go-v2 checkout")
	flag.Parse()
	if *sdkDir == "" {
		return errors.New("--sdk-dir is required")
	}
	if flag.NArg() != 0 {
		return fmt.Errorf("unexpected positional arguments: %v", flag.Args())
	}

	serviceDirs, err := discoverServiceDirs(*sdkDir)
	if err != nil {
		return err
	}

	seen := make(map[mappingKey]struct{})
	rows := make([]mappingRow, 0)
	for _, serviceDir := range serviceDirs {
		serviceRows, err := loadServiceRows(serviceDir)
		if err != nil {
			return err
		}
		for _, row := range serviceRows {
			key := mappingKey{
				pkg:    row.Package,
				method: row.Method,
				api:    row.APIMethods[0],
			}
			if _, ok := seen[key]; ok {
				continue
			}
			seen[key] = struct{}{}
			rows = append(rows, row)
		}
	}

	slices.SortFunc(rows, func(left, right mappingRow) int {
		if cmp := compareStrings(left.Package, right.Package); cmp != 0 {
			return cmp
		}
		if cmp := compareStrings(left.Receiver, right.Receiver); cmp != 0 {
			return cmp
		}
		if cmp := compareStrings(left.Method, right.Method); cmp != 0 {
			return cmp
		}
		return compareAPIMethod(left.APIMethods[0], right.APIMethods[0])
	})

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	return encoder.Encode(rows)
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
func loadServiceRows(serviceDir string) ([]mappingRow, error) {
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

	service := filepath.Base(serviceDir)
	packagePath := "github.com/aws/aws-sdk-go-v2/service/" + service
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
		// ignore helpers, paginators, waiters, and presigners: only exported Client
		// methods that dispatch into invokeOperation are treated as SDK entry points.
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
