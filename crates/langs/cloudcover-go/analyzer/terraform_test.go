package main

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

const terraformTestLock = `provider "registry.terraform.io/hashicorp/aws" {
  version = "6.64.0"
  constraints = "~> 6.0"
  hashes = []
}
`

func writeTerraformTestFile(t *testing.T, root, name, content string) {
	t.Helper()
	path := filepath.Join(root, name)
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestAnalyzeTerraformRootWithoutModuleManifest(t *testing.T) {
	for _, dataDir := range []string{"absent", "empty", "modules"} {
		t.Run(dataDir, func(t *testing.T) {
			root := t.TempDir()
			writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
			writeTerraformTestFile(t, root, "main.tf", `resource "aws_s3_bucket" "root" {}
 data "aws_ecr_image" "base" { repository_name = "example" }
`)
			if dataDir != "absent" {
				dir := filepath.Join(root, ".terraform")
				if dataDir == "modules" {
					dir = filepath.Join(dir, "modules")
				}
				if err := os.MkdirAll(dir, 0o700); err != nil {
					t.Fatal(err)
				}
			}
			references, version, err := analyzeTerraformDir(root)
			if err != nil {
				t.Fatal(err)
			}
			want := []terraformReference{
				{Kind: "data_source", TypeName: "aws_ecr_image", Action: "read"},
				{Kind: "resource", TypeName: "aws_s3_bucket", Action: "create"},
				{Kind: "resource", TypeName: "aws_s3_bucket", Action: "delete"},
				{Kind: "resource", TypeName: "aws_s3_bucket", Action: "read"},
				{Kind: "resource", TypeName: "aws_s3_bucket", Action: "update"},
			}
			if version != "6.64.0" || !reflect.DeepEqual(references, want) {
				t.Fatalf("got %v at %q; want %v at exact locked version 6.64.0", references, version, want)
			}
		})
	}
}

func TestAnalyzeTerraformRejectsInvalidInitialization(t *testing.T) {
	const manifestPath = ".terraform/modules/modules.json"
	const moduleCall = `module "child" { source = "./child" }`
	for _, test := range []struct {
		name, config, lock, manifest, wantError string
		unreadable                              string
	}{
		{name: "module call without manifest", config: moduleCall, lock: terraformTestLock, wantError: "Terraform module manifest is required"},
		{name: "module call omitted from manifest", config: moduleCall, lock: terraformTestLock, manifest: `{"Modules":[{"Key":"","Dir":"."}]}`, wantError: `module "child" is missing`},
		{name: "nested call omitted from manifest", config: moduleCall, lock: terraformTestLock, manifest: `{"Modules":[{"Key":"child","Dir":"child"}]}`, wantError: `module "child.nested" is missing`},
		{name: "manifest entry without directory", config: moduleCall, lock: terraformTestLock, manifest: `{"Modules":[{"Key":"child"}]}`, wantError: `entry "child" has no directory`},
		{name: "corrupt manifest without calls", lock: terraformTestLock, manifest: `{`, wantError: "Failed to parse module manifest"},
		{name: "unreadable manifest without calls", lock: terraformTestLock, unreadable: manifestPath, wantError: "Failed to read module manifest"},
		{name: "missing lock", wantError: "Terraform provider lock file is required"},
		{name: "corrupt lock", lock: `provider {`, wantError: "Failed to parse lock file"},
		{name: "unreadable lock", unreadable: ".terraform.lock.hcl", wantError: "Failed to read lock file"},
		{name: "missing AWS provider", lock: `provider "registry.terraform.io/hashicorp/random" { version = "3.7.2" }`, wantError: "AWS provider version is missing"},
		{name: "missing provider version", lock: `provider "registry.terraform.io/hashicorp/aws" {}`, wantError: "AWS provider version is missing"},
		{name: "invalid provider version expression", lock: `provider "registry.terraform.io/hashicorp/aws" { version = unknown.version }`, wantError: "AWS provider version is missing"},
		{name: "invalid root config without manifest", config: `resource {`, lock: terraformTestLock, wantError: "failed to parse Terraform module"},
		{name: "invalid child config", config: moduleCall, lock: terraformTestLock, manifest: `{"Modules":[{"Key":"child","Dir":"invalid"}]}`, wantError: "failed to parse Terraform module"},
	} {
		t.Run(test.name, func(t *testing.T) {
			root := t.TempDir()
			writeTerraformTestFile(t, root, "main.tf", test.config)
			writeTerraformTestFile(t, root, "child/main.tf", `module "nested" { source = "../nested" }`)
			writeTerraformTestFile(t, root, "invalid/main.tf", `resource {`)
			if test.lock != "" {
				writeTerraformTestFile(t, root, ".terraform.lock.hcl", test.lock)
			}
			if test.manifest != "" {
				writeTerraformTestFile(t, root, manifestPath, test.manifest)
			}
			if test.unreadable != "" {
				// A directory reliably makes ReadFile fail, including under privileged test users.
				if err := os.MkdirAll(filepath.Join(root, test.unreadable), 0o700); err != nil {
					t.Fatal(err)
				}
			}
			references, version, err := analyzeTerraformDir(root)
			if err == nil || !strings.Contains(err.Error(), test.wantError) {
				t.Fatalf("got error %v; want %q", err, test.wantError)
			}
			if references != nil || version != "" {
				t.Fatalf("returned partial analysis on error: %v at %q", references, version)
			}
		})
	}
}

func TestAnalyzeTerraformTraversesInitializedModules(t *testing.T) {
	root := t.TempDir()
	writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
	writeTerraformTestFile(t, root, "main.tf", `module "child" { source = "./child" }`)
	writeTerraformTestFile(t, root, "child/main.tf", `module "nested" { source = "../nested" }
resource "aws_s3_bucket" "child" {}`)
	writeTerraformTestFile(t, root, "nested/main.tf", `data "aws_ecr_image" "nested" { repository_name = "example" }`)
	writeTerraformTestFile(t, root, ".terraform/modules/modules.json", `{"Modules":[
{"Key":"","Dir":"."},
{"Key":"child","Dir":"child"},
{"Key":"child.nested","Dir":"nested"},
{"Key":"stale","Dir":"does-not-exist"}
]}`)
	references, version, err := analyzeTerraformDir(root)
	if err != nil {
		t.Fatal(err)
	}
	if version != "6.64.0" || len(references) != 5 {
		t.Fatalf("got %v at %q; want five lifecycle/data references at 6.64.0", references, version)
	}
}

func TestAnalyzeTerraformRejectsUnsupportedListQueries(t *testing.T) {
	for _, tc := range []struct {
		name, file, source string
		mixed              bool
	}{
		{name: "query only", file: "users.tfquery.hcl", source: `list "aws_iam_user" "users" { provider = aws }`},
		{name: "mixed query", file: "users.tfquery.hcl", source: `list "aws_iam_user" "users" { provider = aws }`, mixed: true},
		{name: "JSON query only", file: "users.tfquery.json", source: `{"list":{"aws_iam_user":{"users":{"provider":"aws"}}}}`},
		{name: "mixed JSON query", file: "users.tfquery.json", source: `{"list":{"aws_iam_user":{"users":{}}}}`, mixed: true},
		{name: "empty query is unsupported input", file: "empty.tfquery.hcl"},
		{name: "malformed query", file: "bad.tfquery.hcl", source: `list {`},
		{name: "inline only", file: "users.tf", source: `list "aws_iam_user" "users" { provider = aws }`},
		{name: "mixed inline", file: "users.tf", source: `list "aws_iam_user" "users" { provider = aws }`, mixed: true},
		{name: "inline JSON", file: "users.tf.json", source: `{"list":{"aws_iam_user":{"users":{"provider":"aws"}}}}`, mixed: true},
		{name: "invalid list labels", file: "users.tf", source: `list "aws_iam_user" {}`},
		{name: "override list", file: "users_override.tf", source: `list "aws_iam_user" "users" {}`, mixed: true},
		{name: "legacy syntax", file: "legacy.tf", source: `resource "aws_s3_bucket" "root" { bucket = "example", acl = "private" }
list "aws_iam_user" "users" { provider = aws }`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
			if tc.mixed {
				writeTerraformTestFile(t, root, "main.tf", `resource "aws_s3_bucket" "root" {}`)
			}
			writeTerraformTestFile(t, root, tc.file, tc.source)
			refs, version, err := analyzeTerraformDir(root)
			if err == nil || !strings.Contains(err.Error(), "unsupported Terraform list queries") || !strings.Contains(err.Error(), tc.file) {
				t.Fatalf("got %v, want named unsupported query", err)
			}
			if refs != nil || version != "" {
				t.Fatalf("partial analysis escaped: %v at %q", refs, version)
			}
		})
	}
}

func TestAnalyzeTerraformRejectsQueriesOnlyInReachableModules(t *testing.T) {
	for _, file := range []string{"users.tfquery.hcl", "users.tfquery.json", "users.tf", "users.tf.json"} {
		t.Run(file, func(t *testing.T) {
			root := t.TempDir()
			writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
			writeTerraformTestFile(t, root, "main.tf", `module "child" { source = "./child" }
resource "aws_s3_bucket" "root" {}`)
			writeTerraformTestFile(t, root, "child/main.tf", `module "nested" { source = "../nested" }`)
			writeTerraformTestFile(t, root, "nested/main.tf", "# empty reachable module\n")
			writeTerraformTestFile(t, root, ".terraform/modules/modules.json", `{"Modules":[{"Key":"child","Dir":"child"},{"Key":"child.nested","Dir":"nested"},{"Key":"stale","Dir":"stale"}]}`)
			source := `list "aws_iam_user" "users" { provider = aws }`
			if strings.HasSuffix(file, ".json") {
				source = `{"list":{"aws_iam_user":{"users":{}}}}`
			}
			writeTerraformTestFile(t, root, "stale/"+file, source)
			refs, _, err := analyzeTerraformDir(root)
			if err != nil || len(refs) != 4 {
				t.Fatalf("stale/unreferenced directory affected root: %v %v", refs, err)
			}
			writeTerraformTestFile(t, root, "nested/"+file, source)
			refs, version, err := analyzeTerraformDir(root)
			if err == nil || !strings.Contains(err.Error(), filepath.Join("nested", file)) {
				t.Fatalf("nested query not reported: %v", err)
			}
			if refs != nil || version != "" {
				t.Fatalf("partial nested analysis escaped: %v at %q", refs, version)
			}
		})
	}
}

func TestAnalyzeTerraformIgnoresListTextAndNonInputFiles(t *testing.T) {
	root := t.TempDir()
	writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
	writeTerraformTestFile(t, root, "main.tf", `# list "aws_iam_user" "comment" {}
locals {
 list = "ordinary attribute"
 example = <<-EOT
list "aws_iam_user" "documentation" {}
EOT
}
resource "aws_s3_bucket" "list" {}
data "aws_iam_policy_document" "list" {}
`)
	for _, file := range []string{".hidden.tfquery.hcl", "backup.tfquery.hcl~", "#backup.tfquery.hcl#", "README.md", "unreferenced/users.tfquery.hcl"} {
		writeTerraformTestFile(t, root, file, `list "aws_iam_user" "users" {}`)
	}
	if err := os.MkdirAll(filepath.Join(root, "directory.tfquery.hcl"), 0o700); err != nil {
		t.Fatal(err)
	}
	refs, version, err := analyzeTerraformDir(root)
	if err != nil || len(refs) != 5 || version != "6.64.0" {
		t.Fatalf("ordinary input changed: %v %q %v", refs, version, err)
	}
}

func TestGoAnalysisDoesNotInspectTerraformQueries(t *testing.T) {
	root := t.TempDir()
	writeTerraformTestFile(t, root, "go.mod", "module example.com/plain\n\ngo 1.24.0\n")
	writeTerraformTestFile(t, root, "main.go", "package plain\nfunc Example() {}\n")
	writeTerraformTestFile(t, root, "users.tfquery.hcl", `list "aws_iam_user" "users" {}`)
	if _, _, err := analyzeDir(root); err != nil {
		t.Fatalf("Go path affected by Terraform guard: %v", err)
	}
}

func TestAnalyzeTerraformExplicitQueryFiles(t *testing.T) {
	for _, name := range []string{"users.tfquery.hcl", "users.tfquery.json"} {
		root := t.TempDir()
		writeTerraformTestFile(t, root, name, "{}")
		refs, version, err := analyzeTerraformDir(filepath.Join(root, name))
		if err == nil || !strings.Contains(err.Error(), "unsupported Terraform list queries") {
			t.Fatalf("explicit query got %v", err)
		}
		if refs != nil || version != "" {
			t.Fatalf("partial analysis on explicit query: %v %q", refs, version)
		}
	}
}

func TestAnalyzeTerraformOrdinaryJSONAndLegacyInputs(t *testing.T) {
	for _, tc := range []struct{ name, source string }{
		{"main.tf.json", `{"resource":{"aws_s3_bucket":{"ordinary":{}}},"locals":{"list":{"example":"a value, not a block"}}}`},
		{"legacy.tf", `resource "aws_s3_bucket" "ordinary" { bucket = "example", acl = "private" }
list = { example = "an ignored legacy attribute, not a block" }`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
			writeTerraformTestFile(t, root, tc.name, tc.source)
			refs, version, err := analyzeTerraformDir(root)
			if err != nil || len(refs) != 4 || version != "6.64.0" {
				t.Fatalf("ordinary configuration changed: %v %q %v", refs, version, err)
			}
		})
	}
}

func TestAnalyzeTerraformInvalidOrdinaryJSONKeepsParseError(t *testing.T) {
	root := t.TempDir()
	writeTerraformTestFile(t, root, ".terraform.lock.hcl", terraformTestLock)
	writeTerraformTestFile(t, root, "main.tf.json", `[true]`)
	refs, version, err := analyzeTerraformDir(root)
	if err == nil || !strings.Contains(err.Error(), "failed to parse Terraform module") || strings.Contains(err.Error(), "unsupported Terraform list queries") {
		t.Fatalf("wrong ordinary JSON error: %v", err)
	}
	if refs != nil || version != "" {
		t.Fatalf("partial invalid JSON analysis: %v %q", refs, version)
	}
}
