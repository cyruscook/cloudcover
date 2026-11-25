package main

import (
	"crypto/sha256"
	"fmt"
	"go/ast"
	"go/types"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"golang.org/x/tools/go/ssa"
)

// Controlled, executable fixtures model the reviewed v6.74.0 SDK list protocol;
// SDK clients only record calls. They never contact AWS. Actual provider-source
// generation is separately bound by the source receipts in main.go.
func TestSDKListTagsStateAndTypedFallback(t *testing.T) {
	dir := t.TempDir()
	const sdkPath = "github.com/aws/aws-sdk-go/service/iam"
	files := map[string]string{
		"go.mod":     "module " + providerModulePath + "\n\ngo 1.27.0\nrequire " + sdkPath + " v1.0.0\nreplace " + sdkPath + " => ./sdk\n",
		"sdk/go.mod": "module " + sdkPath + "\n\ngo 1.27.0\n",
		"sdk/iam.go": `package iam
type Client struct { Calls []string; Tags []string }
func (c *Client) ListUsers() { c.Calls = append(c.Calls, "ListUsers") }
func (c *Client) GetUser() []string { c.Calls = append(c.Calls, "GetUser"); return c.Tags }
func (c *Client) ListUserTags() { c.Calls = append(c.Calls, "ListUserTags") }
func (c *Client) ListRoleTags() { c.Calls = append(c.Calls, "ListRoleTags") }
func (c *Client) ListPolicyTags() { c.Calls = append(c.Calls, "ListPolicyTags") }
func (c *Client) GetRole() { c.Calls = append(c.Calls, "GetRole") }
func (c *Client) GetAssetFile() { c.Calls = append(c.Calls, "GetAssetFile") }
func (c *Client) GetAsset() { c.Calls = append(c.Calls, "GetAsset") }
func (c *Client) GetAgentSpace() { c.Calls = append(c.Calls, "GetAgentSpace") }
func (c *Client) ListAssetFiles() { c.Calls = append(c.Calls, "ListAssetFiles") }
func (c *Client) DescribeConfigurationSets() { c.Calls = append(c.Calls, "DescribeConfigurationSets") }

`,
		"internal/provider/provider.go": "package provider\n",
		"internal/types/tags.go": `package types
type ServicePackageResourceTags string
func ResourceTagsTypeAndAttribute(resourceType, identifierAttribute string) ServicePackageResourceTags { return ServicePackageResourceTags(resourceType) }
`,
		"internal/tags/tags.go": `package tags
import sdk "` + sdkPath + `"
type ResourceTypeTagLister interface { ListTags(int, *sdk.Client, string, string) }
// Present distinguishes Some(empty) from None.
type State struct { Present bool; Tags []string }
func (s *State) IsNone() bool { return !s.Present }
`,
		"internal/provider/interceptors/htags.go": `package interceptors
import (
 sdk "` + sdkPath + `"
 "` + providerModulePath + `/internal/tags"
)
type HTags string
func (h HTags) ListTags(ctx int, sp tags.ResourceTypeTagLister, c *sdk.Client, identifier string) {
 resourceType := string(h)
 sp.ListTags(ctx, c, identifier, resourceType)
}
`,
		"internal/provider/framework/listresource/tags.go": `package listresource
import (
 sdk "` + sdkPath + `"
 "` + providerModulePath + `/internal/tags"
 "` + providerModulePath + `/internal/provider/interceptors"
)
type tagsInterceptorSDK struct { interceptors.HTags }
type Params struct { IncludeResource bool; TagsOut *tags.State; C *sdk.Client; SP tags.ResourceTypeTagLister }
func (r tagsInterceptorSDK) Read(ctx int, params Params) {
 if !params.IncludeResource { return }
 tagsInContext := params
 if tagsInContext.TagsOut.IsNone() { r.ListTags(ctx, params.SP, params.C, "id") }
 params.TagsOut.Present = false
}
type tagsInterceptor struct { interceptors.HTags }
func (r tagsInterceptor) Read(ctx int, params Params) {
 if !params.IncludeResource { return }
 if params.TagsOut.IsNone() { r.ListTags(ctx, params.SP, params.C, "id") }
}
func FrameworkResult(c *sdk.Client, sp tags.ResourceTypeTagLister, enabled bool) {
 if enabled { r:=tagsInterceptor{HTags:"User"}; r.Read(0, Params{IncludeResource:true, TagsOut:&tags.State{}, C:c, SP:sp}) }
}
func SetResult(c *sdk.Client, sp tags.ResourceTypeTagLister, include bool, state *tags.State, resourceType string) {
 r := tagsInterceptorSDK{HTags: interceptors.HTags(resourceType)}
 r.Read(0, Params{IncludeResource:include, TagsOut:state, C:c, SP:sp})
}
`,
		"internal/service/iam/list.go": `package iam
import (
 sdk "` + sdkPath + `"
 "` + providerModulePath + `/internal/tags"
 lr "` + providerModulePath + `/internal/provider/framework/listresource"
 it "` + providerModulePath + `/internal/types"
)
type servicePackage struct{}
func (*servicePackage) ListTags(ctx int, c *sdk.Client, identifier, resourceType string) {
 switch resourceType {
 case "User": c.ListUserTags()
 case "Role", "ServiceLinkedRole": c.ListRoleTags()
 case "Policy": c.ListPolicyTags()
 default: return
 }
}
func User(c *sdk.Client, include bool) {
 c.ListUsers()
 state := &tags.State{}
 if include { state.Tags = c.GetUser(); state.Present = true }
 lr.SetResult(c, &servicePackage{}, include, state, "User")
}
func UserFallback(c *sdk.Client) { lr.SetResult(c, &servicePackage{}, true, &tags.State{}, "User") }
func Role(c *sdk.Client) { lr.SetResult(c, &servicePackage{}, true, &tags.State{}, "Role") }
// A direct unbound call must not inherit the HTags edge's discriminator.
func DirectAndFallback(c *sdk.Client) { UserFallback(c); new(servicePackage).ListTags(0,c,"id","Role") }
func UntaggedAsset(c *sdk.Client, accessDenied bool) {
 c.ListAssetFiles(); c.GetAssetFile()
 if accessDenied {c.GetAsset(); c.GetAgentSpace()}
 lr.FrameworkResult(c, &servicePackage{}, false)
}
func UntaggedAssociation(c *sdk.Client) {c.DescribeConfigurationSets(); lr.FrameworkResult(c,&servicePackage{},false)}
func UntaggedWithDirectTags(c *sdk.Client) {UntaggedAssociation(c); c.ListUserTags()}
type spec struct { Factory func(*sdk.Client, bool); TypeName string; Tags it.ServicePackageResourceTags }
var registration = spec{Factory:User, TypeName:"aws_iam_user", Tags:it.ResourceTagsTypeAndAttribute("User","id")}
var untagged = spec{Factory:UntaggedAsset, TypeName:"aws_devopsagent_asset_file"}
`,
		"internal/service/iam/list_test.go": `package iam
import ("testing"; "slices"; sdk "` + sdkPath + `")
func TestOfflineCalls(t *testing.T) {
 for _, tc := range []struct { name string; include bool; tags []string; want []string }{
 {"identity", false, nil, []string{"ListUsers"}},
 {"full_nil", true, nil, []string{"ListUsers","GetUser"}},
 {"full_empty", true, []string{}, []string{"ListUsers","GetUser"}},
 {"full_tagged", true, []string{"team=example"}, []string{"ListUsers","GetUser"}},
 } { t.Run(tc.name,func(t *testing.T) { c:=&sdk.Client{Tags:tc.tags}; User(c,tc.include); if !slices.Equal(c.Calls,tc.want) {t.Fatalf("%v != %v",c.Calls,tc.want)} }) }
 c:=&sdk.Client{}; UserFallback(c); if !slices.Equal(c.Calls,[]string{"ListUserTags"}) {t.Fatal(c.Calls)}
 c=&sdk.Client{}; Role(c); if !slices.Equal(c.Calls,[]string{"ListRoleTags"}) {t.Fatal(c.Calls)}
 c=&sdk.Client{}; UntaggedAsset(c,true); if !slices.Equal(c.Calls,[]string{"ListAssetFiles","GetAssetFile","GetAsset","GetAgentSpace"}) {t.Fatal(c.Calls)}
 c=&sdk.Client{}; UntaggedAssociation(c); if !slices.Equal(c.Calls,[]string{"DescribeConfigurationSets"}) {t.Fatal(c.Calls)}
}
`,
	}
	for path, source := range files {
		full := filepath.Join(dir, path)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(full, []byte(source), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	cmd := exec.Command("go", "test", "./...")
	cmd.Dir = dir
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("runtime fixture: %v\n%s", err, out)
	}
	index, err := loadProviderIndex(dir)
	if err != nil {
		t.Fatal(err)
	}
	pkg := index.byPath[providerModulePath+"/internal/service/iam"][0]
	mappings := map[sdkMethodKey][]apiMethod{}
	for _, name := range []string{"ListUsers", "GetUser", "ListUserTags", "ListRoleTags", "ListPolicyTags", "GetRole", "GetAssetFile", "GetAsset", "GetAgentSpace", "ListAssetFiles", "DescribeConfigurationSets"} {
		mappings[sdkMethodKey{pkg: sdkPath, receiver: "Client", method: name}] = []apiMethod{{Service: "iam", Name: name}}
	}
	// Exercise the same typed registration decoder used by discovery.
	found := false
	for _, file := range pkg.Syntax {
		ast.Inspect(file, func(n ast.Node) bool {
			lit, ok := n.(*ast.CompositeLit)
			if !ok {
				return true
			}
			spec, err := entrypointFromCompositeLit(pkg, "list_resource", "fixture.go", lit)
			if err == nil && spec.typeName == "aws_iam_user" {
				found = true
				if spec.tagMetadataOmitted {
					t.Fatal("enabled tags marked omitted")
				}
				if spec.tagResourceType != "User" {
					t.Fatalf("tag type = %q", spec.tagResourceType)
				}
			}
			if err == nil && spec.typeName == "aws_devopsagent_asset_file" && !spec.tagMetadataOmitted {
				t.Fatal("omitted Tags not preserved")
			}
			return true
		})
	}
	if !found {
		t.Fatal("no typed registration")
	}
	// Static analysis keeps its existing union whenever fallback remains reachable.
	// Runtime controls above prove that the actual typed dispatcher calls only the
	// matching User/Role API; no general switch pruning is needed for this fix.
	for _, tc := range []struct {
		name  string
		prune bool
		want  []string
	}{
		{"User", true, []string{"GetUser", "ListUsers"}},
		{"UserFallback", false, []string{"ListPolicyTags", "ListRoleTags", "ListUserTags"}},
		{"Role", false, []string{"ListPolicyTags", "ListRoleTags", "ListUserTags"}},
		{"DirectAndFallback", false, []string{"ListPolicyTags", "ListRoleTags", "ListUserTags"}},
	} {
		analyzer := newAPIMethodAnalyzer(index, mappings, pkg.PkgPath)
		if tc.prune {
			analyzer.pruned = sdkListTagFallbackRanges(index)
			if len(analyzer.pruned) != 1 {
				t.Fatalf("fallback ranges=%v", analyzer.pruned)
			}
		}
		root := index.prog.FuncValue(pkg.Types.Scope().Lookup(tc.name).(*types.Func))
		result, err := analyzer.collect([]*ssa.Function{root})
		if err != nil {
			t.Fatal(err)
		}
		var names []string
		for _, method := range result.methods {
			names = append(names, method.Name)
		}
		if !slices.Equal(names, tc.want) {
			t.Fatalf("%s prune=%v got %v want %v", tc.name, tc.prune, names, tc.want)
		}
	}

	for _, tc := range []struct {
		name string
		want []string
	}{
		{"UntaggedAsset", []string{"GetAgentSpace", "GetAsset", "GetAssetFile", "ListAssetFiles"}},
		{"UntaggedAssociation", []string{"DescribeConfigurationSets"}},
		{"UntaggedWithDirectTags", []string{"DescribeConfigurationSets", "ListUserTags"}},
	} {
		analyzer := newAPIMethodAnalyzer(index, mappings, pkg.PkgPath)
		analyzer.pruned = unregisteredListTagInterceptorRanges(index)
		if len(analyzer.pruned) != 2 {
			t.Fatalf("disabled interceptor ranges: %v", analyzer.pruned)
		}
		root := index.prog.FuncValue(pkg.Types.Scope().Lookup(tc.name).(*types.Func))
		result, err := analyzer.collect([]*ssa.Function{root})
		if err != nil {
			t.Fatal(err)
		}
		var names []string
		for _, method := range result.methods {
			names = append(names, method.Name)
		}
		if !slices.Equal(names, tc.want) {
			t.Fatalf("%s got %v want %v", tc.name, names, tc.want)
		}
	}

}

func TestSourceEvidenceMustMatchEveryFile(t *testing.T) {
	dir := t.TempDir()
	body := []byte("reviewed source\n")
	if err := os.WriteFile(filepath.Join(dir, "source.go"), body, 0o600); err != nil {
		t.Fatal(err)
	}
	receipt := map[string]string{"source.go": fmt.Sprintf("%x", sha256.Sum256(body))}
	if !matchesSourceFiles(dir, receipt) {
		t.Fatal("identical source rejected")
	}
	if err := os.WriteFile(filepath.Join(dir, "source.go"), []byte(strings.ReplaceAll(string(body), "reviewed", "mutated")), 0o600); err != nil {
		t.Fatal(err)
	}
	if matchesSourceFiles(dir, receipt) {
		t.Fatal("changed source accepted")
	}
	if matchesTagProtocol(dir) {
		t.Fatal("missing protocol source accepted")
	}
	spec := entrypointSpec{kind: "list_resource", packagePath: providerModulePath + "/internal/service/iam", factory: "newUserResourceAsListResource", typeName: "aws_iam_user", tagResourceType: "User"}
	if reviewedUserListTagsAlreadySet(dir, spec) {
		t.Fatal("missing user source accepted")
	}
}

// Optional real-source check accompanies local release regeneration. Normal CI
// uses the controlled no-network fixture above, rather than fetching upstream.
func TestReviewedProviderSourceReceipt(t *testing.T) {
	root := os.Getenv("CLOUDCOVER_TEST_PROVIDER_DIR")
	if root == "" {
		t.Skip("set CLOUDCOVER_TEST_PROVIDER_DIR to the v6.74.0 analysis checkout")
	}
	spec := entrypointSpec{kind: "list_resource", packagePath: providerModulePath + "/internal/service/iam", factory: "newUserResourceAsListResource", typeName: "aws_iam_user", tagResourceType: "User"}
	if !reviewedUserListTagsAlreadySet(root, spec) {
		t.Fatal("provider is not the exact reviewed tag-state source")
	}
	all := map[string]string{}
	for path, digest := range reviewedListTagProtocol {
		all[path] = digest
	}
	for path, digest := range reviewedUserListTagState {
		all[path] = digest
	}
	dir := t.TempDir()
	for path := range all {
		body, err := os.ReadFile(filepath.Join(root, path))
		if err != nil {
			t.Fatal(err)
		}
		target := filepath.Join(dir, path)
		if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(target, body, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if reviewedProviderCheckout(dir) {
		t.Fatal("source copies without the exact Git provenance accepted")
	}
	if !matchesUserListTagSource(dir) {
		t.Fatal("copied evidence did not match")
	}
	for path := range all {
		target := filepath.Join(dir, path)
		original, err := os.ReadFile(target)
		if err != nil {
			t.Fatal(err)
		}
		// Even cosmetic drift is conservative; in particular, changed empty-tag,
		// context, registration, carrier or interceptor behavior cannot reuse proof.
		if err := os.WriteFile(target, append(append([]byte{}, original...), []byte("\n// drift\n")...), 0o600); err != nil {
			t.Fatal(err)
		}
		if matchesUserListTagSource(dir) {
			t.Fatalf("accepted changed %s", path)
		}
		if err := os.WriteFile(target, original, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	for _, mutate := range []func(*entrypointSpec){
		func(s *entrypointSpec) { s.tagResourceType = "Role" },
		func(s *entrypointSpec) { s.tagResourceType = "" },
		func(s *entrypointSpec) { s.kind = "resource" },
		func(s *entrypointSpec) { s.factory = "otherFactory" },
		func(s *entrypointSpec) { s.typeName = "aws_iam_role" },
	} {
		changed := spec
		mutate(&changed)
		if reviewedUserListTagsAlreadySet(root, changed) {
			t.Fatalf("accepted changed entrypoint: %#v", changed)
		}
	}
}

func TestConflictingTagRegistrationIsRejected(t *testing.T) {
	known := entrypointSpec{kind: "list_resource", packagePath: providerModulePath + "/internal/service/iam", factory: "newUserResourceAsListResource", typeName: "aws_iam_user", tagResourceType: "User", sourceFile: "first.go"}
	for _, tag := range []string{"Role", ""} {
		other := known
		other.sourceFile = "other.go"
		other.tagResourceType = tag
		if _, err := compactEntrypointSpecs([]entrypointSpec{known, other}); err == nil {
			t.Fatalf("accepted ambiguous tag type %q", tag)
		}
	}
}

func TestOmittedAndEnabledUntypedTagRegistrationsConflict(t *testing.T) {
	absent := entrypointSpec{kind: "list_resource", packagePath: providerModulePath + "/internal/service/pinpointsmsvoicev2", factory: "newProtectConfigurationAssociationResourceAsListResource", typeName: "aws_pinpointsmsvoicev2_protect_configuration_association", tagMetadataOmitted: true, sourceFile: "absent.go"}
	enabled := absent
	enabled.tagMetadataOmitted = false
	enabled.sourceFile = "enabled.go"
	// ResourceTagsAttribute(ARN) and omitted Tags both have no type discriminator,
	// but only the former registers an interceptor.
	if _, err := compactEntrypointSpecs([]entrypointSpec{absent, enabled}); err == nil {
		t.Fatal("accepted conflicting tag enablement")
	}
}

func TestUntaggedListRecognitionIsNarrow(t *testing.T) {
	for _, spec := range []entrypointSpec{
		{kind: "list_resource", packagePath: providerModulePath + "/internal/service/devopsagent", factory: "newAssetFileResourceAsListResource", typeName: "aws_devopsagent_asset_file", tagMetadataOmitted: true},
		{kind: "list_resource", packagePath: providerModulePath + "/internal/service/pinpointsmsvoicev2", factory: "newProtectConfigurationAssociationResourceAsListResource", typeName: "aws_pinpointsmsvoicev2_protect_configuration_association", tagMetadataOmitted: true},
	} {
		if !isReviewedUntaggedListSpec(spec) {
			t.Fatalf("reviewed omitted-Tags registration rejected: %#v", spec)
		}
		for _, mutate := range []func(*entrypointSpec){
			func(s *entrypointSpec) { s.tagMetadataOmitted = false },
			func(s *entrypointSpec) { s.factory = "unreviewed" },
			func(s *entrypointSpec) { s.kind = "resource" },
			func(s *entrypointSpec) { s.typeName = "aws_pinpointsmsvoicev2_protect_configuration" },
		} {
			changed := spec
			mutate(&changed)
			if isReviewedUntaggedListSpec(changed) {
				t.Fatalf("unreviewed registration accepted: %#v", changed)
			}
		}
	}
}
