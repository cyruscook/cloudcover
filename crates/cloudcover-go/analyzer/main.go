package main

import (
	"encoding/json"
	"go/types"
	"slices"
	"strings"
	"unsafe"

	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/static"
	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
)

/*
#include <stdlib.h>
*/
import "C"

type method struct {
	Package  string  `json:"package"`
	Receiver *string `json:"receiver"`
	Name     string  `json:"name"`
}

type successResponse struct {
	Methods []method `json:"methods"`
}

type errorResponse struct {
	Error string `json:"error"`
}

//export CloudCoverAnalyzeGo
func CloudCoverAnalyzeGo(path *C.char) *C.char {
	if path == nil {
		return mustCString(marshalError("path is required"))
	}
	dir := C.GoString(path)
	methods, err := analyzeDir(dir)
	if err != nil {
		return mustCString(marshalError(err.Error()))
	}
	payload, marshalErr := json.Marshal(successResponse{Methods: methods})
	if marshalErr != nil {
		return mustCString(marshalError(marshalErr.Error()))
	}
	return mustCString(string(payload))
}

//export CloudCoverFreeCString
func CloudCoverFreeCString(ptr *C.char) {
	if ptr != nil {
		C.free(unsafe.Pointer(ptr))
	}
}

func analyzeDir(dir string) ([]method, error) {
	initial, err := packages.Load(&packages.Config{
		Mode:  packages.LoadAllSyntax,
		Dir:   dir,
		Tests: false,
	}, "./...")
	if err != nil {
		return nil, err
	}

	loadErrors := make([]string, 0)
	packages.Visit(initial, nil, func(pkg *packages.Package) {
		for _, pkgErr := range pkg.Errors {
			loadErrors = append(loadErrors, pkgErr.Error())
		}
	})
	if len(loadErrors) > 0 {
		return nil, stringError(strings.Join(loadErrors, "; "))
	}

	prog, ssaPackages := ssautil.AllPackages(initial, ssa.InstantiateGenerics)
	for _, pkg := range ssaPackages {
		pkg.Build()
	}
	prog.Build()
	cg := static.CallGraph(prog)

	methods := make([]method, 0)
	visitErr := callgraph.GraphVisitEdges(cg, func(edge *callgraph.Edge) error {
		calleeMethod, ok := edgeToMethod(edge)
		if ok {
			methods = append(methods, calleeMethod)
		}
		return nil
	})
	if visitErr != nil {
		return nil, visitErr
	}

	slices.SortFunc(methods, compareMethods)
	methods = slices.Compact(methods)
	return methods, nil
}

func edgeToMethod(edge *callgraph.Edge) (method, bool) {
	if edge == nil || edge.Callee == nil {
		return method{}, false
	}
	callee := edge.Callee.Func
	if callee == nil {
		return method{}, false
	}
	pkg := callee.Package()
	if pkg == nil || pkg.Pkg == nil {
		return method{}, false
	}

	return method{
		Package:  pkg.Pkg.Path(),
		Receiver: receiverName(callee.Signature),
		Name:     callee.Name(),
	}, true
}

func receiverName(signature *types.Signature) *string {
	if signature == nil || signature.Recv() == nil {
		return nil
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
		name := named.Obj().Name()
		return &name
	}
	return nil
}

func compareMethods(left, right method) int {
	if left.Package < right.Package {
		return -1
	}
	if left.Package > right.Package {
		return 1
	}
	if left.Receiver == nil && right.Receiver != nil {
		return -1
	}
	if left.Receiver != nil && right.Receiver == nil {
		return 1
	}
	if left.Receiver != nil && right.Receiver != nil {
		if *left.Receiver < *right.Receiver {
			return -1
		}
		if *left.Receiver > *right.Receiver {
			return 1
		}
	}
	if left.Name < right.Name {
		return -1
	}
	if left.Name > right.Name {
		return 1
	}
	return 0
}

func marshalError(message string) string {
	payload, err := json.Marshal(errorResponse{Error: message})
	if err != nil {
		return `{"error":"failed to encode error response"}`
	}
	return string(payload)
}

func mustCString(value string) *C.char {
	return C.CString(value)
}

type stringError string

func (err stringError) Error() string {
	return string(err)
}


func main() {}
