package main

import (
	"go/types"
	"os"
	"path/filepath"
	"slices"
	"testing"
)

func TestCollectAPIMethodsIncludesExternalS3Uploader(t *testing.T) {
	for _, test := range []struct{ pkg, method string }{
		{"github.com/aws/aws-sdk-go-v2/feature/s3/manager", "Upload"},
		{"github.com/aws/aws-sdk-go/service/s3/s3manager", "Upload"},
		{"github.com/aws/aws-sdk-go/service/s3/s3manager", "UploadWithContext"},
		{"example.com/unrelated", "Upload"},
		{"github.com/aws/aws-sdk-go-v2/feature/s3/manager", "NewUploader"},
	} {
		t.Run(test.pkg+"/"+test.method, func(t *testing.T) {
			t.Parallel()
			dir := t.TempDir()
			files := map[string]string{
				"go.mod":                        "module " + providerModulePath + "\n\ngo 1.26.0\nrequire " + test.pkg + " v1.0.0\nreplace " + test.pkg + " => ./manager\n",
				"manager/go.mod":                "module " + test.pkg + "\n\ngo 1.26.0\n",
				"manager/upload.go":             "package manager\ntype Uploader struct{}\nfunc (*Uploader) " + test.method + "() {}\n",
				"internal/provider/provider.go": "package provider\n",
				"internal/service/s3/object.go": "package s3\nimport manager \"" + test.pkg + "\"\nfunc handler() { new(manager.Uploader)." + test.method + "() }\n",
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
			pkg, err := packageForPath(index, providerModulePath+"/internal/service/s3")
			if err != nil {
				t.Fatal(err)
			}
			roots, err := ssaFunctionsForObject(index, pkg.Types.Scope().Lookup("handler").(*types.Func), "handler")
			if err != nil {
				t.Fatal(err)
			}
			analysis, err := collectAPIMethods(index, roots, nil)
			if err != nil {
				t.Fatal(err)
			}
			var want []apiMethod
			if test.pkg != "example.com/unrelated" && test.method != "NewUploader" {
				want = []apiMethod{
					{Service: "s3", Name: "AbortMultipartUpload"},
					{Service: "s3", Name: "CompleteMultipartUpload"},
					{Service: "s3", Name: "CreateMultipartUpload"},
					{Service: "s3", Name: "PutObject"},
					{Service: "s3", Name: "UploadPart"},
				}
			}
			if !slices.Equal(analysis.methods, want) {
				t.Fatalf("uploader methods = %#v, want %#v", analysis.methods, want)
			}
		})
	}
}
