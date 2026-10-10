package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"go/types"
	"slices"
	"testing"

	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
)

func TestServicePackageDispatchKeepsSharedHelpersScoped(t *testing.T) {
	t.Parallel()
	index, handler, mappings := servicePackageDispatchFixture(t)
	// Reuse the index and shared handler in both orders to exercise the
	// interface cache and each service's transitive function summaries.
	for _, service := range []string{"ec2", "lambda", "ec2", "notags", "lambda"} {
		analyzer := newAPIMethodAnalyzer(index, mappings, providerModulePath+"/internal/service/"+service)
		for range 2 {
			analysis, err := analyzer.collect([]*ssa.Function{handler})
			if err != nil {
				t.Fatal(err)
			}
			want := []apiMethod{{Service: "sts", Name: "GetCallerIdentity"}}
			switch service {
			case "ec2":
				want = append([]apiMethod{{Service: "ec2", Name: "DescribeTags"}}, want...)
			case "lambda":
				want = append([]apiMethod{{Service: "lambda", Name: "ListTags"}}, want...)
			}
			if !slices.Equal(analysis.methods, want) {
				t.Fatalf("%s methods = %#v, want %#v", service, analysis.methods, want)
			}
		}
	}
}

func TestServicePackageDispatchWithoutContextKeepsAllImplementations(t *testing.T) {
	t.Parallel()
	index, handler, mappings := servicePackageDispatchFixture(t)
	analysis, err := collectAPIMethods(index, []*ssa.Function{handler}, mappings)
	if err != nil {
		t.Fatal(err)
	}
	want := []apiMethod{
		{Service: "ec2", Name: "DescribeTags"},
		{Service: "lambda", Name: "ListTags"},
		{Service: "sts", Name: "GetCallerIdentity"},
	}
	if !slices.Equal(analysis.methods, want) {
		t.Fatalf("unscoped methods = %#v, want %#v", analysis.methods, want)
	}
}

func servicePackageDispatchFixture(t *testing.T) (*packageIndex, *ssa.Function, map[sdkMethodKey][]apiMethod) {
	t.Helper()
	const sdkPath = "github.com/aws/aws-sdk-go/service/example"
	sdkPkg := types.NewPackage(sdkPath, "example")
	client := types.NewNamed(types.NewTypeName(token.NoPos, sdkPkg, "Client", nil), types.NewStruct(nil, nil), nil)
	sdkPkg.Scope().Insert(client.Obj())
	for _, name := range []string{"ListThings", "GetThings", "DeleteThings"} {
		client.AddMethod(newMethod(sdkPkg, client, name))
	}
	sdkPkg.MarkComplete()

	program := ssa.NewProgram(token.NewFileSet(), 0)
	program.CreatePackage(sdkPkg, nil, nil, true)
	index := &packageIndex{
		prog:        program,
		byTypes:     map[*types.Package]*packages.Package{},
		byPath:      map[string][]*packages.Package{},
		funcDecls:   map[*types.Func]*ast.FuncDecl{},
		ssaFuncs:    map[*types.Func][]*ssa.Function{},
		ssaBySyntax: map[ast.Node][]*ssa.Function{},
	}
	for _, fixture := range []struct{ path, source string }{
		{"internal/tags", `package tags
import example "github.com/aws/aws-sdk-go/service/example"
type TagLister interface { ListTags(*example.Client) }
func handler(value TagLister, client *example.Client) {
	value.ListTags(client)
	// A genuine call to another AWS service must survive service scoping.
	client.DeleteThings()
}`},
		{"internal/service/ec2", `package ec2
import example "github.com/aws/aws-sdk-go/service/example"
type servicePackage struct{}
func (*servicePackage) ListTags(client *example.Client) { client.ListThings() }
`},
		{"internal/service/lambda", `package lambda
import example "github.com/aws/aws-sdk-go/service/example"
type servicePackage struct{}
func (*servicePackage) ListTags(client *example.Client) { client.GetThings() }
`},
		{"internal/service/notags", "package notags\ntype servicePackage struct{}\n"},
	} {
		path := providerModulePath + "/" + fixture.path
		file, err := parser.ParseFile(program.Fset, "fixture.go", fixture.source, parser.SkipObjectResolution)
		if err != nil {
			t.Fatal(err)
		}
		info := &types.Info{
			Defs:       map[*ast.Ident]types.Object{},
			Uses:       map[*ast.Ident]types.Object{},
			Types:      map[ast.Expr]types.TypeAndValue{},
			Selections: map[*ast.SelectorExpr]*types.Selection{},
		}
		checked, err := (&types.Config{Importer: packageMapImporter{sdkPath: sdkPkg}}).Check(path, program.Fset, []*ast.File{file}, info)
		if err != nil {
			t.Fatal(err)
		}
		pkg := &packages.Package{Types: checked, TypesInfo: info}
		index.byTypes[checked] = pkg
		index.byPath[path] = []*packages.Package{pkg}
		program.CreatePackage(checked, []*ast.File{file}, info, true)
		for _, declaration := range file.Decls {
			if fn, ok := declaration.(*ast.FuncDecl); ok {
				obj := info.Defs[fn.Name].(*types.Func)
				index.funcDecls[obj] = fn
				ssaFn := program.FuncValue(obj)
				index.ssaFuncs[obj] = []*ssa.Function{ssaFn}
				index.ssaBySyntax[fn] = []*ssa.Function{ssaFn}
			}
		}
	}
	program.Build()
	pkg := index.byPath[providerModulePath+"/internal/tags"][0]
	handler := program.FuncValue(pkg.Types.Scope().Lookup("handler").(*types.Func))
	mappings := map[sdkMethodKey][]apiMethod{
		{pkg: sdkPath, receiver: "Client", method: "ListThings"}:   {{Service: "ec2", Name: "DescribeTags"}},
		{pkg: sdkPath, receiver: "Client", method: "GetThings"}:    {{Service: "lambda", Name: "ListTags"}},
		{pkg: sdkPath, receiver: "Client", method: "DeleteThings"}: {{Service: "sts", Name: "GetCallerIdentity"}},
	}
	return index, handler, mappings
}
