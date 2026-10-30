package main

import (
	"go/types"
	"os"
	"path/filepath"
	"testing"
)

func TestLoadProviderIndexSupportsGo127UniqueExportData(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	// Provider v6.71.0 uses the standard library's unique.Handle through a
	// generic provider helper. Older x/tools cannot read its Go 1.27 export data.
	files := map[string]string{
		"go.mod":                        "module " + providerModulePath + "\n\ngo 1.27.3\n",
		"internal/provider/provider.go": "package provider\n",
		"internal/unique/handle.go": `package unique
import "unique"
func IsHandleNil[T comparable](h unique.Handle[T]) bool {
	var zero unique.Handle[T]
	return h == zero
}
`,
		"internal/service/example/resource.go": `package example
import (
	"unique"
	providerunique "` + providerModulePath + `/internal/unique"
)
func handler() bool { return providerunique.IsHandleNil(unique.Make("value")) }
`,
	}
	for path, source := range files {
		path = filepath.Join(dir, path)
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(source), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	index, err := loadProviderIndex(dir)
	if err != nil {
		t.Fatal(err)
	}
	pkg, err := packageForPath(index, providerModulePath+"/internal/service/example")
	if err != nil {
		t.Fatal(err)
	}
	roots, err := ssaFunctionsForObject(index, pkg.Types.Scope().Lookup("handler").(*types.Func), "handler")
	if err != nil {
		t.Fatal(err)
	}
	analyzer := newAPIMethodAnalyzer(index, nil, pkg.PkgPath)
	if _, err := analyzer.collect(roots); err != nil {
		t.Fatal(err)
	}
	for fn, summary := range analyzer.summaries {
		if fn.Package() != nil && fn.Package().Pkg.Path() == providerModulePath+"/internal/unique" &&
			fn.Syntax() != nil && len(fn.Blocks) != 0 && summary.state == apiMethodSummaryComplete {
			return
		}
	}
	t.Fatal("generic provider helper body was not traversed")
}
