package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"go/types"
	"slices"
	"strings"
	"testing"

	"golang.org/x/tools/go/packages"
	"golang.org/x/tools/go/ssa"
	"golang.org/x/tools/go/ssa/ssautil"
)

func TestSDKKeyForCallable(t *testing.T) {
	t.Parallel()

	const ecrPackage = "github.com/aws/aws-sdk-go-v2/service/ecr"
	tests := []struct {
		name       string
		pkg        string
		receiver   string
		method     string
		expected   sdkMethodKey
		expectedOK bool
	}{
		{
			name:       "client method",
			pkg:        ecrPackage,
			receiver:   "Client",
			method:     "DescribeImages",
			expected:   sdkMethodKey{pkg: ecrPackage, receiver: "Client", method: "DescribeImages"},
			expectedOK: true,
		},
		{
			name:       "paginator constructor",
			pkg:        ecrPackage,
			method:     "NewDescribeImagesPaginator",
			expected:   sdkMethodKey{pkg: ecrPackage, method: "NewDescribeImagesPaginator"},
			expectedOK: true,
		},
		{
			name:       "non paginator function",
			pkg:        ecrPackage,
			method:     "NewFromConfig",
			expected:   sdkMethodKey{pkg: ecrPackage, method: "NewFromConfig"},
			expectedOK: true,
		},
		{
			name:       "nested package callable",
			pkg:        ecrPackage + "/types",
			method:     "NewDescribeImagesPaginator",
			expected:   sdkMethodKey{pkg: ecrPackage + "/types", method: "NewDescribeImagesPaginator"},
			expectedOK: true,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()
			actual, ok := sdkKeyForCallable(test.pkg, test.receiver, test.method)
			if ok != test.expectedOK {
				t.Fatalf("sdkKeyForCallable() ok = %v, want %v", ok, test.expectedOK)
			}
			if actual != test.expected {
				t.Fatalf("sdkKeyForCallable() = %#v, want %#v", actual, test.expected)
			}
		})
	}
}

func TestAWSV2OperationShapeControlsExactMapping(t *testing.T) {
	t.Parallel()

	pkg := types.NewPackage("github.com/aws/aws-sdk-go-v2/service/example", "example")
	client := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "Client", nil), types.NewStruct(nil, nil), nil)
	setIPAddressType := addAWSV2ClientOperation(pkg, client, "SetIpAddressType")
	unmappedSetHypotheticalOperation := addAWSV2ClientOperation(pkg, client, "SetHypotheticalOperation")
	paginator := newAWSV2PaginatorNextPage(pkg, "ListThingsPaginator")
	options := newAWSV2OptionsAccessor(pkg, client)
	mappings := map[sdkMethodKey][]apiMethod{
		mustSDKKey(t, setIPAddressType): []apiMethod{{Service: "elasticloadbalancingv2", Name: "SetIpAddressType"}},
		mustSDKKey(t, paginator):        []apiMethod{{Service: "example", Name: "ListThings"}},
		mustSDKKey(t, options):          []apiMethod{{Service: "example", Name: "OptionsMustNotMap"}},
	}

	tests := []struct {
		name    string
		method  *types.Func
		want    []apiMethod
		wantErr bool
	}{
		{name: "mapped Set API operation", method: setIPAddressType, want: mappings[mustSDKKey(t, setIPAddressType)]},
		{name: "mapped paginator next page", method: paginator, want: mappings[mustSDKKey(t, paginator)]},
		{name: "options accessor", method: options},
		{name: "unmapped Set-shaped operation", method: unmappedSetHypotheticalOperation, wantErr: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, err := apiMethodsForSDKCallable(test.method, mappings, true)
			if (err != nil) != test.wantErr {
				t.Fatalf("apiMethodsForSDKCallable(%s) error = %v, want error: %v", test.method.Name(), err, test.wantErr)
			}
			if !slices.Equal(got, test.want) {
				t.Fatalf("apiMethodsForSDKCallable(%s) = %#v, want %#v", test.method.Name(), got, test.want)
			}
		})
	}
}

func TestAWSV2SetterHelpersDoNotMapWithoutOperationShape(t *testing.T) {
	t.Parallel()

	pkg := types.NewPackage("github.com/aws/aws-sdk-go-v2/service/example", "example")
	client := types.NewNamed(types.NewTypeName(token.NoPos, pkg, "Client", nil), types.NewStruct(nil, nil), nil)
	for _, helperName := range []string{"SetFilter", "SetSAMLOptions", "SetEncryptionContextEquals"} {
		t.Run(helperName, func(t *testing.T) {
			helper := newMethod(pkg, client, helperName)
			mappings := map[sdkMethodKey][]apiMethod{
				mustSDKKey(t, helper): []apiMethod{{Service: "example", Name: helperName}},
			}
			got, err := apiMethodsForSDKCallable(helper, mappings, true)
			if err != nil {
				t.Fatalf("apiMethodsForSDKCallable(%s) error = %v", helperName, err)
			}
			if len(got) != 0 {
				t.Fatalf("apiMethodsForSDKCallable(%s) = %#v, want no API methods", helperName, got)
			}
		})
	}
}

func TestCollectAPIMethodsAllowsEmptyResolvedHandler(t *testing.T) {
	t.Parallel()

	program := ssa.NewProgram(token.NewFileSet(), 0)
	handler := program.NewFunction("handler", types.NewSignature(nil, types.NewTuple(), types.NewTuple(), false), "")
	analysis, err := collectAPIMethods(&packageIndex{}, []*ssa.Function{handler}, nil)
	if err != nil {
		t.Fatalf("collectAPIMethods() error = %v", err)
	}
	if len(analysis.methods) != 0 {
		t.Fatalf("collectAPIMethods() methods = %#v, want no API methods", analysis.methods)
	}

	output, err := json.Marshal(mappingRow{
		Kind:       "resource",
		TypeName:   "aws_example",
		Action:     "read",
		APIMethods: analysis.methods,
	})
	if err != nil {
		t.Fatalf("marshal empty handler row: %v", err)
	}
	const want = `{"kind":"resource","type_name":"aws_example","action":"read","api_methods":[]}`
	if string(output) != want {
		t.Fatalf("empty handler row = %s, want %s", output, want)
	}
}

func TestCollectAPIMethodsRejectsUnresolvedHandler(t *testing.T) {
	t.Parallel()

	if _, err := collectAPIMethods(&packageIndex{}, nil, nil); err == nil {
		t.Fatal("collectAPIMethods() accepted a handler with no resolved SSA functions")
	}
}

func TestAPIMethodAnalyzerMemoizesSharedFunctionSummaries(t *testing.T) {
	t.Parallel()

	index, handlers, mappings, sharedKey := sharedAPIMethodFixture(t)
	analyzer := newAPIMethodAnalyzer(index, mappings)
	want := []apiMethod{{Service: "example", Name: "ListThings"}}
	first, err := analyzer.collect([]*ssa.Function{handlers[0]})
	if err != nil {
		t.Fatalf("collect(%s) error = %v", handlers[0].Name(), err)
	}
	if !slices.Equal(first.methods, want) {
		t.Fatalf("collect(%s) methods = %#v, want %#v", handlers[0].Name(), first.methods, want)
	}

	delete(mappings, sharedKey)
	second, err := analyzer.collect([]*ssa.Function{handlers[1]})
	if err != nil {
		t.Fatalf("collect(%s) error = %v", handlers[1].Name(), err)
	}
	if !slices.Equal(second.methods, want) {
		t.Fatalf("collect(%s) methods = %#v, want %#v", handlers[1].Name(), second.methods, want)
	}
}

func TestCollectDirectLocalCalleesSkipsBodylessInterfaceMethod(t *testing.T) {
	t.Parallel()

	index, handler := localCalleeFixture(t, `
package flex

type NestedObjectCollectionValue interface {
	ToObjectSlice()
}

func handler(value NestedObjectCollectionValue) {
	value.ToObjectSlice()
}`)

	callees, err := collectDirectLocalCallees(index, handler)
	if err != nil {
		t.Fatalf("collectDirectLocalCallees() error = %v", err)
	}
	if len(callees) != 0 {
		t.Fatalf("collectDirectLocalCallees() = %#v, want no concrete callees", callees)
	}
}

func TestCollectDirectLocalCalleesResolvesNoExpandSelectionInExactSSATypeUniverse(t *testing.T) {
	t.Parallel()

	index, handlers, noExpands := duplicatePathLocalCalleeFixture(t)
	// Selection resolution must not depend on object-index recovery.
	index.funcDecls = nil
	index.ssaFuncs = nil
	index.ssaFuncsByIdentity = nil
	index.ssaBySyntax = nil
	for i, handler := range handlers {
		callees, err := collectDirectLocalCallees(index, handler)
		if err != nil {
			t.Fatalf("collectDirectLocalCallees(handler %d) error = %v", i, err)
		}
		if len(callees) != 1 || callees[0] != noExpands[i] {
			t.Fatalf("collectDirectLocalCallees(handler %d) = %#v, want exact NoExpand %#v", i, callees, noExpands[i])
		}
	}
}

func TestCollectDirectLocalCalleesUnwrapsPromotedPointerMethod(t *testing.T) {
	t.Parallel()

	index, handler := localCalleeFixture(t, `
package flex

type meta struct{}

func (*meta) Meta() {}

type withMeta struct {
	*meta
}

func handler(value *withMeta) {
	value.Meta()
}`)
	selection := methodSelectionForCall(t, index, handler, "Meta")
	wrapper := index.prog.MethodValue(selection)
	if wrapper == nil {
		t.Fatal("MethodValue(withMeta.Meta) is missing")
	}
	if wrapper.Syntax() != nil {
		t.Fatalf("MethodValue(withMeta.Meta) syntax = %T, want synthetic wrapper without syntax", wrapper.Syntax())
	}
	declared, ok := selection.Obj().(*types.Func)
	if !ok {
		t.Fatal("withMeta.Meta selection has no declared function")
	}
	body := index.prog.FuncValue(declared)
	if body == nil {
		t.Fatal("FuncValue(withMeta.Meta declaration) is missing")
	}

	callees, err := collectDirectLocalCallees(index, handler)
	if err != nil {
		t.Fatalf("collectDirectLocalCallees() error = %v", err)
	}
	if len(callees) != 1 || callees[0] != body {
		t.Fatalf("collectDirectLocalCallees() = %#v, want declared Meta body %#v", callees, body)
	}
	if decl, ok := callees[0].Syntax().(*ast.FuncDecl); !ok || decl.Name.Name != "Meta" {
		t.Fatalf("collectDirectLocalCallees() analyzed %T, want Meta declaration", callees[0].Syntax())
	}
}

func TestResolveConcreteTypeHandlersResolvesAccountAccessPromotedGenericUpdate(t *testing.T) {
	t.Parallel()

	index, pkg, factory, update, updateDecl, updateBody := concreteHandlerFixture(t, "application.go")
	concreteType, err := resolveReturnedConcreteType(index, pkg, factory)
	if err != nil {
		t.Fatalf("resolveReturnedConcreteType() error = %v", err)
	}
	named, err := namedType(concreteType)
	if err != nil {
		t.Fatalf("namedType() error = %v", err)
	}
	selection := types.NewMethodSet(types.NewPointer(named)).Lookup(nil, "Update")
	if selection == nil {
		t.Fatal("applicationResource Update method selection is missing")
	}
	selectedUpdate, ok := selection.Obj().(*types.Func)
	if !ok {
		t.Fatalf("applicationResource Update selection object = %T, want *types.Func", selection.Obj())
	}
	selectedIdentity, ok := canonicalFunctionIdentity(selectedUpdate)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(applicationResource.Update selection) failed")
	}
	updateIdentity, ok := canonicalFunctionIdentity(update)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(declared Update) failed")
	}
	if selectedIdentity != updateIdentity {
		t.Fatalf("applicationResource Update identity = %#v, want declared Update identity %#v", selectedIdentity, updateIdentity)
	}
	wrapper := index.prog.MethodValue(selection)
	if wrapper == nil || wrapper.Syntax() != nil {
		t.Fatalf("MethodValue(applicationResource.Update) = %#v, want synthetic promoted wrapper", wrapper)
	}

	handlers, err := resolveConcreteTypeHandlers(index, pkg, factory, []namedMethodAction{{"Update", "update"}})
	if err != nil {
		t.Fatalf("resolveConcreteTypeHandlers() error = %v", err)
	}
	if len(handlers) != 1 || handlers[0].action != "update" || len(handlers[0].funcs) != 1 {
		t.Fatalf("resolveConcreteTypeHandlers() = %#v, want one promoted update handler", handlers)
	}
	handlerBody := handlers[0].funcs[0]
	if handlerBody.Syntax() != updateDecl {
		t.Fatalf("resolveConcreteTypeHandlers() analyzed %T, want Update declaration", handlerBody.Syntax())
	}
	if ssaFunctionSourceIdentityFor(handlerBody, updateDecl, selectedIdentity) !=
		ssaFunctionSourceIdentityFor(updateBody, updateDecl, updateIdentity) {
		t.Fatal("resolveConcreteTypeHandlers() did not include the declared Update source body")
	}
}

func TestResolveConcreteTypeHandlersRejectsMissingPromotedGenericUpdateBody(t *testing.T) {
	t.Parallel()

	index, pkg, factory, _, _, _ := concreteHandlerFixture(t, "application.go")
	index.prog = ssa.NewProgram(token.NewFileSet(), 0)
	index.funcDecls = nil
	index.ssaFuncs = nil
	index.ssaFuncsByIdentity = nil
	index.ssaBySyntax = nil

	_, err := resolveConcreteTypeHandlers(index, pkg, factory, []namedMethodAction{{"Update", "update"}})
	if err == nil || !strings.Contains(err.Error(), "resolve update handler: SSA function missing for Update") {
		t.Fatalf("resolveConcreteTypeHandlers() error = %v, want missing promoted Update body", err)
	}
}

func TestResolveConcreteTypeHandlersRejectsAmbiguousPromotedGenericUpdateBody(t *testing.T) {
	t.Parallel()

	index, pkg, factory, update, updateDecl, updateBody := concreteHandlerFixture(t, "application.go")
	_, _, _, otherUpdate, otherUpdateDecl, otherUpdateBody := concreteHandlerFixture(t, "other_application.go")
	identity, ok := canonicalFunctionIdentity(update)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(Update) failed")
	}
	if otherIdentity, ok := canonicalFunctionIdentity(otherUpdate); !ok || otherIdentity != identity {
		t.Fatal("fixture Update methods do not share a canonical identity")
	}
	index.prog = ssa.NewProgram(token.NewFileSet(), 0)
	index.funcDecls = nil
	index.ssaFuncs = nil
	index.ssaFuncsByIdentity = nil
	index.funcDeclsByIdentity = map[functionIdentity][]*ast.FuncDecl{identity: {updateDecl, otherUpdateDecl}}
	if sourceDeclarationIdentityFor(index, updateDecl, identity) ==
		sourceDeclarationIdentityFor(index, otherUpdateDecl, identity) {
		t.Fatal("colliding Update declarations share a source identity")
	}
	index.ssaBySyntax = map[ast.Node][]*ssa.Function{
		updateDecl:      {updateBody},
		otherUpdateDecl: {otherUpdateBody},
	}

	_, err := resolveConcreteTypeHandlers(index, pkg, factory, []namedMethodAction{{"Update", "update"}})
	if err == nil || !strings.Contains(err.Error(), "resolve update handler: SSA function resolution ambiguous for concrete Update") {
		t.Fatalf("resolveConcreteTypeHandlers() error = %v, want ambiguous promoted Update body", err)
	}
}

func TestDirectSSAFunctionsForSelectionRejectsMissingUnderlyingBody(t *testing.T) {
	t.Parallel()

	index, handler := localCalleeFixture(t, `
package flex

type tagOptions struct{}

func (tagOptions) NoExpand() {}

func handler() {
	tagOptions{}.NoExpand()
}`)
	index.prog = ssa.NewProgram(token.NewFileSet(), 0)

	_, found, err := directSSAFunctionsForSelection(index, methodSelectionForCall(t, index, handler, "NoExpand"), "NoExpand")
	if !found || err == nil || !strings.Contains(err.Error(), "SSA function missing for NoExpand") {
		t.Fatalf("directSSAFunctionsForSelection() = found %t, error %v, want missing underlying body", found, err)
	}
}

func TestDirectSSAFunctionsForSelectionRejectsAmbiguousUnderlyingBody(t *testing.T) {
	t.Parallel()

	origin, _, decl, body := concreteNoExpandFixture(t)
	otherOrigin, _, otherDecl, otherBody := concreteNoExpandFixtureAt(t, "other_tags.go")
	identity, ok := canonicalFunctionIdentity(origin)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(NoExpand) failed")
	}
	if otherIdentity, ok := canonicalFunctionIdentity(otherOrigin); !ok || otherIdentity != identity {
		t.Fatal("fixture NoExpand methods do not share a canonical identity")
	}
	index := &packageIndex{
		prog: ssa.NewProgram(token.NewFileSet(), 0),
		funcDeclsByIdentity: map[functionIdentity][]*ast.FuncDecl{
			identity: {decl, otherDecl},
		},
		ssaBySyntax: map[ast.Node][]*ssa.Function{
			decl:      {body},
			otherDecl: {otherBody},
		},
	}

	_, found, err := directSSAFunctionsForSelection(index, methodSelectionForDeclaredMethod(t, origin), "NoExpand")
	if !found || err == nil || !strings.Contains(err.Error(), "ambiguous for concrete NoExpand") {
		t.Fatalf("directSSAFunctionsForSelection() = found %t, error %v, want ambiguous underlying body", found, err)
	}
}

func TestCollectDirectLocalCalleesRejectsMissingConcreteMethodSelection(t *testing.T) {
	t.Parallel()

	index, handler := localCalleeFixture(t, `
package flex

type tagOptions struct{}

func (tagOptions) NoExpand() {}

func handler() {
	tagOptions{}.NoExpand()
}`)
	index.prog = nil

	_, err := collectDirectLocalCallees(index, handler)
	if err == nil || !strings.Contains(err.Error(), "SSA function missing for") {
		t.Fatalf("collectDirectLocalCallees() error = %v, want missing concrete SSA error", err)
	}
}

func TestCollectDirectLocalCalleesSkipsBodylessLinkname(t *testing.T) {
	t.Parallel()

	index, handler := localCalleeFixture(t, `
package flex


//go:linkname external example.com/external.symbol
func external()

func handler() {
	external()
}`)

	callees, err := collectDirectLocalCallees(index, handler)
	if err != nil {
		t.Fatalf("collectDirectLocalCallees() error = %v", err)
	}
	if len(callees) != 0 {
		t.Fatalf("collectDirectLocalCallees() returned %d callees, want 0", len(callees))
	}
}

func TestSSAFunctionsForObjectRecoversNoExpandAcrossUniversesAndGenericInstances(t *testing.T) {
	t.Parallel()

	// A package reload gives the same declaration a different *types.Func
	// pointer from the one retained by SSA.
	origin, lookup, noExpandDecl, noExpandBody := concreteNoExpandFixture(t)
	originIdentity, ok := canonicalFunctionIdentity(origin)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(indexed NoExpand) failed")
	}
	lookupIdentity, ok := canonicalFunctionIdentity(lookup)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(lookup NoExpand) failed")
	}
	if originIdentity != lookupIdentity {
		t.Fatalf("NoExpand identities differ: indexed %#v, lookup %#v", originIdentity, lookupIdentity)
	}
	if originIdentity.receiver != providerModulePath+"/internal/framework/flex.tagOptions" {
		t.Fatalf("NoExpand receiver identity = %q, want package-qualified tagOptions", originIdentity.receiver)
	}

	duplicateOrigin, _, duplicateDecl, duplicateBody := concreteNoExpandFixture(t)
	duplicateIdentity, ok := canonicalFunctionIdentity(duplicateOrigin)
	if !ok || duplicateIdentity != originIdentity {
		t.Fatal("duplicate NoExpand fixture does not share the canonical identity")
	}
	if ssaFunctionSourceIdentityFor(noExpandBody, noExpandDecl, originIdentity) !=
		ssaFunctionSourceIdentityFor(duplicateBody, duplicateDecl, duplicateIdentity) {
		t.Fatal("duplicate-universe NoExpand bodies do not share a source identity")
	}
	if _, err := canonicalConcreteSSAFunctions([]*ssa.Function{noExpandBody, duplicateBody}, "NoExpand"); err != nil {
		t.Fatalf("canonicalConcreteSSAFunctions(NoExpand) error = %v", err)
	}

	some, someDecl, someFunctions := genericSomeFixture(t)
	someIdentity, ok := canonicalFunctionIdentity(some)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(Some) failed")
	}
	index := &packageIndex{
		funcDecls: map[*types.Func]*ast.FuncDecl{
			origin: noExpandDecl,
			some:   someDecl,
		},
		funcDeclsByIdentity: map[functionIdentity][]*ast.FuncDecl{
			originIdentity: {noExpandDecl},
			someIdentity:   {someDecl},
		},
		ssaFuncs: map[*types.Func][]*ssa.Function{
			origin: {noExpandBody},
		},
		ssaFuncsByIdentity: map[functionIdentity][]*ssa.Function{
			originIdentity: {noExpandBody},
		},
		ssaBySyntax: map[ast.Node][]*ssa.Function{
			noExpandDecl: {noExpandBody},
			someDecl:     someFunctions,
		},
	}
	for _, fn := range someFunctions {
		obj, ok := fn.Object().(*types.Func)
		if !ok {
			continue
		}
		index.ssaFuncs[obj] = append(index.ssaFuncs[obj], fn)
		if identity, ok := canonicalFunctionIdentity(obj); ok {
			index.ssaFuncsByIdentity[identity] = append(index.ssaFuncsByIdentity[identity], fn)
		}
	}
	if index.funcDecls[lookup] != nil || len(index.ssaFuncs[lookup]) != 0 {
		t.Fatal("fixture lookup unexpectedly shares the indexed function object")
	}

	noExpandFunctions, err := ssaFunctionsForObject(index, lookup, "NoExpand")
	if err != nil {
		t.Fatalf("ssaFunctionsForObject(NoExpand) error = %v", err)
	}
	if len(noExpandFunctions) != 1 || noExpandFunctions[0] != noExpandBody {
		t.Fatalf("ssaFunctionsForObject(NoExpand) = %#v, want exact body %#v", noExpandFunctions, noExpandBody)
	}

	someFunctions, err = ssaFunctionsForObject(index, some, "Some")
	if err != nil {
		t.Fatalf("ssaFunctionsForObject(Some) error = %v", err)
	}
	assertSomeGenericInstantiations(t, someFunctions, someDecl)
}

func TestSSAFunctionsForObjectAcceptsSomeGenericInstantiations(t *testing.T) {
	t.Parallel()

	some, decl, instantiated := genericSomeFixture(t)
	identity, ok := canonicalFunctionIdentity(some)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(Some) failed")
	}
	index := &packageIndex{
		funcDecls:           map[*types.Func]*ast.FuncDecl{some: decl},
		funcDeclsByIdentity: map[functionIdentity][]*ast.FuncDecl{identity: {decl}},
		ssaFuncs:            map[*types.Func][]*ssa.Function{},
		ssaFuncsByIdentity:  map[functionIdentity][]*ssa.Function{},
		ssaBySyntax:         map[ast.Node][]*ssa.Function{decl: instantiated},
	}
	for _, fn := range instantiated {
		obj, ok := fn.Object().(*types.Func)
		if !ok {
			continue
		}
		index.ssaFuncs[obj] = append(index.ssaFuncs[obj], fn)
		if functionIdentity, ok := canonicalFunctionIdentity(obj); ok {
			index.ssaFuncsByIdentity[functionIdentity] = append(index.ssaFuncsByIdentity[functionIdentity], fn)
		}
	}

	funcs, err := ssaFunctionsForObject(index, some, "Some")
	if err != nil {
		t.Fatalf("ssaFunctionsForObject(Some) error = %v", err)
	}
	assertSomeGenericInstantiations(t, funcs, decl)
}

func genericSomeFixture(t *testing.T) (*types.Func, *ast.FuncDecl, []*ssa.Function) {
	t.Helper()

	const packagePath = providerModulePath + "/internal/types/option"
	const source = `package option

type Option[T any] []T

const value = iota

func Some[T any](v T) Option[T] {
	return Option[T]{
		value: v,
	}
}

func stringOption() {
	_ = Some[string]("value")
}

func integerOption() {
	_ = Some[int](1)
}

func booleanOption() {
	_ = Some[bool](true)
}`
	fileSet := token.NewFileSet()
	file, err := parser.ParseFile(fileSet, "option.go", source, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse fixture: %v", err)
	}
	info := &types.Info{
		Defs:       map[*ast.Ident]types.Object{},
		Instances:  map[*ast.Ident]types.Instance{},
		Types:      map[ast.Expr]types.TypeAndValue{},
		Uses:       map[*ast.Ident]types.Object{},
		Selections: map[*ast.SelectorExpr]*types.Selection{},
	}
	checked, err := (&types.Config{}).Check(packagePath, fileSet, []*ast.File{file}, info)
	if err != nil {
		t.Fatalf("type-check fixture: %v", err)
	}
	some, decl := functionDeclaration(t, file, info, "Some")

	program := ssa.NewProgram(fileSet, ssa.InstantiateGenerics)
	ssaPackage := program.CreatePackage(checked, []*ast.File{file}, info, true)
	ssaPackage.Build()
	instantiated := make([]*ssa.Function, 0, 3)
	for fn := range ssautil.AllFunctions(program) {
		if fn != nil && fn.Syntax() == decl && len(fn.TypeArgs()) != 0 {
			instantiated = append(instantiated, fn)
		}
	}
	return some, decl, instantiated
}

func assertSomeGenericInstantiations(t *testing.T, funcs []*ssa.Function, decl *ast.FuncDecl) {
	t.Helper()

	if len(funcs) != 3 {
		t.Fatalf("ssaFunctionsForObject(Some) resolved %d functions, want 3", len(funcs))
	}
	typeArgs := make([]string, 0, len(funcs))
	for _, fn := range funcs {
		if fn.Syntax() != decl {
			t.Fatalf("ssaFunctionsForObject(Some) returned function from %T, want Some declaration", fn.Syntax())
		}
		if fn.Origin() == nil || fn.Origin().Syntax() != decl {
			t.Fatalf("ssaFunctionsForObject(Some) returned non-instantiation %s", fn)
		}
		if len(fn.TypeArgs()) != 1 {
			t.Fatalf("ssaFunctionsForObject(Some) returned %s with %d type arguments, want 1", fn, len(fn.TypeArgs()))
		}
		typeArgs = append(typeArgs, types.TypeString(fn.TypeArgs()[0], nil))
	}
	slices.Sort(typeArgs)
	if !slices.Equal(typeArgs, []string{"bool", "int", "string"}) {
		t.Fatalf("ssaFunctionsForObject(Some) type arguments = %v, want [bool int string]", typeArgs)
	}
}

func TestSSAFunctionsForObjectRejectsMissingConcreteMethodRecovery(t *testing.T) {
	t.Parallel()

	_, lookup, _, _ := concreteNoExpandFixture(t)
	_, err := ssaFunctionsForObject(&packageIndex{}, lookup, "NoExpand")
	if err == nil || !strings.Contains(err.Error(), "SSA function missing for NoExpand") {
		t.Fatalf("ssaFunctionsForObject(NoExpand) error = %v, want missing concrete SSA error", err)
	}
}

func TestSSAFunctionsForObjectRejectsDistinctSourceDeclarations(t *testing.T) {
	t.Parallel()

	origin, _, _, body := concreteNoExpandFixture(t)
	_, _, _, otherBody := concreteNoExpandFixtureAt(t, "other_tags.go")
	index := &packageIndex{
		ssaFuncs: map[*types.Func][]*ssa.Function{origin: {body, otherBody}},
	}

	_, err := ssaFunctionsForObject(index, origin, "NoExpand")
	if err == nil || !strings.Contains(err.Error(), "ambiguous for concrete NoExpand") {
		t.Fatalf("ssaFunctionsForObject(NoExpand) error = %v, want distinct declaration ambiguity", err)
	}
}

func TestSSAFunctionsForObjectRejectsAmbiguousConcreteMethodRecovery(t *testing.T) {
	t.Parallel()

	origin, lookup, decl, body := concreteNoExpandFixture(t)
	otherOrigin, _, otherDecl, otherBody := concreteNoExpandFixtureAt(t, "other_tags.go")
	identity, ok := canonicalFunctionIdentity(origin)
	if !ok {
		t.Fatal("canonicalFunctionIdentity(NoExpand) failed")
	}
	otherIdentity, ok := canonicalFunctionIdentity(otherOrigin)
	if !ok || otherIdentity != identity {
		t.Fatal("fixture NoExpand methods do not share a canonical identity")
	}
	index := &packageIndex{
		funcDeclsByIdentity: map[functionIdentity][]*ast.FuncDecl{identity: {decl, otherDecl}},
		ssaFuncs:            map[*types.Func][]*ssa.Function{},
		ssaBySyntax: map[ast.Node][]*ssa.Function{
			decl:      {body},
			otherDecl: {otherBody},
		},
	}

	_, err := ssaFunctionsForObject(index, lookup, "NoExpand")
	if err == nil || !strings.Contains(err.Error(), "ambiguous for concrete NoExpand") {
		t.Fatalf("ssaFunctionsForObject(NoExpand) error = %v, want ambiguous concrete SSA error", err)
	}
}

func concreteNoExpandFixture(t *testing.T) (*types.Func, *types.Func, *ast.FuncDecl, *ssa.Function) {
	t.Helper()
	return concreteNoExpandFixtureAt(t, "tags.go")
}

func concreteNoExpandFixtureAt(t *testing.T, sourceFile string) (*types.Func, *types.Func, *ast.FuncDecl, *ssa.Function) {
	t.Helper()

	const packagePath = providerModulePath + "/internal/framework/flex"
	const source = `package flex

type tagOptions string

func (o tagOptions) Contains(option string) bool {
	return string(o) == option
}

func (o tagOptions) NoExpand() bool {
	return o.Contains("noexpand")
}`
	originFileSet := token.NewFileSet()
	originFile, err := parser.ParseFile(originFileSet, sourceFile, source, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse origin fixture: %v", err)
	}
	originInfo := &types.Info{
		Defs:       map[*ast.Ident]types.Object{},
		Types:      map[ast.Expr]types.TypeAndValue{},
		Uses:       map[*ast.Ident]types.Object{},
		Selections: map[*ast.SelectorExpr]*types.Selection{},
	}
	originPackage, err := (&types.Config{}).Check(packagePath, originFileSet, []*ast.File{originFile}, originInfo)
	if err != nil {
		t.Fatalf("type-check origin fixture: %v", err)
	}
	origin, decl := functionDeclaration(t, originFile, originInfo, "NoExpand")

	program := ssa.NewProgram(originFileSet, 0)
	ssaPackage := program.CreatePackage(originPackage, []*ast.File{originFile}, originInfo, true)
	ssaPackage.Build()
	signature, ok := origin.Type().(*types.Signature)
	if !ok || signature.Recv() == nil {
		t.Fatal("fixture NoExpand is not a method")
	}
	selection := types.NewMethodSet(signature.Recv().Type()).Lookup(originPackage, origin.Name())
	if selection == nil {
		t.Fatal("fixture NoExpand method selection is missing")
	}
	body := program.MethodValue(selection)
	if body == nil || body.Syntax() != decl {
		t.Fatal("fixture NoExpand SSA body does not retain its declaration syntax")
	}

	lookupFileSet := token.NewFileSet()
	lookupFile, err := parser.ParseFile(lookupFileSet, sourceFile, source, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse lookup fixture: %v", err)
	}
	lookupInfo := &types.Info{
		Defs:       map[*ast.Ident]types.Object{},
		Types:      map[ast.Expr]types.TypeAndValue{},
		Uses:       map[*ast.Ident]types.Object{},
		Selections: map[*ast.SelectorExpr]*types.Selection{},
	}
	if _, err := (&types.Config{}).Check(packagePath, lookupFileSet, []*ast.File{lookupFile}, lookupInfo); err != nil {
		t.Fatalf("type-check lookup fixture: %v", err)
	}
	lookup, _ := functionDeclaration(t, lookupFile, lookupInfo, "NoExpand")
	return origin, lookup, decl, body
}

func functionDeclaration(t *testing.T, file *ast.File, info *types.Info, name string) (*types.Func, *ast.FuncDecl) {
	t.Helper()

	for _, declaration := range file.Decls {
		decl, ok := declaration.(*ast.FuncDecl)
		if !ok || decl.Name.Name != name {
			continue
		}
		obj, ok := info.Defs[decl.Name].(*types.Func)
		if !ok {
			t.Fatalf("fixture declaration %s is not a function", name)
		}
		return obj, decl
	}
	t.Fatalf("fixture has no %s declaration", name)
	return nil, nil
}

func newMethod(pkg *types.Package, receiver *types.Named, name string) *types.Func {
	recv := types.NewVar(token.NoPos, pkg, "", types.NewPointer(receiver))
	signature := types.NewSignature(recv, types.NewTuple(), types.NewTuple(), false)
	return types.NewFunc(token.NoPos, pkg, name, signature)
}

func mustSDKKey(t *testing.T, method *types.Func) sdkMethodKey {
	t.Helper()

	key, ok := sdkKeyForObject(method)
	if !ok {
		t.Fatalf("sdkKeyForObject(%s) failed", method.Name())
	}
	return key
}

func addAWSV2ClientOperation(pkg *types.Package, client *types.Named, operation string) *types.Func {
	method := newAWSV2ClientOperation(pkg, client, operation)
	client.AddMethod(method)
	return method
}

func newAWSV2ClientOperation(pkg *types.Package, client *types.Named, operation string) *types.Func {
	recv := types.NewVar(token.NoPos, pkg, "", types.NewPointer(client))
	params := types.NewTuple(
		types.NewVar(token.NoPos, pkg, "ctx", awsV2ContextType()),
		types.NewVar(token.NoPos, pkg, "params", types.NewPointer(awsV2NamedStruct(pkg, operation+"Input"))),
		types.NewVar(token.NoPos, pkg, "optFns", awsV2OptionsVariadicType(pkg)),
	)
	results := types.NewTuple(
		types.NewVar(token.NoPos, pkg, "", types.NewPointer(awsV2NamedStruct(pkg, operation+"Output"))),
		types.NewVar(token.NoPos, nil, "", types.Universe.Lookup("error").Type()),
	)
	return types.NewFunc(token.NoPos, pkg, operation, types.NewSignature(recv, params, results, true))
}

func newAWSV2PaginatorNextPage(pkg *types.Package, paginator string) *types.Func {
	receiver := awsV2NamedStruct(pkg, paginator)
	operation, _ := strings.CutSuffix(paginator, "Paginator")
	recv := types.NewVar(token.NoPos, pkg, "", types.NewPointer(receiver))
	params := types.NewTuple(
		types.NewVar(token.NoPos, pkg, "ctx", awsV2ContextType()),
		types.NewVar(token.NoPos, pkg, "optFns", awsV2OptionsVariadicType(pkg)),
	)
	results := types.NewTuple(
		types.NewVar(token.NoPos, pkg, "", types.NewPointer(awsV2NamedStruct(pkg, operation+"Output"))),
		types.NewVar(token.NoPos, nil, "", types.Universe.Lookup("error").Type()),
	)
	return types.NewFunc(token.NoPos, pkg, "NextPage", types.NewSignature(recv, params, results, true))
}

func newAWSV2OptionsAccessor(pkg *types.Package, client *types.Named) *types.Func {
	recv := types.NewVar(token.NoPos, pkg, "", types.NewPointer(client))
	results := types.NewTuple(types.NewVar(token.NoPos, pkg, "", awsV2NamedStruct(pkg, "Options")))
	return types.NewFunc(token.NoPos, pkg, "Options", types.NewSignature(recv, types.NewTuple(), results, false))
}

func awsV2ContextType() types.Type {
	contextPackage := types.NewPackage("context", "context")
	return types.NewNamed(
		types.NewTypeName(token.NoPos, contextPackage, "Context", nil),
		types.NewInterfaceType(nil, nil).Complete(),
		nil,
	)
}

func awsV2OptionsVariadicType(pkg *types.Package) types.Type {
	option := types.NewSignature(
		nil,
		types.NewTuple(types.NewVar(token.NoPos, pkg, "", types.NewPointer(awsV2NamedStruct(pkg, "Options")))),
		types.NewTuple(),
		false,
	)
	return types.NewSlice(option)
}

func awsV2NamedStruct(pkg *types.Package, name string) *types.Named {
	return types.NewNamed(types.NewTypeName(token.NoPos, pkg, name, nil), types.NewStruct(nil, nil), nil)
}

func TestExtractSpecsFromServiceMethodParsesRegistryLiterals(t *testing.T) {
	t.Parallel()

	const fixturePackagePath = "example.com/service"
	const sourceFile = "service_package_gen.go"
	const declarations = `
package service

type servicePackage struct{}

type ServicePackageSDKResource struct {
	Factory  func()
	TypeName string
}

func resourceAnalyzer() {}
func resourceLegacy() {}
`
	tests := []struct {
		name            string
		method          string
		allowTypeErrors bool
		want            []entrypointSpec
		wantErr         string
	}{
		{
			name: "map registry",
			method: `
type resourceRegistry map[string]func()

func (p *servicePackage) SDKResources() resourceRegistry {
	return resourceRegistry{
		"aws_legacy_analyzer": resourceLegacy,
	}
}`,
			want: []entrypointSpec{{
				kind:        "resource",
				typeName:    "aws_legacy_analyzer",
				factory:     "resourceLegacy",
				sourceFile:  sourceFile,
				packagePath: fixturePackagePath,
			}},
		},
		{
			name: "generated slice registry",
			method: `
func (p *servicePackage) SDKResources() []*ServicePackageSDKResource {
	return []*ServicePackageSDKResource{
		{
			Factory:  resourceAnalyzer,
			TypeName: "aws_accessanalyzer_analyzer",
		},
	}
}`,
			want: []entrypointSpec{{
				kind:        "resource",
				typeName:    "aws_accessanalyzer_analyzer",
				factory:     "resourceAnalyzer",
				sourceFile:  sourceFile,
				packagePath: fixturePackagePath,
			}},
		},
		{
			name: "pointer composite item",
			method: `
func (p *servicePackage) SDKResources() []*ServicePackageSDKResource {
	return []*ServicePackageSDKResource{
		&ServicePackageSDKResource{
			Factory:  resourceAnalyzer,
			TypeName: "aws_accessanalyzer_pointer",
		},
	}
}`,
			want: []entrypointSpec{{
				kind:        "resource",
				typeName:    "aws_accessanalyzer_pointer",
				factory:     "resourceAnalyzer",
				sourceFile:  sourceFile,
				packagePath: fixturePackagePath,
			}},
		},
		{
			name: "empty map registry",
			method: `
func (p *servicePackage) SDKResources() map[string]func() {
	return map[string]func(){}
}`,
			wantErr: "service package method SDKResources yielded no entrypoints",
		},
		{
			name: "empty generated slice registry",
			method: `
func (p *servicePackage) SDKResources() []*ServicePackageSDKResource {
	return []*ServicePackageSDKResource{}
}`,
			wantErr: "service package method SDKResources yielded no entrypoints",
		},
		{
			name:            "map with a list item",
			allowTypeErrors: true,
			method: `
func (p *servicePackage) SDKResources() map[string]func() {
	return map[string]func(){
		"aws_legacy_analyzer": resourceLegacy,
		{Factory: resourceAnalyzer, TypeName: "aws_accessanalyzer_analyzer"},
	}
}`,
			wantErr: "entrypoint map contains *ast.CompositeLit, not a key/value entry",
		},
		{
			name:            "list with a map item",
			allowTypeErrors: true,
			method: `
func (p *servicePackage) SDKResources() []*ServicePackageSDKResource {
	return []*ServicePackageSDKResource{
		"aws_accessanalyzer_analyzer": resourceAnalyzer,
	}
}`,
			wantErr: "entrypoint list contains *ast.KeyValueExpr, not a composite literal",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			pkg, method := serviceMethodFixture(t, fixturePackagePath, declarations+test.method, test.allowTypeErrors)
			got, err := extractSpecsFromServiceMethod(nil, pkg, method, "resource", sourceFile)
			if test.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), test.wantErr) {
					t.Fatalf("extractSpecsFromServiceMethod() error = %v, want %q", err, test.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("extractSpecsFromServiceMethod() error = %v", err)
			}
			if !slices.Equal(got, test.want) {
				t.Fatalf("extractSpecsFromServiceMethod() = %#v, want %#v", got, test.want)
			}
		})
	}
}

func serviceMethodFixture(t *testing.T, packagePath, source string, allowTypeErrors bool) (*packages.Package, *ast.FuncDecl) {
	t.Helper()

	fileSet := token.NewFileSet()
	file, err := parser.ParseFile(fileSet, "service_package_gen.go", source, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse fixture: %v", err)
	}
	info := &types.Info{
		Defs:  make(map[*ast.Ident]types.Object),
		Types: make(map[ast.Expr]types.TypeAndValue),
		Uses:  make(map[*ast.Ident]types.Object),
	}
	checked, err := (&types.Config{}).Check(packagePath, fileSet, []*ast.File{file}, info)
	if err != nil && !allowTypeErrors {
		t.Fatalf("type-check fixture: %v", err)
	}
	for _, declaration := range file.Decls {
		method, ok := declaration.(*ast.FuncDecl)
		if ok && method.Name.Name == "SDKResources" {
			return &packages.Package{Types: checked, TypesInfo: info}, method
		}
	}
	t.Fatal("fixture has no SDKResources method")
	return nil, nil
}

func TestDiscoverEntrypointsParsesFrameworkFactoryRegistrations(t *testing.T) {
	const registry = `package example

func registerFrameworkResourceFactory(factory func() any) {}
func registerFrameworkDataSourceFactory(factory func() any) {}
`
	const factories = `package example

type metadataResponse struct {
	TypeName string
}

func init() {
	registerFrameworkResourceFactory(newResourceExample)
	registerFrameworkDataSourceFactory(newDataSourceExample)
}

func newResourceExample() any {
	return &resourceExample{}
}

type resourceExample struct{}

func (*resourceExample) Metadata(response *metadataResponse) {
	response.TypeName = "aws_example_resource"
}

func newDataSourceExample() any {
	return &dataSourceExample{}
}

type dataSourceExample struct{}

func (*dataSourceExample) Metadata(response *metadataResponse) {
	response.TypeName = "aws_example_data_source"
}
`
	tests := []struct {
		name            string
		factories       string
		allowTypeErrors bool
		want            []entrypointSpec
		wantErr         string
	}{
		{
			name:      "v4.36 factory registrations",
			factories: factories,
			want: []entrypointSpec{
				{
					kind:        "data_source",
					typeName:    "aws_example_data_source",
					factory:     "newDataSourceExample",
					sourceFile:  providerModulePath + "/internal/service/example/registry.go",
					packagePath: providerModulePath + "/internal/service/example",
				},
				{
					kind:        "resource",
					typeName:    "aws_example_resource",
					factory:     "newResourceExample",
					sourceFile:  providerModulePath + "/internal/service/example/registry.go",
					packagePath: providerModulePath + "/internal/service/example",
				},
			},
		},
		{
			name:      "zero registrations",
			factories: "package example\n\nfunc init() {}\n",
			wantErr:   "service package entrypoint files yielded no entrypoints",
		},
		{
			name: "nested registration is not a direct init entrypoint",
			factories: `package example

func init() {
	if true {
		registerFrameworkResourceFactory(nil)
	}
}
`,
			wantErr: "service package entrypoint files yielded no entrypoints",
		},
		{
			name:            "malformed registration",
			factories:       "package example\n\nfunc init() { registerFrameworkResourceFactory() }\n",
			allowTypeErrors: true,
			wantErr:         "resource registry call registerFrameworkResourceFactory has 0 factory arguments, want 1",
		},
		{
			name: "malformed metadata type name",
			factories: `package example

const resourceTypeName = "aws_example_resource"

func init() {
	registerFrameworkResourceFactory(newResourceExample)
}

func newResourceExample() any {
	return &resourceExample{}
}

type resourceExample struct{}

type metadataResponse struct {
	TypeName string
}

func (*resourceExample) Metadata(response *metadataResponse) {
	response.TypeName = resourceTypeName
}
`,
			wantErr: "Metadata for newResourceExample: TypeName assignment: expected string literal",
		},
		{
			name: "other TypeName assignment is not metadata response",
			factories: `package example

func init() {
	registerFrameworkResourceFactory(newResourceExample)
}

func newResourceExample() any {
	return &resourceExample{}
}

type resourceExample struct{}

type metadataResponse struct {
	TypeName string
}

func (*resourceExample) Metadata(response *metadataResponse) {
	other := &metadataResponse{}
	other.TypeName = "aws_example_resource"
}
`,
			wantErr: "Metadata for newResourceExample: has 0 response TypeName assignments, want 1",
		},
		{
			name: "multiple metadata response type names",
			factories: `package example

func init() {
	registerFrameworkResourceFactory(newResourceExample)
}

func newResourceExample() any {
	return &resourceExample{}
}

type resourceExample struct{}

type metadataResponse struct {
	TypeName string
}

func (*resourceExample) Metadata(response *metadataResponse) {
	response.TypeName = "aws_example_resource"
	response.TypeName = "aws_other_resource"
}
`,
			wantErr: "Metadata for newResourceExample: has 2 response TypeName assignments, want 1",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			index := servicePackageRegistryFixture(t, registry, test.factories, "", test.allowTypeErrors)
			got, err := discoverEntrypoints(index)
			if test.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), test.wantErr) {
					t.Fatalf("discoverEntrypoints() error = %v, want %q", err, test.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("discoverEntrypoints() error = %v", err)
			}
			if !slices.Equal(got, test.want) {
				t.Fatalf("discoverEntrypoints() = %#v, want %#v", got, test.want)
			}
		})
	}
}

func TestDiscoverEntrypointsParsesGeneratedFrameworkDataSourceRegistrations(t *testing.T) {
	const registry = `package example

type dataSourceWithConfigure interface{}

type servicePackage struct {
	frameworkDataSourceFactories []func(any) (dataSourceWithConfigure, error)
}

func (p *servicePackage) FrameworkDataSources(any) []func(any) (dataSourceWithConfigure, error) {
	return p.frameworkDataSourceFactories
}

func (p *servicePackage) registerFrameworkDataSourceFactory(factory func(any) (dataSourceWithConfigure, error)) {
	p.frameworkDataSourceFactories = append(p.frameworkDataSourceFactories, factory)
}

var _sp = &servicePackage{}
`
	const factories = `package example

type metadataResponse struct {
	TypeName string
}

func init() {
	_sp.registerFrameworkDataSourceFactory(newDataSourceExample)
}

func newDataSourceExample(any) (dataSourceWithConfigure, error) {
	return &dataSourceExample{}, nil
}

type dataSourceExample struct{}

func (*dataSourceExample) Metadata(_ any, _ any, response *metadataResponse) {
	response.TypeName = "aws_example_data_source"
}
`
	tests := []struct {
		name            string
		registry        string
		factories       string
		provider        string
		allowTypeErrors bool
		want            []entrypointSpec
		wantErr         string
	}{
		{
			name:      "v4.49 generated framework data source registry",
			registry:  registry,
			factories: factories,
			want: []entrypointSpec{{
				kind:        "data_source",
				typeName:    "aws_example_data_source",
				factory:     "newDataSourceExample",
				sourceFile:  providerModulePath + "/internal/service/example/registry.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name: "v4.56 generated empty framework data source collection",
			registry: `package example

type dataSourceWithConfigure interface{}
type schemaResource struct{}

type servicePackage struct{}

func (p *servicePackage) FrameworkDataSources(any) []func(any) (dataSourceWithConfigure, error) {
	return []func(any) (dataSourceWithConfigure, error){}
}

func (p *servicePackage) SDKDataSources(any) map[string]func() *schemaResource {
	return map[string]func() *schemaResource{"aws_legacy_data_source": legacyDataSource}
}

func legacyDataSource() *schemaResource {
	return &schemaResource{}
}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			want: []entrypointSpec{{
				kind:        "data_source",
				typeName:    "aws_legacy_data_source",
				factory:     "legacyDataSource",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name: "v4.60 generated framework factory metadata",
			registry: `package example

type dataSourceWithConfigure interface{}

type resourceTags struct {
	IdentifierAttribute string
}

type frameworkDataSource struct {
	Factory func(any) (dataSourceWithConfigure, error)
	Name    string
	Tags    *resourceTags
}

type servicePackage struct{}

func (p *servicePackage) FrameworkDataSources(any) []*frameworkDataSource {
	return []*frameworkDataSource{
		{Factory: newDataSourceExample, Name: "Example", Tags: &resourceTags{IdentifierAttribute: "id"}},
	}
}

type schemaResource struct{}

func (p *servicePackage) SDKDataSources(any) map[string]func() *schemaResource {
	return map[string]func() *schemaResource{}
}

func newDataSourceExample(any) (dataSourceWithConfigure, error) {
	return &dataSourceExample{}, nil
}

type dataSourceExample struct{}

type metadataResponse struct {
	TypeName string
}

func (*dataSourceExample) Metadata(_ any, _ any, response *metadataResponse) {
	response.TypeName = "aws_example_data_source"
}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			provider: `package example

type legacyResource struct{}

func legacyDataSource() *legacyResource {
	return &legacyResource{}
}

var _ = struct {
	DataSourcesMap map[string]*legacyResource
}{
	DataSourcesMap: map[string]*legacyResource{
		"aws_legacy_data_source": legacyDataSource(),
	},
}
`,
			want: []entrypointSpec{{
				kind:        "data_source",
				typeName:    "aws_example_data_source",
				factory:     "newDataSourceExample",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}, {
				kind:        "data_source",
				typeName:    "aws_legacy_data_source",
				factory:     "legacyDataSource",
				sourceFile:  providerModulePath + "/internal/service/example/provider.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name: "v5.95 generated framework factory type name",
			registry: `package example

type dataSourceWithConfigure interface{}

type frameworkDataSource struct {
	Factory  func(any) (dataSourceWithConfigure, error)
	TypeName string
	Name     string
}

type servicePackage struct{}

func (p *servicePackage) FrameworkDataSources(any) []*frameworkDataSource {
	return []*frameworkDataSource{
		{Factory: newDataSourceExample, TypeName: "aws_example_data_source", Name: "Example"},
	}
}

func newDataSourceExample(any) (dataSourceWithConfigure, error) {
	return &dataSourceExample{}, nil
}

type dataSourceExample struct{}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			want: []entrypointSpec{{
				kind:        "data_source",
				typeName:    "aws_example_data_source",
				factory:     "newDataSourceExample",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name: "v6 generated framework registration metadata",
			registry: `package example

type dataSourceWithConfigure interface{}
type resourceWithConfigure interface{}

type metadataHandle struct{}
type identity struct{}
type importSpec struct{}

type frameworkDataSource struct {
	Factory  func(any) (dataSourceWithConfigure, error)
	TypeName string
	Name     string
	Tags     metadataHandle
	Region   metadataHandle
}

type frameworkResource struct {
	Factory  func(any) (resourceWithConfigure, error)
	TypeName string
	Name     string
	Tags     metadataHandle
	Region   metadataHandle
	Identity identity
	Import   importSpec
}

type servicePackage struct{}

type schemaResource struct{}

type sdkDataSource struct {
	Factory  func() *schemaResource
	TypeName string
	Name     string
	Tags     metadataHandle
	Region   metadataHandle
}

func (p *servicePackage) SDKDataSources(any) []*sdkDataSource {
	return []*sdkDataSource{
		{Factory: legacyDataSource, TypeName: "aws_example_legacy_data_source", Name: "Legacy"},
	}
}

func legacyDataSource() *schemaResource {
	return &schemaResource{}
}

func (p *servicePackage) FrameworkDataSources(any) []*frameworkDataSource {
	return []*frameworkDataSource{
		{Factory: newDataSourceExample, TypeName: "aws_example_data_source", Name: "Example"},
	}
}

func (p *servicePackage) FrameworkResources(any) []*frameworkResource {
	return []*frameworkResource{
		{Factory: newResourceExample, TypeName: "aws_example_resource", Name: "Example"},
	}
}

func newDataSourceExample(any) (dataSourceWithConfigure, error) {
	return &dataSourceExample{}, nil
}

func newResourceExample(any) (resourceWithConfigure, error) {
	return &resourceExample{}, nil
}

type dataSourceExample struct{}
type resourceExample struct{}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			want: []entrypointSpec{{
				kind:        "data_source",
				typeName:    "aws_example_data_source",
				factory:     "newDataSourceExample",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}, {
				kind:        "data_source",
				typeName:    "aws_example_legacy_data_source",
				factory:     "legacyDataSource",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}, {
				kind:        "resource",
				typeName:    "aws_example_resource",
				factory:     "newResourceExample",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name: "v5.77 generated ephemeral factory metadata",
			registry: `package example

type ephemeralWithConfigure interface{}

type ephemeralEntry struct {
	Factory func(any) (ephemeralWithConfigure, error)
	Name    string
}

type servicePackage struct{}

func (p *servicePackage) EphemeralResources(any) []*ephemeralEntry {
	return []*ephemeralEntry{
		{Factory: newEphemeralExample, Name: "Example"},
	}
}

func newEphemeralExample(any) (ephemeralWithConfigure, error) {
	return &ephemeralExample{}, nil
}

type ephemeralExample struct{}

type metadataResponse struct {
	TypeName string
}

func (*ephemeralExample) Metadata(_ any, _ any, response *metadataResponse) {
	response.TypeName = "aws_example_ephemeral"
}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			want: []entrypointSpec{{
				kind:        "ephemeral_resource",
				typeName:    "aws_example_ephemeral",
				factory:     "newEphemeralExample",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name: "v4.60 generated SDK factory metadata",
			registry: `package example

type dataSourceWithConfigure interface{}
type schemaResource struct{}
type resourceTags struct {
	IdentifierAttribute string
}

type sdkDataSource struct {
	Factory  func() *schemaResource
	TypeName string
	Name     string
	Tags     *resourceTags
}

type servicePackage struct{}

func (p *servicePackage) FrameworkDataSources(any) []func(any) (dataSourceWithConfigure, error) {
	return []func(any) (dataSourceWithConfigure, error){}
}

func (p *servicePackage) SDKDataSources(any) []*sdkDataSource {
	return []*sdkDataSource{
		{
			Factory:  legacyDataSource,
			TypeName: "aws_legacy_data_source",
			Name:     "Legacy",
			Tags:     &resourceTags{IdentifierAttribute: "id"},
		},
	}
}

func legacyDataSource() *schemaResource {
	return &schemaResource{}
}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			want: []entrypointSpec{{
				kind:        "data_source",
				typeName:    "aws_legacy_data_source",
				factory:     "legacyDataSource",
				sourceFile:  providerModulePath + "/internal/service/example/service_package_data_gen.go",
				packagePath: providerModulePath + "/internal/service/example",
			}},
		},
		{
			name:            "generated registry requires one factory",
			registry:        registry,
			factories:       "package example\n\nfunc init() { _sp.registerFrameworkDataSourceFactory() }\n",
			allowTypeErrors: true,
			wantErr:         "data_source registry call registerFrameworkDataSourceFactory has 0 factory arguments, want 1",
		},
		{
			name:            "generated registry requires a factory function",
			registry:        registry,
			factories:       "package example\n\nvar notAFactory int\n\nfunc init() { _sp.registerFrameworkDataSourceFactory(notAFactory) }\n",
			allowTypeErrors: true,
			wantErr:         "notAFactory does not resolve to a function",
		},
		{
			name:     "generated registry requires a metadata type name",
			registry: registry,
			factories: `package example

func init() {
	_sp.registerFrameworkDataSourceFactory(newDataSourceWithoutMetadata)
}

func newDataSourceWithoutMetadata(any) (dataSourceWithConfigure, error) {
	return &dataSourceWithoutMetadata{}, nil
}

type dataSourceWithoutMetadata struct{}
`,
			wantErr: "concrete type for newDataSourceWithoutMetadata has no Metadata method",
		},
		{
			name:     "generated registry requires the singleton receiver",
			registry: registry,
			factories: `package example

var other = _sp

func init() {
	other.registerFrameworkDataSourceFactory(newDataSourceExample)
}

func newDataSourceExample(any) (dataSourceWithConfigure, error) {
	return &dataSourceExample{}, nil
}

type dataSourceExample struct{}

type metadataResponse struct {
	TypeName string
}

func (*dataSourceExample) Metadata(_ any, _ any, response *metadataResponse) {
	response.TypeName = "aws_example_data_source"
}
`,
			wantErr: "data_source registry call registerFrameworkDataSourceFactory must use the service package singleton _sp",
		},
		{
			name: "generated registry requires a matching helper",
			registry: `package example

type dataSourceWithConfigure interface{}

type servicePackage struct {
	frameworkDataSourceFactories []func(any) (dataSourceWithConfigure, error)
}

func (p *servicePackage) FrameworkDataSources(any) []func(any) (dataSourceWithConfigure, error) {
	return p.frameworkDataSourceFactories
}

var _sp = &servicePackage{}
`,
			factories: "package example\n",
			wantErr:   "service package method FrameworkDataSources returned factory collection field frameworkDataSourceFactories without a matching registry helper",
		},
		{
			name: "generated registry requires the named factory collection field",
			registry: `package example

type dataSourceWithConfigure interface{}

type servicePackage struct {
	frameworkDataSourceFactories []func(any) (dataSourceWithConfigure, error)
}

func (p *servicePackage) FrameworkDataSources(any) []func(any) (dataSourceWithConfigure, error) {
	return p.unresolvableFactories
}
`,
			factories:       "package example\n",
			allowTypeErrors: true,
			wantErr:         "service package method FrameworkDataSources returned an unresolvable entrypoint specification",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			index := servicePackageRegistryFixture(t, test.registry, test.factories, test.provider, test.allowTypeErrors)
			got, err := discoverEntrypoints(index)
			if test.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), test.wantErr) {
					t.Fatalf("discoverEntrypoints() error = %v, want %q", err, test.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("discoverEntrypoints() error = %v", err)
			}
			if !slices.Equal(got, test.want) {
				t.Fatalf("discoverEntrypoints() = %#v, want %#v", got, test.want)
			}
		})
	}
}

func TestDiscoverEntrypointsParsesGeneratedSDKRegistrations(t *testing.T) {
	const registry = `package example

type schemaResource struct{}

type sdkFactoryEntry struct {
	TypeName string
	Factory  func() *schemaResource
}

type servicePackage struct {
	sdkDataSourceFactories []sdkFactoryEntry
	sdkResourceFactories   []sdkFactoryEntry
}

func (p *servicePackage) SDKDataSources() []sdkFactoryEntry {
	return p.sdkDataSourceFactories
}

func (p *servicePackage) SDKResources() []sdkFactoryEntry {
	return p.sdkResourceFactories
}

func (p *servicePackage) registerSDKDataSourceFactory(typeName string, factory func() *schemaResource) {
	p.sdkDataSourceFactories = append(p.sdkDataSourceFactories, sdkFactoryEntry{TypeName: typeName, Factory: factory})
}

func (p *servicePackage) registerSDKResourceFactory(typeName string, factory func() *schemaResource) {
	p.sdkResourceFactories = append(p.sdkResourceFactories, sdkFactoryEntry{TypeName: typeName, Factory: factory})
}

var _sp = &servicePackage{}
`
	const factories = `package example

func init() {
	_sp.registerSDKDataSourceFactory("aws_example_data_source", dataSourceExample)
	_sp.registerSDKResourceFactory("aws_example_resource", resourceExample)
}

func dataSourceExample() *schemaResource {
	return &schemaResource{}
}

func resourceExample() *schemaResource {
	return &schemaResource{}
}
`
	index := servicePackageRegistryFixture(t, registry, factories, "", false)
	got, err := discoverEntrypoints(index)
	if err != nil {
		t.Fatalf("discoverEntrypoints() error = %v", err)
	}
	want := []entrypointSpec{
		{
			kind:        "data_source",
			typeName:    "aws_example_data_source",
			factory:     "dataSourceExample",
			sourceFile:  providerModulePath + "/internal/service/example/registry.go",
			packagePath: providerModulePath + "/internal/service/example",
		},
		{
			kind:        "resource",
			typeName:    "aws_example_resource",
			factory:     "resourceExample",
			sourceFile:  providerModulePath + "/internal/service/example/registry.go",
			packagePath: providerModulePath + "/internal/service/example",
		},
	}
	if !slices.Equal(got, want) {
		t.Fatalf("discoverEntrypoints() = %#v, want %#v", got, want)
	}
}

func TestDiscoverEntrypointsMergesFrameworkAndProviderRegistries(t *testing.T) {
	const registry = `package example

func registerFrameworkResourceFactory(factory func() any) {}
func registerFrameworkDataSourceFactory(factory func() any) {}
`
	const factories = `package example

type metadataResponse struct {
	TypeName string
}

func init() {
	registerFrameworkResourceFactory(newFrameworkResource)
	registerFrameworkDataSourceFactory(newFrameworkDataSource)
}

func newFrameworkResource() any {
	return &frameworkResource{}
}

type frameworkResource struct{}

func (*frameworkResource) Metadata(response *metadataResponse) {
	response.TypeName = "aws_framework_resource"
}

func newFrameworkDataSource() any {
	return &frameworkDataSource{}
}

type frameworkDataSource struct{}

func (*frameworkDataSource) Metadata(response *metadataResponse) {
	response.TypeName = "aws_framework_data_source"
}
`
	const provider = `package example

type providerRegistry struct {
	ResourcesMap   map[string]any
	DataSourcesMap map[string]any
}

func legacyResource() any {
	return nil
}

func legacyDataSource() any {
	return nil
}

func Provider() *providerRegistry {
	return &providerRegistry{
		ResourcesMap: map[string]any{
			"aws_framework_resource": newFrameworkResource(),
			"aws_legacy_resource":    legacyResource(),
		},
		DataSourcesMap: map[string]any{
			"aws_framework_data_source": newFrameworkDataSource(),
			"aws_legacy_data_source":    legacyDataSource(),
		},
	}
}
`
	index := servicePackageRegistryFixture(t, registry, factories, provider, false)
	got, err := discoverEntrypoints(index)
	if err != nil {
		t.Fatalf("discoverEntrypoints() error = %v", err)
	}
	want := []entrypointSpec{
		{
			kind:        "data_source",
			typeName:    "aws_framework_data_source",
			factory:     "newFrameworkDataSource",
			sourceFile:  providerModulePath + "/internal/service/example/provider.go",
			packagePath: providerModulePath + "/internal/service/example",
		},
		{
			kind:        "data_source",
			typeName:    "aws_legacy_data_source",
			factory:     "legacyDataSource",
			sourceFile:  providerModulePath + "/internal/service/example/provider.go",
			packagePath: providerModulePath + "/internal/service/example",
		},
		{
			kind:        "resource",
			typeName:    "aws_framework_resource",
			factory:     "newFrameworkResource",
			sourceFile:  providerModulePath + "/internal/service/example/provider.go",
			packagePath: providerModulePath + "/internal/service/example",
		},
		{
			kind:        "resource",
			typeName:    "aws_legacy_resource",
			factory:     "legacyResource",
			sourceFile:  providerModulePath + "/internal/service/example/provider.go",
			packagePath: providerModulePath + "/internal/service/example",
		},
	}
	if !slices.Equal(got, want) {
		t.Fatalf("discoverEntrypoints() = %#v, want %#v", got, want)
	}
}

func TestDiscoverEntrypointsRejectsConflictingFrameworkAndProviderFactories(t *testing.T) {
	const registry = `package example

func registerFrameworkResourceFactory(factory func() any) {}
`
	const factories = `package example

type metadataResponse struct {
	TypeName string
}

func init() {
	registerFrameworkResourceFactory(newFrameworkResource)
}

func newFrameworkResource() any {
	return &frameworkResource{}
}

type frameworkResource struct{}

func (*frameworkResource) Metadata(response *metadataResponse) {
	response.TypeName = "aws_framework_resource"
}
`
	const provider = `package example

type providerRegistry struct {
	ResourcesMap map[string]any
}

func conflictingResource() any {
	return nil
}

func Provider() *providerRegistry {
	return &providerRegistry{
		ResourcesMap: map[string]any{
			"aws_framework_resource": conflictingResource(),
		},
	}
}
`
	index := servicePackageRegistryFixture(t, registry, factories, provider, false)
	_, err := discoverEntrypoints(index)
	if err == nil || !strings.Contains(err.Error(), "conflicting resource entrypoints for aws_framework_resource") {
		t.Fatalf("discoverEntrypoints() error = %v, want conflicting resource factories", err)
	}
}

func servicePackageRegistryFixture(t *testing.T, registry, factories, provider string, allowTypeErrors bool) *packageIndex {
	t.Helper()

	const packagePath = providerModulePath + "/internal/service/example"
	const servicePackagePath = packagePath + "/service_package_data_gen.go"
	const registryPath = packagePath + "/registry.go"
	const providerPath = packagePath + "/provider.go"
	if provider == "" {
		provider = "package example\n"
	}

	fileSet := token.NewFileSet()
	servicePackageFile, err := parser.ParseFile(fileSet, servicePackagePath, registry, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse service package fixture: %v", err)
	}
	registryFile, err := parser.ParseFile(fileSet, registryPath, factories, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse registry fixture: %v", err)
	}
	providerFile, err := parser.ParseFile(fileSet, providerPath, provider, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse provider fixture: %v", err)
	}
	info := &types.Info{
		Defs:  make(map[*ast.Ident]types.Object),
		Types: make(map[ast.Expr]types.TypeAndValue),
		Uses:  make(map[*ast.Ident]types.Object),
	}
	checked, err := (&types.Config{}).Check(packagePath, fileSet, []*ast.File{servicePackageFile, registryFile, providerFile}, info)
	if err != nil && !allowTypeErrors {
		t.Fatalf("type-check fixture: %v", err)
	}
	if checked == nil {
		t.Fatal("type-check fixture returned no package")
	}
	pkg := &packages.Package{
		PkgPath:   packagePath,
		GoFiles:   []string{servicePackagePath, registryPath, providerPath},
		Syntax:    []*ast.File{servicePackageFile, registryFile, providerFile},
		Types:     checked,
		TypesInfo: info,
	}
	index := &packageIndex{
		byTypes:   map[*types.Package]*packages.Package{checked: pkg},
		byPath:    map[string][]*packages.Package{packagePath: {pkg}},
		byFile:    map[string]*packages.Package{servicePackagePath: pkg, registryPath: pkg, providerPath: pkg},
		funcDecls: make(map[*types.Func]*ast.FuncDecl),
	}
	for _, file := range pkg.Syntax {
		for _, declaration := range file.Decls {
			funcDecl, ok := declaration.(*ast.FuncDecl)
			if !ok {
				continue
			}
			if function, ok := info.Defs[funcDecl.Name].(*types.Func); ok {
				index.funcDecls[function] = funcDecl
			}
		}
	}
	return index
}

func concreteHandlerFixture(t *testing.T, sourceFile string) (*packageIndex, *packages.Package, *ast.FuncDecl, *types.Func, *ast.FuncDecl, *ssa.Function) {
	t.Helper()

	const packagePath = providerModulePath + "/internal/service/accountaccess"
	const source = `package accountaccess

type withNoOpUpdate[T any] struct{}

func (w *withNoOpUpdate[T]) Update() {}

type ResourceWithModel[T any] struct {
	withNoOpUpdate[T]
}

type applicationResourceModel struct{}

type applicationResource struct {
	ResourceWithModel[applicationResourceModel]
}

func newApplicationResource() any {
	r := &applicationResource{}
	return r
}`
	fileSet := token.NewFileSet()
	file, err := parser.ParseFile(fileSet, sourceFile, source, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse fixture: %v", err)
	}
	info := &types.Info{
		Defs:       make(map[*ast.Ident]types.Object),
		Types:      make(map[ast.Expr]types.TypeAndValue),
		Uses:       make(map[*ast.Ident]types.Object),
		Selections: make(map[*ast.SelectorExpr]*types.Selection),
	}
	checked, err := (&types.Config{}).Check(packagePath, fileSet, []*ast.File{file}, info)
	if err != nil {
		t.Fatalf("type-check fixture: %v", err)
	}
	program := ssa.NewProgram(fileSet, 0)
	program.CreatePackage(checked, []*ast.File{file}, info, true).Build()

	_, factory := functionDeclaration(t, file, info, "newApplicationResource")
	update, updateDecl := functionDeclaration(t, file, info, "Update")
	updateBody := program.FuncValue(update)
	if updateBody == nil || updateBody.Syntax() != updateDecl {
		t.Fatal("fixture Update SSA body does not retain its declaration syntax")
	}
	sourcePkg := &packages.Package{Types: checked, TypesInfo: info}
	index := &packageIndex{
		prog:        program,
		byTypes:     map[*types.Package]*packages.Package{checked: sourcePkg},
		byPath:      map[string][]*packages.Package{packagePath: {sourcePkg}},
		funcDecls:   map[*types.Func]*ast.FuncDecl{},
		ssaFuncs:    map[*types.Func][]*ssa.Function{},
		ssaBySyntax: map[ast.Node][]*ssa.Function{},
	}
	for _, declaration := range file.Decls {
		funcDecl, ok := declaration.(*ast.FuncDecl)
		if !ok {
			continue
		}
		if obj, ok := info.Defs[funcDecl.Name].(*types.Func); ok {
			index.funcDecls[obj] = funcDecl
		}
	}
	return index, sourcePkg, factory, update, updateDecl, updateBody
}

func localCalleeFixture(t *testing.T, source string) (*packageIndex, *ssa.Function) {
	t.Helper()

	fileSet := token.NewFileSet()
	file, err := parser.ParseFile(fileSet, "autoflex_expand.go", source, parser.SkipObjectResolution|parser.ParseComments)
	if err != nil {
		t.Fatalf("parse fixture: %v", err)
	}
	info := &types.Info{
		Defs:       make(map[*ast.Ident]types.Object),
		Types:      make(map[ast.Expr]types.TypeAndValue),
		Uses:       make(map[*ast.Ident]types.Object),
		Selections: make(map[*ast.SelectorExpr]*types.Selection),
	}
	packagePath := providerModulePath + "/internal/framework/flex"
	checked, err := (&types.Config{}).Check(packagePath, fileSet, []*ast.File{file}, info)
	if err != nil {
		t.Fatalf("type-check fixture: %v", err)
	}
	program := ssa.NewProgram(fileSet, 0)
	ssaPackage := program.CreatePackage(checked, []*ast.File{file}, info, true)
	ssaPackage.Build()
	handler := ssaPackage.Func("handler")

	if handler == nil {
		t.Fatal("fixture has no handler SSA function")
	}

	sourcePkg := &packages.Package{Types: checked, TypesInfo: info}
	index := &packageIndex{
		prog:        program,
		byTypes:     map[*types.Package]*packages.Package{checked: sourcePkg},
		byPath:      map[string][]*packages.Package{packagePath: []*packages.Package{sourcePkg}},
		funcDecls:   map[*types.Func]*ast.FuncDecl{},
		ssaFuncs:    map[*types.Func][]*ssa.Function{},
		ssaBySyntax: map[ast.Node][]*ssa.Function{},
	}
	for _, declaration := range file.Decls {
		funcDecl, ok := declaration.(*ast.FuncDecl)
		if !ok {
			continue
		}
		if obj, ok := info.Defs[funcDecl.Name].(*types.Func); ok {
			index.funcDecls[obj] = funcDecl
		}
	}
	return index, handler
}
func sharedAPIMethodFixture(t *testing.T) (*packageIndex, []*ssa.Function, map[sdkMethodKey][]apiMethod, sdkMethodKey) {
	t.Helper()

	const sdkPath = "github.com/aws/aws-sdk-go/service/example"
	fileSet := token.NewFileSet()
	file, err := parser.ParseFile(fileSet, "shared.go", `package flex

import example "github.com/aws/aws-sdk-go/service/example"

func handlerOne(client *example.Client) {
	shared(client)
}

func handlerTwo(client *example.Client) {
	shared(client)
}

func shared(client *example.Client) {
	client.ListThings()
	cycle(client)
}

func cycle(client *example.Client) {
	shared(client)
}`, parser.SkipObjectResolution)
	if err != nil {
		t.Fatalf("parse fixture: %v", err)
	}

	sdkPkg := types.NewPackage(sdkPath, "example")
	client := types.NewNamed(types.NewTypeName(token.NoPos, sdkPkg, "Client", nil), types.NewStruct(nil, nil), nil)
	sdkPkg.Scope().Insert(client.Obj())
	listThings := newMethod(sdkPkg, client, "ListThings")
	client.AddMethod(listThings)
	sdkPkg.MarkComplete()

	info := &types.Info{
		Defs:       make(map[*ast.Ident]types.Object),
		Types:      make(map[ast.Expr]types.TypeAndValue),
		Uses:       make(map[*ast.Ident]types.Object),
		Selections: make(map[*ast.SelectorExpr]*types.Selection),
	}
	packagePath := providerModulePath + "/internal/framework/flex"
	checked, err := (&types.Config{Importer: packageMapImporter{sdkPath: sdkPkg}}).Check(packagePath, fileSet, []*ast.File{file}, info)
	if err != nil {
		t.Fatalf("type-check fixture: %v", err)
	}
	program := ssa.NewProgram(fileSet, 0)
	program.CreatePackage(sdkPkg, nil, nil, true)
	ssaPackage := program.CreatePackage(checked, []*ast.File{file}, info, true)
	ssaPackage.Build()

	sourcePkg := &packages.Package{Types: checked, TypesInfo: info}
	index := &packageIndex{
		prog:        program,
		byTypes:     map[*types.Package]*packages.Package{checked: sourcePkg},
		byPath:      map[string][]*packages.Package{packagePath: {sourcePkg}},
		funcDecls:   map[*types.Func]*ast.FuncDecl{},
		ssaFuncs:    map[*types.Func][]*ssa.Function{},
		ssaBySyntax: map[ast.Node][]*ssa.Function{},
	}
	handlers := make([]*ssa.Function, 0, 2)
	for _, name := range []string{"handlerOne", "handlerTwo", "shared", "cycle"} {
		obj, decl := functionDeclaration(t, file, info, name)
		fn := ssaPackage.Func(name)
		if fn == nil || fn.Syntax() != decl {
			t.Fatalf("fixture %s SSA body is missing", name)
		}
		index.funcDecls[obj] = decl
		index.ssaFuncs[obj] = []*ssa.Function{fn}
		if name == "handlerOne" || name == "handlerTwo" {
			handlers = append(handlers, fn)
		}
	}
	key := mustSDKKey(t, listThings)
	return index, handlers, map[sdkMethodKey][]apiMethod{
		key: {{Service: "example", Name: "ListThings"}},
	}, key
}

type packageMapImporter map[string]*types.Package

func (importer packageMapImporter) Import(path string) (*types.Package, error) {
	pkg := importer[path]
	if pkg == nil {
		return nil, fmt.Errorf("package %s is not available", path)
	}
	return pkg, nil
}

func methodSelectionForCall(t *testing.T, index *packageIndex, fn *ssa.Function, method string) *types.Selection {
	t.Helper()

	if fn == nil || fn.Package() == nil || fn.Package().Pkg == nil {
		t.Fatal("fixture handler has no package")
	}
	sourcePkg := index.byTypes[fn.Package().Pkg]
	if sourcePkg == nil {
		t.Fatal("fixture handler package is not indexed")
	}
	var selection *types.Selection
	ast.Inspect(fn.Syntax(), func(node ast.Node) bool {
		selector, ok := node.(*ast.SelectorExpr)
		if !ok || selector.Sel.Name != method {
			return true
		}
		selection = sourcePkg.TypesInfo.Selections[selector]
		return selection == nil
	})
	if selection == nil {
		t.Fatalf("fixture has no %s method selection", method)
	}
	return selection
}

func methodSelectionForDeclaredMethod(t *testing.T, method *types.Func) *types.Selection {
	t.Helper()

	signature, ok := method.Type().(*types.Signature)
	if !ok || signature.Recv() == nil {
		t.Fatalf("%s is not a method", method.Name())
	}
	selection := types.NewMethodSet(signature.Recv().Type()).Lookup(method.Pkg(), method.Name())
	if selection == nil {
		t.Fatalf("method selection is missing for %s", method.Name())
	}
	return selection
}

func duplicatePathLocalCalleeFixture(t *testing.T) (*packageIndex, []*ssa.Function, []*ssa.Function) {
	t.Helper()

	const packagePath = providerModulePath + "/internal/framework/flex"
	const source = `package flex

type tagOptions struct{}

func (tagOptions) NoExpand() {}

func handler() {
	tagOptions{}.NoExpand()
}`

	fileSet := token.NewFileSet()
	program := ssa.NewProgram(fileSet, 0)
	index := &packageIndex{
		prog:               program,
		byTypes:            map[*types.Package]*packages.Package{},
		byPath:             map[string][]*packages.Package{},
		funcDecls:          map[*types.Func]*ast.FuncDecl{},
		ssaFuncs:           map[*types.Func][]*ssa.Function{},
		ssaFuncsByIdentity: map[functionIdentity][]*ssa.Function{},
		ssaBySyntax:        map[ast.Node][]*ssa.Function{},
	}
	handlers := make([]*ssa.Function, 0, 2)
	noExpands := make([]*ssa.Function, 0, 2)
	for i := 0; i < 2; i++ {
		file, err := parser.ParseFile(fileSet, fmt.Sprintf("tags_%d.go", i), source, parser.SkipObjectResolution)
		if err != nil {
			t.Fatalf("parse universe %d fixture: %v", i, err)
		}
		info := &types.Info{
			Defs:       map[*ast.Ident]types.Object{},
			Types:      map[ast.Expr]types.TypeAndValue{},
			Uses:       map[*ast.Ident]types.Object{},
			Selections: map[*ast.SelectorExpr]*types.Selection{},
		}
		checked, err := (&types.Config{}).Check(packagePath, fileSet, []*ast.File{file}, info)
		if err != nil {
			t.Fatalf("type-check universe %d fixture: %v", i, err)
		}
		sourcePkg := &packages.Package{Types: checked, TypesInfo: info}
		index.byTypes[checked] = sourcePkg
		index.byPath[packagePath] = append(index.byPath[packagePath], sourcePkg)

		noExpandObj, noExpandDecl := functionDeclaration(t, file, info, "NoExpand")
		_, handlerDecl := functionDeclaration(t, file, info, "handler")
		ssaPackage := program.CreatePackage(checked, []*ast.File{file}, info, true)
		ssaPackage.Build()
		signature, ok := noExpandObj.Type().(*types.Signature)
		if !ok || signature.Recv() == nil {
			t.Fatalf("universe %d NoExpand is not a method", i)
		}
		selection := types.NewMethodSet(signature.Recv().Type()).Lookup(checked, noExpandObj.Name())
		if selection == nil {
			t.Fatalf("universe %d NoExpand method selection is missing", i)
		}
		noExpand := program.MethodValue(selection)
		if noExpand == nil || noExpand.Syntax() != noExpandDecl {
			t.Fatalf("universe %d NoExpand SSA body does not retain its declaration syntax", i)
		}
		handler := ssaPackage.Func("handler")
		if handler == nil || handler.Syntax() != handlerDecl {
			t.Fatalf("universe %d handler SSA body does not retain its declaration syntax", i)
		}
		index.funcDecls[noExpandObj] = noExpandDecl
		handlers = append(handlers, handler)
		noExpands = append(noExpands, noExpand)
	}
	return index, handlers, noExpands
}
