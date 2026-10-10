package main

import (
	"os"
	"path/filepath"
	"slices"
	"testing"
)

func TestFactoryBoundPromotedInterfaceDispatch(t *testing.T) {
	dir := t.TempDir()
	const sdkPath = "github.com/aws/aws-sdk-go/service/ec2"
	files := map[string]string{
		"go.mod":     "module " + providerModulePath + "\n\ngo 1.26.0\nrequire " + sdkPath + " v1.0.0\nreplace " + sdkPath + " => ./sdk\n",
		"sdk/go.mod": "module " + sdkPath + "\n\ngo 1.26.0\n",
		"sdk/ec2.go": `package ec2
type EC2 struct{}
func (*EC2) AuthorizeSecurityGroupIngress() {}
func (*EC2) AuthorizeSecurityGroupIngressRequest() {}
func (*EC2) AuthorizeSecurityGroupEgress() {}
func (*EC2) AuthorizeSecurityGroupEgressRequest() {}
func (*EC2) DescribeSecurityGroupRules() {}
func (*EC2) DescribeSecurityGroupRulesRequest() {}
func (*EC2) RevokeSecurityGroupIngress() {}
func (*EC2) RevokeSecurityGroupIngressRequest() {}
func (*EC2) RevokeSecurityGroupEgress() {}
func (*EC2) RevokeSecurityGroupEgressRequest() {}
`,
		"internal/provider/provider.go": "package provider\n",
		"internal/service/ec2/rules.go": `package ec2
import sdk "` + sdkPath + `"
type resource interface { Create(); Read(); Delete() }
type rule interface { create(); find(); delete() }
type base struct { rule }
func (r *base) Create() { r.create() }
func (r *base) Read() { r.find() }
func (r *base) Delete() { r.delete() }
type ingress struct { base }
func newIngress() resource { r := &ingress{}; r.rule = r; return r }
func (*ingress) create() { new(sdk.EC2).AuthorizeSecurityGroupIngress() }
func (*ingress) find() { new(sdk.EC2).DescribeSecurityGroupRules() }
func (*ingress) delete() { new(sdk.EC2).RevokeSecurityGroupIngress() }
type egress struct { base }
func newEgress() resource { r := &egress{}; r.rule = r; return r }
func (*egress) create() { new(sdk.EC2).AuthorizeSecurityGroupEgress() }
func (*egress) find() { new(sdk.EC2).DescribeSecurityGroupRules() }
func (*egress) delete() { new(sdk.EC2).RevokeSecurityGroupEgress() }
// Unbound dynamic dispatch must continue to include every implementation.
func newUnbound() resource { return &base{} }
func newReassigned() resource { r := &ingress{}; r.rule = r; r.rule = &egress{}; return r }
func newConditional() resource { r := &ingress{}; r.rule = r; if unknown() { r.rule = &egress{} }; return r }
func unknown() bool { return true }
func newUnrelated() resource { other := &egress{}; other.rule = other; r := &ingress{}; return r }
func newConditionalReturn() resource { r := &ingress{}; r.rule = r; if unknown() { return &egress{} }; return r }
func newReplacedReceiver() resource { r := &ingress{}; r.rule = r; r = &ingress{}; return r }
type callbackBase struct { create func(); find func(); delete func() }
func (r *callbackBase) Create() { r.create() }
func (r *callbackBase) Read() { r.find() }
func (r *callbackBase) Delete() { r.delete() }
type callbackIngress struct { callbackBase }
func newCallbackIngress() resource {
 r := &callbackIngress{}
 r.create = r.createRule; r.find = r.findRule; r.delete = r.deleteRule
 return r
}
func (*callbackIngress) createRule() { new(sdk.EC2).AuthorizeSecurityGroupIngress() }
func (*callbackIngress) findRule() { new(sdk.EC2).DescribeSecurityGroupRules() }
func (*callbackIngress) deleteRule() { new(sdk.EC2).RevokeSecurityGroupIngress() }
type callbackEgress struct { callbackBase }
func newCallbackEgress() resource {
 r := &callbackEgress{}
 r.create = r.createRule; r.find = r.findRule; r.delete = r.deleteRule
 return r
}
func (*callbackEgress) createRule() { new(sdk.EC2).AuthorizeSecurityGroupEgress() }
func (*callbackEgress) findRule() { new(sdk.EC2).DescribeSecurityGroupRules() }
func (*callbackEgress) deleteRule() { new(sdk.EC2).RevokeSecurityGroupEgress() }
type explicitBase struct { rule }
func (r *explicitBase) Create() { r.rule.create() }
func (r *explicitBase) Read() { r.rule.find() }
func (r *explicitBase) Delete() { r.rule.delete() }
type explicitIngress struct { explicitBase }
func newExplicitIngress() resource { r := &explicitIngress{}; r.rule = r; return r }
func (*explicitIngress) create() { new(sdk.EC2).AuthorizeSecurityGroupIngress() }
func (*explicitIngress) find() { new(sdk.EC2).DescribeSecurityGroupRules() }
func (*explicitIngress) delete() { new(sdk.EC2).RevokeSecurityGroupIngress() }
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
	// Both orders exercise shared handler bodies and the interface cache.
	for _, direction := range []string{"Ingress", "Egress", "CallbackIngress", "CallbackEgress", "ExplicitIngress", "Unbound", "Reassigned", "Conditional", "Unrelated", "ConditionalReturn", "ReplacedReceiver", "Egress", "Ingress"} {
		spec := entrypointSpec{kind: "resource", typeName: "aws_rule", factory: "new" + direction, packagePath: providerModulePath + "/internal/service/ec2"}
		handlers, err := resolveHandlers(index, spec)
		if err != nil {
			t.Fatal(err)
		}
		for _, handler := range handlers {
			unbound := direction == "Unbound" || direction == "Reassigned" || direction == "Conditional" || direction == "Unrelated" || direction == "ConditionalReturn" || direction == "ReplacedReceiver"
			if unbound && len(handler.interfaceBindings) != 0 {
				t.Fatalf("%s has an unproven factory binding", direction)
			}
			analyzer := newAPIMethodAnalyzer(index, nil, spec.packagePath)
			analyzer.interfaceBindings = handler.interfaceBindings
			analyzer.functionBindings = handler.functionBindings
			analysis, err := analyzer.collect(handler.funcs)
			if err != nil {
				t.Fatal(err)
			}
			var names []string
			apiDirection := direction
			if direction == "CallbackIngress" || direction == "ExplicitIngress" {
				apiDirection = "Ingress"
			}
			if direction == "CallbackEgress" {
				apiDirection = "Egress"
			}
			switch handler.action {
			case "create":
				names = []string{"AuthorizeSecurityGroup" + apiDirection}
				if unbound {
					names = []string{"AuthorizeSecurityGroupEgress", "AuthorizeSecurityGroupIngress"}
				}
			case "read":
				names = []string{"DescribeSecurityGroupRules"}
			case "delete":
				names = []string{"RevokeSecurityGroup" + apiDirection}
				if unbound {
					names = []string{"RevokeSecurityGroupEgress", "RevokeSecurityGroupIngress"}
				}
			}
			want := make([]apiMethod, 0, len(names))
			for _, name := range names {
				want = append(want, apiMethod{Service: "ec2", Name: name})
			}
			if !slices.Equal(analysis.methods, want) {
				t.Fatalf("%s %s = %#v, want %#v", direction, handler.action, analysis.methods, want)
			}
		}
	}
}
