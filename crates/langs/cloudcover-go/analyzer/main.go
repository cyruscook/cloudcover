package main

import (
	"encoding/json"
	"fmt"
	"go/types"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"unsafe"

	"github.com/hashicorp/terraform-config-inspect/tfconfig"
	"golang.org/x/tools/go/callgraph"
	"golang.org/x/tools/go/callgraph/vta"
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

type moduleReplacement struct {
	Path    string `json:"path"`
	Version string `json:"version,omitempty"`
}

type module struct {
	Path    string             `json:"path"`
	Version string             `json:"version"`
	Replace *moduleReplacement `json:"replace,omitempty"`
}

type successResponse struct {
	Methods []method `json:"methods"`
	Modules []module `json:"modules"`
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
	methods, modules, err := analyzeDir(dir)
	if err != nil {
		return mustCString(marshalError(err.Error()))
	}
	payload, marshalErr := json.Marshal(successResponse{Methods: methods, Modules: modules})
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

func analyzeDir(dir string) ([]method, []module, error) {
	initial, err := packages.Load(&packages.Config{
		Mode:  packages.LoadAllSyntax | packages.NeedModule,
		Dir:   dir,
		Tests: false,
	}, "./...")
	if err != nil {
		return nil, nil, err
	}

	loadErrors := make([]string, 0)
	modulesByPath := make(map[string]module)
	packages.Visit(initial, nil, func(pkg *packages.Package) {
		for _, pkgErr := range pkg.Errors {
			loadErrors = append(loadErrors, pkgErr.Error())
		}
		if pkg.Module != nil && !pkg.Module.Main {
			modulesByPath[pkg.Module.Path] = resolvedModule(pkg.Module)
		}
	})
	if len(loadErrors) > 0 {
		return nil, nil, stringError(strings.Join(loadErrors, "; "))
	}

	prog, ssaPackages := ssautil.AllPackages(initial, ssa.InstantiateGenerics)
	for _, pkg := range ssaPackages {
		pkg.Build()
	}
	prog.Build()
	cg := vta.CallGraph(ssautil.AllFunctions(prog), nil)

	methods := make([]method, 0)
	visitErr := callgraph.GraphVisitEdges(cg, func(edge *callgraph.Edge) error {
		calleeMethod, ok := edgeToMethod(edge)
		if ok {
			methods = append(methods, calleeMethod)
		}
		return nil
	})
	if visitErr != nil {
		return nil, nil, visitErr
	}

	slices.SortFunc(methods, compareMethods)
	methods = slices.CompactFunc(methods, func(left, right method) bool {
		return compareMethods(left, right) == 0
	})
	modules := make([]module, 0, len(modulesByPath))
	for _, dependency := range modulesByPath {
		modules = append(modules, dependency)
	}
	slices.SortFunc(modules, compareModules)
	return methods, modules, nil
}

func resolvedModule(value *packages.Module) module {
	result := module{Path: value.Path, Version: value.Version}
	if value.Replace != nil {
		result.Replace = &moduleReplacement{
			Path:    value.Replace.Path,
			Version: value.Replace.Version,
		}
	}
	return result
}

func compareModules(left, right module) int {
	if cmp := compareStrings(left.Path, right.Path); cmp != 0 {
		return cmp
	}
	return compareStrings(left.Version, right.Version)
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

type terraformReference struct {
	Kind     string `json:"kind"`
	TypeName string `json:"type_name"`
	Action   string `json:"action"`
}

type terraformSuccessResponse struct {
	Methods         []terraformReference `json:"methods"`
	ProviderVersion string               `json:"provider_version"`
}

type terraformModuleManifest struct {
	Modules []terraformModuleManifestEntry `json:"Modules"`
}

type terraformModuleManifestEntry struct {
	Key string `json:"Key"`
	Dir string `json:"Dir"`
}

//export CloudCoverAnalyzeTerraform
func CloudCoverAnalyzeTerraform(path *C.char) *C.char {
	if path == nil {
		return mustCString(marshalError("path is required"))
	}
	references, providerVersion, err := analyzeTerraformDir(C.GoString(path))
	if err != nil {
		return mustCString(marshalError(err.Error()))
	}
	payload, marshalErr := json.Marshal(terraformSuccessResponse{
		Methods: references, ProviderVersion: providerVersion,
	})
	if marshalErr != nil {
		return mustCString(marshalError(marshalErr.Error()))
	}
	return mustCString(string(payload))
}

//export CloudCoverFreeTerraformCString
func CloudCoverFreeTerraformCString(ptr *C.char) {
	if ptr != nil {
		C.free(unsafe.Pointer(ptr))
	}
}

func analyzeTerraformDir(root string) ([]terraformReference, string, error) {
	info, err := os.Stat(root)
	if err != nil {
		return nil, "", err
	}
	if !info.IsDir() {
		return nil, "", fmt.Errorf("path is not a directory: %s", root)
	}

	lockPath := filepath.Join(root, ".terraform.lock.hcl")
	if _, err := os.Stat(lockPath); err != nil {
		return nil, "", fmt.Errorf("Terraform provider lock file is required at %s", lockPath)
	}
	manifestPath := filepath.Join(root, ".terraform", "modules", "modules.json")
	if _, err := os.Stat(manifestPath); err != nil {
		return nil, "", fmt.Errorf("Terraform module manifest is required at %s", manifestPath)
	}

	configuration := tfconfig.LoadPostInit(root, filepath.Join(root, ".terraform"))
	for _, diagnostic := range configuration.Diagnostics {
		if diagnostic.Severity == tfconfig.DiagError {
			return nil, "", fmt.Errorf("Terraform initialization metadata: %s", diagnostic.Detail)
		}
	}
	provider, ok := configuration.Providers["registry.terraform.io/hashicorp/aws"]
	if !ok || provider.Version == "" {
		return nil, "", fmt.Errorf("Terraform AWS provider version is missing from .terraform.lock.hcl")
	}

	content, err := os.ReadFile(manifestPath)
	if err != nil {
		return nil, "", err
	}
	var manifest terraformModuleManifest
	if err := json.Unmarshal(content, &manifest); err != nil {
		return nil, "", fmt.Errorf("failed to parse Terraform module manifest: %w", err)
	}
	moduleDirs := make(map[string]string, len(manifest.Modules)+1)
	moduleDirs[""] = root
	for _, module := range manifest.Modules {
		if module.Key == "" {
			continue
		}
		if module.Dir == "" {
			return nil, "", fmt.Errorf("Terraform module manifest entry %q has no directory", module.Key)
		}
		dir := module.Dir
		if !filepath.IsAbs(dir) {
			dir = filepath.Join(root, dir)
		}
		moduleDirs[module.Key] = filepath.Clean(dir)
	}

	rootModule, diagnostics := tfconfig.LoadModule(root)
	if diagnostics.HasErrors() {
		return nil, "", fmt.Errorf("failed to parse Terraform module %s: %s", root, diagnostics.Error())
	}
	modules := map[string]*tfconfig.Module{"": rootModule}
	moduleKeys := []string{""}
	for len(moduleKeys) > 0 {
		parentKey := moduleKeys[0]
		moduleKeys = moduleKeys[1:]
		parent := modules[parentKey]

		callNames := make([]string, 0, len(parent.ModuleCalls))
		for callName := range parent.ModuleCalls {
			callNames = append(callNames, callName)
		}
		sort.Strings(callNames)
		for _, callName := range callNames {
			moduleKey := callName
			if parentKey != "" {
				moduleKey = parentKey + "." + callName
			}
			if _, loaded := modules[moduleKey]; loaded {
				continue
			}
			dir, ok := moduleDirs[moduleKey]
			if !ok {
				return nil, "", fmt.Errorf(
					"Terraform module %q is missing from the initialized module manifest",
					moduleKey,
				)
			}
			module, diagnostics := tfconfig.LoadModule(dir)
			if diagnostics.HasErrors() {
				return nil, "", fmt.Errorf("failed to parse Terraform module %s: %s", dir, diagnostics.Error())
			}
			modules[moduleKey] = module
			moduleKeys = append(moduleKeys, moduleKey)
		}
	}

	seen := make(map[string]bool)
	references := make([]terraformReference, 0)
	for _, module := range modules {
		dir := module.Path
		if seen[dir] {
			continue
		}
		seen[dir] = true
		for _, resource := range module.ManagedResources {
			if !usesAWSProvider(module, resource.Provider) {
				continue
			}
			for _, action := range []string{"create", "read", "update", "delete"} {
				references = append(references, terraformReference{
					Kind: "resource", TypeName: resource.Type, Action: action,
				})
			}
		}
		for _, resource := range module.DataResources {
			if !usesAWSProvider(module, resource.Provider) {
				continue
			}
			references = append(references, terraformReference{
				Kind: "data_source", TypeName: resource.Type, Action: "read",
			})
		}
	}

	sort.Slice(references, func(i, j int) bool {
		left, right := references[i], references[j]
		if left.Kind != right.Kind {
			return left.Kind < right.Kind
		}
		if left.TypeName != right.TypeName {
			return left.TypeName < right.TypeName
		}
		return left.Action < right.Action
	})
	result := references[:0]
	for _, reference := range references {
		if len(result) == 0 || result[len(result)-1] != reference {
			result = append(result, reference)
		}
	}
	return result, provider.Version, nil
}

func usesAWSProvider(module *tfconfig.Module, provider tfconfig.ProviderRef) bool {
	requirement, ok := module.RequiredProviders[provider.Name]
	if !ok || requirement.Source == "" {
		return provider.Name == "aws"
	}
	return requirement.Source == "hashicorp/aws" ||
		requirement.Source == "registry.terraform.io/hashicorp/aws"
}
