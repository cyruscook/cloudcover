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
